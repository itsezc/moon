use crate::platform::NodeArch;
use crate::NodeLanguage;
use log::debug;
use proto_core::{
    async_trait, download_from_url, Describable, Downloadable, ProtoError, Resolvable,
};
use std::env::consts;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
pub fn get_archive_file_path(version: &str) -> Result<String, ProtoError> {
    let arch = NodeArch::from_os_arch()?;

    if !matches!(arch, NodeArch::X64 | NodeArch::Arm64) {
        return Err(ProtoError::UnsupportedArchitecture(
            "Node.js".into(),
            arch.to_string(),
        ));
    }

    Ok(format!("node-v{version}-darwin-{arch}"))
}

#[cfg(target_os = "linux")]
pub fn get_archive_file_path(version: &str) -> Result<String, ProtoError> {
    let arch = NodeArch::from_os_arch()?;

    if !matches!(
        arch,
        NodeArch::X64 | NodeArch::Arm | NodeArch::Arm64 | NodeArch::Ppc64 | NodeArch::S390x
    ) {
        return Err(ProtoError::UnsupportedArchitecture(
            "Node.js".into(),
            arch.to_string(),
        ));
    }

    Ok(format!("node-v{version}-linux-{arch}"))
}

#[cfg(target_os = "windows")]
pub fn get_archive_file_path(version: &str) -> Result<String, ProtoError> {
    let arch = NodeArch::from_os_arch()?;

    if !matches!(arch, NodeArch::X64 | NodeArch::X86) {
        return Err(ProtoError::UnsupportedArchitecture(
            "Node.js".into(),
            arch.to_string(),
        ));
    }

    Ok(format!("node-v{version}-win-{arch}"))
}

pub fn get_archive_file(version: &str) -> Result<String, ProtoError> {
    let ext = if consts::OS == "windows" {
        "zip"
    } else {
        "tar.gz"
    };

    Ok(format!("{}.{}", get_archive_file_path(version)?, ext))
}

#[async_trait]
impl Downloadable<'_> for NodeLanguage {
    fn get_download_path(&self) -> Result<PathBuf, ProtoError> {
        Ok(self
            .temp_dir
            .join(get_archive_file(self.get_resolved_version())?))
    }

    async fn download(&self, to_file: &Path, from_url: Option<&str>) -> Result<bool, ProtoError> {
        if to_file.exists() {
            debug!(target: self.get_log_target(), "Tool already downloaded, continuing");

            return Ok(false);
        }

        let version = self.get_resolved_version();
        let from_url = match from_url {
            Some(url) => url.to_owned(),
            None => {
                format!(
                    "https://nodejs.org/dist/v{}/{}",
                    version,
                    get_archive_file(version)?
                )
            }
        };

        debug!(target: self.get_log_target(), "Attempting to download tool from {}", from_url);

        download_from_url(&from_url, &to_file).await?;

        debug!(target: self.get_log_target(), "Successfully downloaded tool");

        Ok(true)
    }
}
