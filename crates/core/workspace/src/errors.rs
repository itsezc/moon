use moon_constants as constants;
use moon_error::MoonError;
use moon_toolchain::{ToolError, ToolchainError};
use moon_vcs::VcsError;
use moonbase::MoonbaseError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceError {
    #[error(
        "Unable to determine workspace root. Please create a <file>{}</file> configuration folder.",
        constants::CONFIG_DIRNAME
    )]
    MissingConfigDir,

    #[error(
        "Unable to locate <file>{}/{}</file> configuration file.",
        constants::CONFIG_DIRNAME,
        constants::CONFIG_WORKSPACE_FILENAME
    )]
    MissingWorkspaceConfigFile,

    #[error(
        "Failed to validate <file>{}/{}</file> configuration file.\n\n{0}",
        constants::CONFIG_DIRNAME,
        constants::CONFIG_TOOLCHAIN_FILENAME
    )]
    InvalidToolchainConfigFile(String),

    #[error(
        "Failed to validate <file>{}/{}</file> configuration file.\n\n{0}",
        constants::CONFIG_DIRNAME,
        constants::CONFIG_WORKSPACE_FILENAME
    )]
    InvalidWorkspaceConfigFile(String),

    #[error(
        "Failed to validate <file>{}/{}</file> configuration file.\n\n{0}",
        constants::CONFIG_DIRNAME,
        constants::CONFIG_GLOBAL_PROJECT_FILENAME
    )]
    InvalidGlobalProjectConfigFile(String),

    #[error(transparent)]
    Moon(#[from] MoonError),

    #[error(transparent)]
    Moonbase(#[from] MoonbaseError),

    #[error(transparent)]
    Tool(#[from] ToolError),

    #[error(transparent)]
    Toolchain(#[from] ToolchainError),

    #[error(transparent)]
    Vcs(#[from] VcsError),
}
