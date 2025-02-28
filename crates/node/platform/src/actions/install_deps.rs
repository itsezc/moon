use moon_action::{Action, ActionStatus};
use moon_config::NodePackageManager;
use moon_error::map_io_to_fs_error;
use moon_error::MoonError;
use moon_lang::has_vendor_installed_dependencies;
use moon_logger::{color, debug, warn};
use moon_node_lang::{PackageJson, NODE, NPM};
use moon_node_tool::NodeTool;
use moon_platform::Runtime;
use moon_project::Project;
use moon_runner_context::RunnerContext;
use moon_terminal::{print_checkpoint, Checkpoint};
use moon_utils::{fs, is_ci, is_offline, time};
use moon_workspace::{Workspace, WorkspaceError};
use std::sync::Arc;
use tokio::sync::RwLock;

const LOG_TARGET: &str = "moon:node-platform:install-deps";

/// Add `packageManager` to root `package.json`.
fn add_package_manager(node: &NodeTool, package_json: &mut PackageJson) -> bool {
    let manager_version = match node.config.package_manager {
        NodePackageManager::Npm => format!("npm@{}", node.config.npm.version),
        NodePackageManager::Pnpm => format!(
            "pnpm@{}",
            match &node.config.pnpm {
                Some(pnpm) => pnpm.version.clone(),
                None => {
                    return false;
                }
            }
        ),
        NodePackageManager::Yarn => format!(
            "yarn@{}",
            match &node.config.yarn {
                Some(yarn) => yarn.version.clone(),
                None => {
                    return false;
                }
            }
        ),
    };

    if package_json.set_package_manager(&manager_version) {
        debug!(
            target: LOG_TARGET,
            "Adding package manager version to root {}",
            color::file(NPM.manifest)
        );

        return true;
    }

    false
}

/// Add `engines` constraint to root `package.json`.
fn add_engines_constraint(node: &NodeTool, package_json: &mut PackageJson) -> bool {
    if node.config.add_engines_constraint && package_json.add_engine("node", &node.config.version) {
        debug!(
            target: LOG_TARGET,
            "Adding engines version constraint to root {}",
            color::file(NPM.manifest)
        );

        return true;
    }

    false
}

fn sync_workspace(workspace: &Workspace, node: &NodeTool) -> Result<(), MoonError> {
    // Sync values to root `package.json`
    PackageJson::sync(&workspace.root, |package_json| {
        add_package_manager(node, package_json);
        add_engines_constraint(node, package_json);

        Ok(())
    })?;

    // Create nvm/nodenv version file
    if let Some(version_manager) = &node.config.sync_version_manager_config {
        let rc_name = version_manager.get_config_filename();
        let rc_path = workspace.root.join(&rc_name);

        fs::write(rc_path, &node.config.version)?;

        debug!(
            target: LOG_TARGET,
            "Syncing Node.js version to root {}",
            color::file(&rc_name)
        );
    }

    Ok(())
}

pub async fn install_deps(
    _action: &mut Action,
    context: Arc<RwLock<RunnerContext>>,
    workspace: Arc<RwLock<Workspace>>,
    runtime: &Runtime,
    project: Option<&Project>,
) -> Result<ActionStatus, WorkspaceError> {
    let workspace = workspace.read().await;
    let context = context.read().await;

    // When cache is write only, avoid install as user is typically force updating cache
    if workspace.cache.get_mode().is_write_only() {
        debug!(target: LOG_TARGET, "Force updating cache, skipping install",);

        return Ok(ActionStatus::Skipped);
    }

    // When running against affected files, avoid install as it interrupts the workflow
    if context.affected_only {
        debug!(
            target: LOG_TARGET,
            "Running against affected files, skipping install",
        );

        return Ok(ActionStatus::Skipped);
    }

    let node = workspace
        .toolchain
        .node
        .get_for_runtime::<NodeTool>(runtime)?;
    let pm = node.get_package_manager();
    let lock_filename = pm.get_lock_filename();
    let manifest_filename = pm.get_manifest_filename();

    // Determine the working directory and whether lockfiles and manifests have been modified
    let working_dir;
    let has_modified_files;

    if let Some(project) = project {
        working_dir = project.root.clone();
        has_modified_files = context
            .touched_files
            .contains(&working_dir.join(&lock_filename))
            || context
                .touched_files
                .contains(&working_dir.join(&manifest_filename));
    } else {
        working_dir = workspace.root.clone();
        has_modified_files = context
            .touched_files
            .iter()
            .any(|f| f.ends_with(&lock_filename) || f.ends_with(&manifest_filename));

        // When installing deps in the workspace root, also sync applicable settings
        sync_workspace(&workspace, node)?;
    }

    // Install dependencies in the current project or workspace
    let lock_filepath = working_dir.join(&lock_filename);
    let mut last_modified = 0;
    let mut cache = workspace
        .cache
        .cache_deps_state(runtime, project.map(|p| p.id.as_ref()))?;

    if lock_filepath.exists() {
        last_modified = time::to_millis(
            fs::metadata(&lock_filepath)?
                .modified()
                .map_err(|e| map_io_to_fs_error(e, lock_filepath.clone()))?,
        );
    }

    // Install deps if the lockfile has been modified since the last time they were installed!
    if has_modified_files || last_modified == 0 || last_modified > cache.last_install_time {
        debug!(
            target: LOG_TARGET,
            "Installing {} dependencies in {}",
            runtime.label(),
            color::path(&working_dir)
        );

        // When in CI, we can avoid installing dependencies because
        // we can assume they've already been installed before moon runs!
        if is_ci() && has_vendor_installed_dependencies(&working_dir, &NODE) {
            warn!(
                target: LOG_TARGET,
                "In a CI environment and dependencies already exist, skipping install"
            );

            return Ok(ActionStatus::Skipped);
        }

        if is_offline() {
            warn!(
                target: LOG_TARGET,
                "No internet connection, assuming offline and skipping install"
            );

            return Ok(ActionStatus::Skipped);
        }

        let should_log_command = workspace.config.runner.log_running_command;

        // install
        {
            let install_command = match node.config.package_manager {
                NodePackageManager::Npm => "npm install",
                NodePackageManager::Pnpm => "pnpm install",
                NodePackageManager::Yarn => "yarn install",
            };

            print_checkpoint(install_command, Checkpoint::Setup);

            pm.install_dependencies(node, &working_dir, should_log_command)
                .await?;
        }

        // dedupe
        if !is_ci() && node.config.dedupe_on_lockfile_change {
            let dedupe_command = match node.config.package_manager {
                NodePackageManager::Npm => "npm dedupe",
                NodePackageManager::Pnpm => "pnpm dedupe",
                NodePackageManager::Yarn => "yarn dedupe",
            };

            debug!(target: LOG_TARGET, "Deduping dependencies");

            print_checkpoint(dedupe_command, Checkpoint::Setup);

            pm.dedupe_dependencies(node, &working_dir, should_log_command)
                .await?;
        }

        // Update the cache with the timestamp
        cache.last_install_time = time::now_millis();
        cache.save()?;

        return Ok(ActionStatus::Passed);
    }

    debug!(
        target: LOG_TARGET,
        "Lockfile has not changed since last install, skipping Node.js dependencies",
    );

    Ok(ActionStatus::Skipped)
}
