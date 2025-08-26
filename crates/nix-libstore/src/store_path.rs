use anyhow::{anyhow, Result};
use std::fmt;
use std::path::PathBuf;

/// A Nix store path
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StorePath {
    /// The full path including the store directory
    path: PathBuf,
}

impl StorePath {
    /// Create a new store path, validating that it follows Nix path conventions
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();

        // Validate the path has a filename
        let filename = path_buf
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("Invalid store path: missing filename"))?;

        // Validate the filename has the expected format with a 32-character hash
        if filename.len() <= 33 || filename.chars().nth(32) != Some('-') {
            return Err(anyhow!(
                "Invalid store path: expected 32-character hash followed by dash: {}",
                filename
            ));
        }

        Ok(Self { path: path_buf })
    }

    /// Get the hash part of the store path (always 32 characters)
    pub fn hash_part(&self) -> &str {
        let filename = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("StorePath was validated at construction");

        &filename[0..32]
    }

    /// Get the name part of the store path (after the hash and dash)
    pub fn name(&self) -> &str {
        let filename = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("StorePath was validated at construction");

        &filename[33..]
    }

    /// Get the full path
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Check if this is a derivation path
    pub fn is_derivation(&self) -> bool {
        self.name().ends_with(".drv")
    }
}

impl fmt::Display for StorePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.to_string_lossy())
    }
}
