use crate::RunnerError;
use console::Term;
use moon_action::{Action, ActionStatus, Attempt};
use moon_cache::RunTargetState;
use moon_config::{PlatformType, TaskOutputStyle};
use moon_emitter::{Emitter, Event, EventFlow};
use moon_error::MoonError;
use moon_hasher::{convert_paths_to_strings, to_hash, Hasher, TargetHasher};
use moon_logger::{color, debug, warn};
use moon_node_platform::actions as node_actions;
use moon_project::Project;
use moon_project_graph::ProjectGraph;
use moon_runner_context::RunnerContext;
use moon_system_platform::actions as system_actions;
use moon_task::{
    Target, TargetError, TargetProjectScope, Task, TaskError, TaskOptionAffectedFiles,
};
use moon_terminal::label_checkpoint;
use moon_terminal::Checkpoint;
use moon_utils::{
    is_ci, is_test_env, path,
    process::{self, format_running_command, output_to_string, Command, Output},
    time,
};
use moon_workspace::Workspace;
use rustc_hash::FxHashMap;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;

const LOG_TARGET: &str = "moon:action:run-target";

pub enum HydrateFrom {
    LocalCache,
    PreviousOutput,
    RemoteCache,
}

pub struct TargetRunner<'a> {
    pub cache: RunTargetState,

    emitter: &'a Emitter,

    project: &'a Project,

    stderr: Term,

    stdout: Term,

    task: &'a Task,

    workspace: &'a Workspace,
}

impl<'a> TargetRunner<'a> {
    pub fn new(
        emitter: &'a Emitter,
        workspace: &'a Workspace,
        project: &'a Project,
        task: &'a Task,
    ) -> Result<TargetRunner<'a>, MoonError> {
        Ok(TargetRunner {
            cache: workspace.cache.cache_run_target_state(&task.target)?,
            emitter,
            project,
            stderr: Term::buffered_stderr(),
            stdout: Term::buffered_stdout(),
            task,
            workspace,
        })
    }

    /// Cache outputs to the `.moon/cache/outputs` folder and to the cloud,
    /// so that subsequent builds are faster, and any local outputs
    /// can be hydrated easily.
    pub async fn archive_outputs(&self) -> Result<(), RunnerError> {
        let hash = &self.cache.hash;

        if hash.is_empty() || !self.is_archivable()? {
            return Ok(());
        }

        // Check that outputs actually exist
        if !self.task.outputs.is_empty() {
            for (i, output) in self.task.output_paths.iter().enumerate() {
                if !output.exists() {
                    return Err(RunnerError::Task(TaskError::MissingOutput(
                        self.task.target.id.clone(),
                        self.task.outputs.get(i).unwrap().to_owned(),
                    )));
                }
            }
        }

        // If so, then cache the archive
        if let EventFlow::Return(archive_path) = self
            .emitter
            .emit(Event::TargetOutputArchiving {
                cache: &self.cache,
                hash,
                project: self.project,
                target: &self.task.target,
                task: self.task,
            })
            .await?
        {
            self.emitter
                .emit(Event::TargetOutputArchived {
                    archive_path: archive_path.into(),
                    hash,
                    project: self.project,
                    target: &self.task.target,
                    task: self.task,
                })
                .await?;
        }

        Ok(())
    }

    /// If we are cached (hash match), hydrate the project with the
    /// cached task outputs found in the hashed archive.
    pub async fn hydrate_outputs(&self) -> Result<(), RunnerError> {
        let hash = &self.cache.hash;

        if hash.is_empty() {
            return Ok(());
        }

        // Hydrate outputs from the cache
        if let EventFlow::Return(archive_path) = self
            .emitter
            .emit(Event::TargetOutputHydrating {
                cache: &self.cache,
                hash,
                project: self.project,
                target: &self.task.target,
                task: self.task,
            })
            .await?
        {
            self.emitter
                .emit(Event::TargetOutputHydrated {
                    archive_path: archive_path.into(),
                    hash,
                    project: self.project,
                    target: &self.task.target,
                    task: self.task,
                })
                .await?;
        }

        // Update the run state with the new hash
        self.cache.save()?;

        Ok(())
    }

    /// Create a hasher that is shared amongst all platforms.
    /// Primarily includes task information.
    pub async fn create_common_hasher(
        &self,
        context: &RunnerContext,
    ) -> Result<TargetHasher, RunnerError> {
        let vcs = &self.workspace.vcs;
        let task = &self.task;
        let project = &self.project;
        let workspace = &self.workspace;
        let globset = task.create_globset()?;
        let mut hasher = TargetHasher::new();

        hasher.hash_project_deps(self.project.get_dependency_ids());
        hasher.hash_task(task);
        hasher.hash_task_deps(task, &context.target_hashes);

        if context.should_inherit_args(&task.target) {
            hasher.hash_args(&context.passthrough_args);
        }

        // For input files, hash them with the vcs layer first
        if !task.input_paths.is_empty() {
            let files = convert_paths_to_strings(&task.input_paths, &workspace.root)?;

            if !files.is_empty() {
                hasher.hash_inputs(vcs.get_file_hashes(&files, true).await?);
            }
        }

        // For input globs, it's much more performant to:
        //  `git ls-tree` -> match against glob patterns
        // Then it is to:
        //  glob + walk the file system -> `git hash-object`
        if !task.input_globs.is_empty() {
            let mut hashed_file_tree = vcs.get_file_tree_hashes(&project.source).await?;

            // Input globs are absolute paths, so we must do the same
            hashed_file_tree
                .retain(|k, _| globset.matches(workspace.root.join(k)).unwrap_or(false));

            hasher.hash_inputs(hashed_file_tree);
        }

        // Include local file changes so that development builds work.
        // Also run this LAST as it should take highest precedence!
        let local_files = vcs.get_touched_files().await?;

        if !local_files.all.is_empty() {
            // Only hash files that are within the task's inputs
            let files = local_files
                .all
                .into_iter()
                .filter(|f| {
                    // Deleted files will crash `git hash-object`
                    !local_files.deleted.contains(f)
                        && globset.matches(workspace.root.join(f)).unwrap_or(false)
                })
                .collect::<Vec<String>>();

            if !files.is_empty() {
                hasher.hash_inputs(vcs.get_file_hashes(&files, false).await?);
            }
        }

        Ok(hasher)
    }

    pub async fn create_command(&self, context: &RunnerContext) -> Result<Command, RunnerError> {
        let workspace = &self.workspace;
        let project = &self.project;
        let task = &self.task;
        let working_dir = if task.options.run_from_workspace_root {
            &workspace.root
        } else {
            &project.root
        };

        debug!(
            target: LOG_TARGET,
            "Creating {} command (in working directory {})",
            color::target(&task.target),
            color::path(working_dir)
        );

        let mut command = match task.platform {
            PlatformType::Node => {
                node_actions::create_target_command(context, workspace, project, task, working_dir)?
            }
            _ => system_actions::create_target_command(task, working_dir),
        };

        command
            .cwd(working_dir)
            .envs(self.create_env_vars().await?)
            // We need to handle non-zero's manually
            .no_error_on_failure();

        // Passthrough args
        if context.should_inherit_args(&self.task.target) {
            command.args(&context.passthrough_args);
        }

        // Terminal colors
        if self.workspace.config.runner.inherit_colors_for_piped_tasks {
            command.inherit_colors();
        }

        // Affected files (must be last args)
        if let Some(check_affected) = &self.task.options.affected_files {
            let mut affected_files = if context.affected_only {
                self.task
                    .get_affected_files(&context.touched_files, &self.project.root)?
            } else {
                Vec::with_capacity(0)
            };

            affected_files.sort();

            if matches!(
                check_affected,
                TaskOptionAffectedFiles::Env | TaskOptionAffectedFiles::Both
            ) {
                command.env(
                    "MOON_AFFECTED_FILES",
                    if affected_files.is_empty() {
                        ".".into()
                    } else {
                        affected_files
                            .iter()
                            .map(|f| f.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(",")
                    },
                );
            }

            if matches!(
                check_affected,
                TaskOptionAffectedFiles::Args | TaskOptionAffectedFiles::Both
            ) {
                if affected_files.is_empty() {
                    command.arg_if_missing(".");
                } else {
                    command.args(affected_files);
                }
            }
        }

        Ok(command)
    }

    pub async fn create_env_vars(&self) -> Result<FxHashMap<String, String>, MoonError> {
        let mut env_vars = FxHashMap::default();

        env_vars.insert(
            "MOON_CACHE_DIR".to_owned(),
            path::to_string(&self.workspace.cache.dir)?,
        );
        env_vars.insert("MOON_PROJECT_ID".to_owned(), self.project.id.clone());
        env_vars.insert(
            "MOON_PROJECT_ROOT".to_owned(),
            path::to_string(&self.project.root)?,
        );
        env_vars.insert(
            "MOON_PROJECT_SOURCE".to_owned(),
            self.project.source.clone(),
        );
        env_vars.insert("MOON_TARGET".to_owned(), self.task.target.id.clone());
        env_vars.insert(
            "MOON_TOOLCHAIN_DIR".to_owned(),
            path::to_string(&self.workspace.toolchain.dir)?,
        );
        env_vars.insert(
            "MOON_WORKSPACE_ROOT".to_owned(),
            path::to_string(&self.workspace.root)?,
        );
        env_vars.insert(
            "MOON_WORKING_DIR".to_owned(),
            path::to_string(&self.workspace.working_dir)?,
        );

        // Store runtime data on the file system so that downstream commands can utilize it
        let runfile = self
            .workspace
            .cache
            .create_runfile(&self.project.id, self.project)?;

        env_vars.insert(
            "MOON_PROJECT_RUNFILE".to_owned(),
            path::to_string(runfile.path)?,
        );

        Ok(env_vars)
    }

    pub fn get_short_hash(&self) -> &str {
        if self.cache.hash.is_empty() {
            "" // Empty when cache is disabled
        } else {
            &self.cache.hash[0..8]
        }
    }

    pub fn flush_output(&self) -> Result<(), MoonError> {
        self.stdout.flush()?;
        self.stderr.flush()?;

        Ok(())
    }

    /// Determine if the current task can be archived.
    pub fn is_archivable(&self) -> Result<bool, TargetError> {
        let task = self.task;

        if task.is_build_type() {
            return Ok(true);
        }

        for target in &self.workspace.config.runner.archivable_targets {
            let target = Target::parse(target)?;

            match &target.project {
                TargetProjectScope::All => {
                    if task.target.task_id == target.task_id {
                        return Ok(true);
                    }
                }
                TargetProjectScope::Id(project_id) => {
                    if let Some(owner_id) = &task.target.project_id {
                        if owner_id == project_id && task.target.task_id == target.task_id {
                            return Ok(true);
                        }
                    }
                }
                TargetProjectScope::Deps => return Err(TargetError::NoProjectDepsInRunContext),
                TargetProjectScope::OwnSelf => return Err(TargetError::NoProjectSelfInRunContext),
            };
        }

        Ok(false)
    }

    /// Hash the target based on all current parameters and return early
    /// if this target hash has already been cached. Based on the state
    /// of the target and project, determine the hydration strategy as well.
    pub async fn is_cached(
        &mut self,
        context: &mut RunnerContext,
        common_hasher: impl Hasher + Serialize,
        platform_hasher: impl Hasher + Serialize,
    ) -> Result<Option<HydrateFrom>, MoonError> {
        let hash = to_hash(&common_hasher, &platform_hasher);

        debug!(
            target: LOG_TARGET,
            "Generated hash {} for target {}",
            color::symbol(&hash),
            color::id(&self.task.target)
        );

        context
            .target_hashes
            .insert(self.task.target.id.clone(), hash.clone());

        // Hash is the same as the previous build, so simply abort!
        // However, ensure the outputs also exist, otherwise we should hydrate.
        if self.cache.hash == hash && self.has_outputs() {
            debug!(
                target: LOG_TARGET,
                "Cache hit for hash {}, reusing previous build",
                color::symbol(&hash),
            );

            return Ok(Some(HydrateFrom::PreviousOutput));
        }

        self.cache.hash = hash.clone();

        // Refresh the hash manifest
        self.workspace
            .cache
            .create_hash_manifest(&hash, &(common_hasher, platform_hasher))?;

        // Check if that hash exists in the cache
        if let EventFlow::Return(value) = self
            .emitter
            .emit(Event::TargetOutputCacheCheck {
                hash: &hash,
                target: &self.task.target,
            })
            .await?
        {
            match value.as_ref() {
                "local-cache" => {
                    debug!(
                        target: LOG_TARGET,
                        "Cache hit for hash {}, hydrating from local cache",
                        color::symbol(&hash),
                    );

                    return Ok(Some(HydrateFrom::LocalCache));
                }
                "remote-cache" => {
                    debug!(
                        target: LOG_TARGET,
                        "Cache hit for hash {}, hydrating from remote cache",
                        color::symbol(&hash),
                    );

                    return Ok(Some(HydrateFrom::RemoteCache));
                }
                _ => {}
            }
        }

        debug!(
            target: LOG_TARGET,
            "Cache miss for hash {}, continuing run",
            color::symbol(&hash),
        );

        Ok(None)
    }

    /// Return true if this target is a no-op.
    pub fn is_no_op(&self) -> bool {
        self.task.is_no_op()
    }

    /// Verify that all task outputs exist for the current target.
    pub fn has_outputs(&self) -> bool {
        self.task.output_paths.iter().all(|p| p.exists())
    }

    /// Run the command as a child process and capture its output. If the process fails
    /// and `retry_count` is greater than 0, attempt the process again in case it passes.
    pub async fn run_command(
        &mut self,
        context: &RunnerContext,
        command: &mut Command,
    ) -> Result<Vec<Attempt>, RunnerError> {
        let attempt_total = self.task.options.retry_count + 1;
        let mut attempt_index = 1;
        let mut attempts = vec![];
        let primary_longest_width = context.primary_targets.iter().map(|t| t.id.len()).max();
        let is_primary = context.primary_targets.contains(&self.task.target);
        let is_real_ci = is_ci() && !is_test_env();
        let output;

        // When the primary target, always stream the output for a better developer experience.
        // However, transitive targets can opt into streaming as well.
        let should_stream_output = if let Some(output_style) = &self.task.options.output_style {
            matches!(output_style, TaskOutputStyle::Stream)
        } else {
            is_primary || is_real_ci
        };

        // Transitive targets may run concurrently, so differentiate them with a prefix.
        let stream_prefix = if is_real_ci || !is_primary || context.primary_targets.len() > 1 {
            Some(&self.task.target.id)
        } else {
            None
        };

        loop {
            let mut attempt = Attempt::new(attempt_index);

            self.print_target_label(
                Checkpoint::RunStart,
                // NOTE: Old streaming output format. Revisit or remove?
                // Mark primary streamed output as passed, since it may stay open forever,
                // or it may use ANSI escape codes to alter the terminal!
                // if is_primary && should_stream_output {
                //     Checkpoint::Pass
                // } else {
                //     Checkpoint::Start
                // },
                &attempt,
                attempt_total,
            )?;

            self.print_target_command(context)?;

            self.flush_output()?;

            let possible_output = if should_stream_output {
                if let Some(prefix) = stream_prefix {
                    command.set_prefix(prefix, primary_longest_width);
                }

                command.exec_stream_and_capture_output().await
            } else {
                command.exec_capture_output().await
            };

            match possible_output {
                // zero and non-zero exit codes
                Ok(out) => {
                    attempt.done(if out.status.success() {
                        ActionStatus::Passed
                    } else {
                        ActionStatus::Failed
                    });

                    if should_stream_output {
                        self.handle_streamed_output(&attempt, attempt_total, &out)?;
                    } else {
                        self.handle_captured_output(&attempt, attempt_total, &out)?;
                    }

                    attempts.push(attempt);

                    if out.status.success() {
                        output = out;
                        break;
                    } else if attempt_index >= attempt_total {
                        return Err(RunnerError::Moon(command.output_to_error(&out, false)));
                    } else {
                        attempt_index += 1;

                        warn!(
                            target: LOG_TARGET,
                            "Target {} failed, running again with attempt {}",
                            color::target(&self.task.target),
                            attempt_index
                        );
                    }
                }
                // process itself failed
                Err(error) => {
                    attempt.done(ActionStatus::Failed);
                    attempts.push(attempt);

                    return Err(RunnerError::Moon(error));
                }
            }
        }

        // Write the cache with the result and output
        self.cache.exit_code = output.status.code().unwrap_or(0);
        self.cache.last_run_time = time::now_millis();
        self.cache.save()?;
        self.cache.save_output_logs(
            output_to_string(&output.stdout),
            output_to_string(&output.stderr),
        )?;

        Ok(attempts)
    }

    pub fn print_cache_item(&self) -> Result<(), MoonError> {
        let item = &self.cache;
        let (stdout, stderr) = item.load_output_logs()?;

        self.print_output_with_style(&stdout, &stderr, item.exit_code != 0)?;

        Ok(())
    }

    pub fn print_checkpoint<T: AsRef<str>>(
        &self,
        checkpoint: Checkpoint,
        comments: &[T],
    ) -> Result<(), MoonError> {
        let label = label_checkpoint(&self.task.target, checkpoint);

        if comments.is_empty() {
            self.stdout.write_line(&label)?;
        } else {
            self.stdout.write_line(&format!(
                "{} {}",
                label,
                color::muted(format!(
                    "({})",
                    comments
                        .iter()
                        .map(|c| c.as_ref())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            ))?;
        }

        Ok(())
    }

    pub fn print_output_with_style(
        &self,
        stdout: &str,
        stderr: &str,
        failed: bool,
    ) -> Result<(), MoonError> {
        let print_stdout = || -> Result<(), MoonError> {
            if !stdout.is_empty() {
                self.stdout.write_line(stdout)?;
            }

            Ok(())
        };

        let print_stderr = || -> Result<(), MoonError> {
            if !stderr.is_empty() {
                self.stderr.write_line(stderr)?;
            }

            Ok(())
        };

        match self.task.options.output_style {
            // Only show output on failure
            Some(TaskOutputStyle::BufferOnlyFailure) => {
                if failed {
                    print_stdout()?;
                    print_stderr()?;
                }
            }
            // Only show the hash
            Some(TaskOutputStyle::Hash) => {
                let hash = &self.cache.hash;

                if !hash.is_empty() {
                    // Print to stderr so it can be captured
                    self.stderr.write_line(hash)?;
                }
            }
            // Show nothing
            Some(TaskOutputStyle::None) => {}
            // Show output on both success and failure
            _ => {
                print_stdout()?;
                print_stderr()?;
            }
        };

        Ok(())
    }

    pub fn print_target_command(&self, context: &RunnerContext) -> Result<(), MoonError> {
        if !self.workspace.config.runner.log_running_command {
            return Ok(());
        }

        let task = &self.task;
        let mut args = vec![];
        args.extend(&task.args);

        if context.should_inherit_args(&task.target) {
            args.extend(&context.passthrough_args);
        }

        let command_line = if args.is_empty() {
            task.command.clone()
        } else {
            format!("{} {}", task.command, process::join_args(args))
        };

        let message = format_running_command(
            &command_line,
            Some(if task.options.run_from_workspace_root {
                &self.workspace.root
            } else {
                &self.project.root
            }),
            Some(&self.workspace.root),
        );

        self.stdout.write_line(&message)?;

        Ok(())
    }

    pub fn print_target_label(
        &self,
        checkpoint: Checkpoint,
        attempt: &Attempt,
        attempt_total: u8,
    ) -> Result<(), MoonError> {
        let mut comments = vec![];

        if attempt.index > 1 {
            comments.push(format!("{}/{}", attempt.index, attempt_total));
        }

        if let Some(duration) = attempt.duration {
            comments.push(time::elapsed(duration));
        }

        if self.should_print_short_hash() && attempt.finished_at.is_some() {
            comments.push(self.get_short_hash().to_owned());
        }

        self.print_checkpoint(checkpoint, &comments)?;

        Ok(())
    }

    // Print label *after* output has been captured, so parallel tasks
    // aren't intertwined and the labels align with the output.
    fn handle_captured_output(
        &self,
        attempt: &Attempt,
        attempt_total: u8,
        output: &Output,
    ) -> Result<(), MoonError> {
        self.print_target_label(
            if output.status.success() {
                Checkpoint::RunPassed
            } else {
                Checkpoint::RunFailed
            },
            attempt,
            attempt_total,
        )?;

        let stdout = output_to_string(&output.stdout);
        let stderr = output_to_string(&output.stderr);

        self.print_output_with_style(&stdout, &stderr, !output.status.success())?;

        self.flush_output()?;

        Ok(())
    }

    // Only print the label when the process has failed,
    // as the actual output has already been streamed to the console.
    fn handle_streamed_output(
        &self,
        attempt: &Attempt,
        attempt_total: u8,
        output: &Output,
    ) -> Result<(), MoonError> {
        self.print_target_label(
            if output.status.success() {
                Checkpoint::RunPassed
            } else {
                Checkpoint::RunFailed
            },
            attempt,
            attempt_total,
        )?;

        // NOTE: Old streaming output format. Revisit or remove?
        // // Transitive target finished streaming, so display the success checkpoint
        // if let Some(TaskOutputStyle::Stream) = self.task.options.output_style {
        //     self.print_target_label(
        //         if output.status.success() {
        //             Checkpoint::Pass
        //         } else {
        //             Checkpoint::Fail
        //         },
        //         attempt,
        //         attempt_total,
        //     )?;

        //     // Otherwise the primary target failed for some reason
        // } else if !output.status.success() {
        //     self.print_target_label(Checkpoint::Fail, attempt, attempt_total)?;
        // }

        self.flush_output()?;

        Ok(())
    }

    fn should_print_short_hash(&self) -> bool {
        // Do not include the hash while testing, as the hash
        // constantly changes and breaks our local snapshots
        !is_test_env() && self.task.options.cache && !self.cache.hash.is_empty()
    }
}

pub async fn run_target(
    action: &mut Action,
    context: Arc<RwLock<RunnerContext>>,
    workspace: Arc<RwLock<Workspace>>,
    project_graph: Arc<RwLock<ProjectGraph>>,
    emitter: Arc<RwLock<Emitter>>,
    target: &Target,
) -> Result<ActionStatus, RunnerError> {
    let (project_id, task_id) = target.ids()?;
    let workspace = workspace.read().await;
    let project_graph = project_graph.read().await;
    let emitter = emitter.read().await;
    let project = project_graph.get(&project_id)?;
    let task = project.get_task(&task_id)?;
    let mut runner = TargetRunner::new(&emitter, &workspace, project, task)?;

    debug!(
        target: LOG_TARGET,
        "Running target {}",
        color::id(&task.target)
    );

    // Abort early if a no operation
    if runner.is_no_op() {
        debug!(
            target: LOG_TARGET,
            "Target {} is a no operation, skipping",
            color::id(&task.target),
        );

        runner.print_checkpoint(Checkpoint::RunPassed, &["no op"])?;
        runner.flush_output()?;

        return Ok(ActionStatus::Passed);
    }

    let mut should_cache = task.options.cache;

    // If the VCS root does not exist (like in a Docker image),
    // we should avoid failing and instead log a warning.
    if !workspace.vcs.is_enabled() {
        should_cache = false;

        warn!(
            target: LOG_TARGET,
            "VCS root not found, caching will be disabled!"
        );
    }

    // Abort early if this build has already been cached/hashed
    if should_cache {
        let mut context = context.write().await;
        let common_hasher = runner.create_common_hasher(&context).await?;

        let is_cached = match task.platform {
            PlatformType::Node => {
                runner
                    .is_cached(
                        &mut context,
                        common_hasher,
                        node_actions::create_target_hasher(&workspace, project).await?,
                    )
                    .await?
            }
            _ => {
                runner
                    .is_cached(
                        &mut context,
                        common_hasher,
                        system_actions::create_target_hasher(&workspace, project)?,
                    )
                    .await?
            }
        };

        if let Some(cache_location) = is_cached {
            // Only hydrate when the hash is different from the previous build,
            // as we can assume the outputs from the previous build still exist?
            if matches!(cache_location, HydrateFrom::LocalCache)
                || matches!(cache_location, HydrateFrom::RemoteCache)
            {
                runner.hydrate_outputs().await?;
            }

            let mut comments = vec![match cache_location {
                HydrateFrom::LocalCache => "cached",
                HydrateFrom::RemoteCache => "cached from remote",
                HydrateFrom::PreviousOutput => "cached from previous run",
            }];

            if runner.should_print_short_hash() {
                comments.push(runner.get_short_hash());
            }

            runner.print_checkpoint(Checkpoint::RunPassed, &comments)?;
            runner.print_cache_item()?;
            runner.flush_output()?;

            return Ok(if matches!(cache_location, HydrateFrom::RemoteCache) {
                ActionStatus::CachedFromRemote
            } else {
                ActionStatus::Cached
            });
        }
    }

    // Create the command to run based on the task
    let context = context.read().await;
    let mut command = runner.create_command(&context).await?;

    // Execute the command and return the number of attempts
    let attempts = runner.run_command(&context, &mut command).await?;
    let status = if action.set_attempts(attempts) {
        ActionStatus::Passed
    } else {
        ActionStatus::Failed
    };

    // If successful, cache the task outputs
    if should_cache {
        runner.archive_outputs().await?;
    }

    Ok(status)
}
