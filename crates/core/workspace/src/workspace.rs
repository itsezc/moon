use crate::errors::WorkspaceError;
use moon_cache::CacheEngine;
use moon_config::{
    format_error_line, format_figment_errors, ConfigError, GlobalProjectConfig, ToolchainConfig,
    WorkspaceConfig,
};
use moon_constants as constants;
use moon_logger::{color, debug, trace};
use moon_platform::{BoxedPlatform, PlatformManager};
use moon_toolchain::Toolchain;
use moon_utils::fs;
use moon_vcs::{Vcs, VcsLoader};
use moonbase::Moonbase;
use std::env;
use std::path::{Path, PathBuf};

const LOG_TARGET: &str = "moon:workspace";

/// Recursively attempt to find the workspace root by locating the ".moon"
/// configuration folder, starting from the current working directory.
fn find_workspace_root<P: AsRef<Path>>(current_dir: P) -> Option<PathBuf> {
    if let Ok(root) = env::var("MOON_WORKSPACE_ROOT") {
        let root: PathBuf = root.parse().expect("Failed to parse MOON_WORKSPACE_ROOT.");

        return Some(root);
    }

    let current_dir = current_dir.as_ref();

    trace!(
        target: "moon:workspace",
        "Attempting to find workspace root at {}",
        color::path(current_dir),
    );

    fs::find_upwards(constants::CONFIG_DIRNAME, current_dir)
        .map(|dir| dir.parent().unwrap().to_path_buf())
}

// .moon/project.yml
fn load_global_project_config(root_dir: &Path) -> Result<GlobalProjectConfig, WorkspaceError> {
    let config_path = root_dir
        .join(constants::CONFIG_DIRNAME)
        .join(constants::CONFIG_GLOBAL_PROJECT_FILENAME);

    trace!(
        target: LOG_TARGET,
        "Attempting to find {} in {}",
        color::file(format!(
            "{}/{}",
            constants::CONFIG_DIRNAME,
            constants::CONFIG_GLOBAL_PROJECT_FILENAME,
        )),
        color::path(root_dir)
    );

    if !config_path.exists() {
        return Ok(GlobalProjectConfig::default());
    }

    match GlobalProjectConfig::load(config_path) {
        Ok(cfg) => Ok(cfg),
        Err(errors) => Err(WorkspaceError::InvalidGlobalProjectConfigFile(
            if let ConfigError::FailedValidation(valids) = errors {
                format_figment_errors(valids)
            } else {
                format_error_line(errors.to_string())
            },
        )),
    }
}

// .moon/toolchain.yml
fn load_toolchain_config(root_dir: &Path) -> Result<ToolchainConfig, WorkspaceError> {
    let config_path = root_dir
        .join(constants::CONFIG_DIRNAME)
        .join(constants::CONFIG_TOOLCHAIN_FILENAME);

    trace!(
        target: LOG_TARGET,
        "Loading {} from {}",
        color::file(format!(
            "{}/{}",
            constants::CONFIG_DIRNAME,
            constants::CONFIG_TOOLCHAIN_FILENAME,
        )),
        color::path(root_dir)
    );

    if !config_path.exists() {
        return Ok(ToolchainConfig::default());
    }

    match ToolchainConfig::load(config_path) {
        Ok(cfg) => Ok(cfg),
        Err(errors) => Err(WorkspaceError::InvalidToolchainConfigFile(
            if let ConfigError::FailedValidation(valids) = errors {
                format_figment_errors(valids)
            } else {
                format_error_line(errors.to_string())
            },
        )),
    }
}

// .moon/workspace.yml
fn load_workspace_config(root_dir: &Path) -> Result<WorkspaceConfig, WorkspaceError> {
    let config_path = root_dir
        .join(constants::CONFIG_DIRNAME)
        .join(constants::CONFIG_WORKSPACE_FILENAME);

    trace!(
        target: LOG_TARGET,
        "Loading {} from {}",
        color::file(format!(
            "{}/{}",
            constants::CONFIG_DIRNAME,
            constants::CONFIG_WORKSPACE_FILENAME,
        )),
        color::path(root_dir)
    );

    if !config_path.exists() {
        return Err(WorkspaceError::MissingWorkspaceConfigFile);
    }

    match WorkspaceConfig::load(config_path) {
        Ok(cfg) => Ok(cfg),
        Err(errors) => Err(WorkspaceError::InvalidWorkspaceConfigFile(
            if let ConfigError::FailedValidation(valids) = errors {
                format_figment_errors(valids)
            } else {
                format_error_line(errors.to_string())
            },
        )),
    }
}

pub struct Workspace {
    /// Engine for reading and writing cache/outputs.
    pub cache: CacheEngine,

    /// Workspace configuration loaded from ".moon/workspace.yml".
    pub config: WorkspaceConfig,

    /// Registered platforms derived from toolchain configuration.
    pub platforms: PlatformManager,

    /// Global project configuration loaded from ".moon/project.yml".
    pub projects_config: GlobalProjectConfig,

    /// The root of the workspace that contains the ".moon" config folder.
    pub root: PathBuf,

    /// When logged in, the auth token and IDs for making API requests.
    pub session: Option<Moonbase>,

    /// The toolchain instance that houses all runtime tools/languages.
    pub toolchain: Toolchain,

    /// Configured version control system.
    pub vcs: Box<dyn Vcs + Send + Sync>,

    /// The current working directory.
    pub working_dir: PathBuf,
}

impl Workspace {
    /// Create a new workspace instance starting from the current working directory.
    /// Will locate the workspace root and load available configuration files.
    pub fn load() -> Result<Workspace, WorkspaceError> {
        Workspace::load_from(env::current_dir().unwrap())
    }

    pub fn load_from<P: AsRef<Path>>(working_dir: P) -> Result<Workspace, WorkspaceError> {
        let working_dir = working_dir.as_ref();
        let Some(root_dir) = find_workspace_root(working_dir) else {
            return Err(WorkspaceError::MissingConfigDir);
        };

        debug!(
            target: LOG_TARGET,
            "Creating workspace at {} (from working directory {})",
            color::path(&root_dir),
            color::path(working_dir)
        );

        // Load configs
        let config = load_workspace_config(&root_dir)?;
        let toolchain_config = load_toolchain_config(&root_dir)?;
        let projects_config = load_global_project_config(&root_dir)?;

        // Setup components
        let cache = CacheEngine::load(&root_dir)?;
        let toolchain = Toolchain::load(&toolchain_config)?;
        let vcs = VcsLoader::load(&root_dir, &config)?;

        Ok(Workspace {
            cache,
            config,
            platforms: PlatformManager::default(),
            projects_config,
            root: root_dir,
            session: None,
            toolchain,
            vcs,
            working_dir: working_dir.to_owned(),
        })
    }

    pub fn register_platform(&mut self, platform: BoxedPlatform) {
        self.platforms.register(platform.get_type(), platform);
    }

    pub async fn signin_to_moonbase(&mut self) -> Result<(), WorkspaceError> {
        let Ok(secret_key) = env::var("MOONBASE_SECRET_KEY") else {
            return Ok(());
        };

        let Ok(api_key) = env::var("MOONBASE_API_KEY") else {
            return Ok(());
        };

        let repo_slug = if self.vcs.is_enabled() {
            self.vcs.get_repository_slug().await?
        } else {
            return Ok(());
        };

        self.session = Moonbase::signin(secret_key, api_key, repo_slug).await;

        Ok(())
    }
}
