use crate::downloader::Downloadable;
use log::trace;
use proto_error::ProtoError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

#[async_trait::async_trait]
pub trait Verifiable<'tool>: Send + Sync + Downloadable<'tool> {
    /// Returns an absolute file path to the checksum file.
    /// This may not exist, as the path is composed ahead of time.
    /// This is typically ~/.prove/temp/<file>.
    fn get_checksum_path(&self) -> Result<PathBuf, ProtoError>;

    /// If applicable, download all files necessary for verifying checksums.
    async fn download_checksum(
        &self,
        to_file: &Path,
        from_url: Option<&str>,
    ) -> Result<bool, ProtoError>;

    /// Verify the downloaded file using the checksum strategy for the tool.
    /// Common strategies are SHA256 and MD5.
    async fn verify_checksum(
        &self,
        checksum_file: &Path,
        download_file: &Path,
    ) -> Result<bool, ProtoError>;
}

pub fn get_sha256_hash_of_file<P: AsRef<Path>>(path: P) -> Result<String, ProtoError> {
    let path = path.as_ref();
    let handle_error = |e: io::Error| ProtoError::Fs(path.to_path_buf(), e.to_string());

    trace!(
        target: "proto:verifier",
        "Calculating SHA256 checksum for file {}",
        path.to_string_lossy()
    );

    let mut file = File::open(path).map_err(handle_error)?;
    let mut sha = Sha256::new();

    io::copy(&mut file, &mut sha).map_err(handle_error)?;

    let hash = format!("{:x}", sha.finalize());

    trace!(
        target: "proto:verifier",
        "Calculated hash {}",
        hash
    );

    Ok(hash)
}
