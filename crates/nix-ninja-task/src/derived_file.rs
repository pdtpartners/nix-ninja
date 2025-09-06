use anyhow::{anyhow, Result};
use nix_libstore::derived_path::SingleDerivedPath;
use nix_libstore::store_path::StorePath;
use std::fmt;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

/// Represents a file input or output for nix-ninja-task builds.
///
/// DerivedFile describes how files are arranged in the build directory that nix-ninja-task
/// creates. The build directory contains symlinks that recreate the original source structure,
/// allowing builds to reference files using relative paths while the actual files come from
/// various Nix store locations.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DerivedFile {
    pub derived_path: SingleDerivedPath,
    pub build_path: PathBuf, // Where file appears in build dir (symlink destination)
    pub rel_path: Option<PathBuf>, // Where file appears within derived path (None for opaque)
}

impl DerivedFile {
    /// Encodes this DerivedFile for passing from nix-ninja to nix-ninja-task.
    ///
    /// Format: `"<path_or_placeholder>:<build_path>:<rel_path>"`
    pub fn to_encoded(&self) -> String {
        let path_str = match &self.derived_path {
            SingleDerivedPath::Opaque(store_path) => {
                store_path.path().to_string_lossy().to_string()
            }
            SingleDerivedPath::Built(built_path) => {
                built_path.placeholder().to_string_lossy().to_string()
            }
        };
        let rel_path_str = self
            .rel_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        format!(
            "{}:{}:{}",
            path_str,
            &self.build_path.to_string_lossy(),
            rel_path_str
        )
    }

    /// Decodes a DerivedFile from the string format created by `to_encoded()`.
    /// Used by nix-ninja-task to recreate build directory symlinks.
    pub fn from_encoded(encoded: &str) -> Result<Self> {
        let mut parts = encoded.split(':');
        let store_path =
            StorePath::new(parts.next().ok_or_else(|| {
                anyhow!("Missing store path in encoded derived file: {encoded}")
            })?)?;
        let derived_path = SingleDerivedPath::Opaque(store_path);
        let build_path = PathBuf::from(
            parts
                .next()
                .ok_or_else(|| anyhow!("Missing build path in encoded derived file: {encoded}"))?,
        );
        let rel_path = parts.next().filter(|s| !s.is_empty()).map(PathBuf::from);

        Ok(DerivedFile {
            derived_path,
            build_path,
            rel_path,
        })
    }
}

impl fmt::Display for DerivedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base_path = match &self.derived_path {
            SingleDerivedPath::Opaque(store_path) => store_path.path().clone(),
            SingleDerivedPath::Built(built_path) => built_path.placeholder(),
        };
        if let Some(rel_path) = &self.rel_path {
            write!(f, "{}", base_path.join(rel_path).to_string_lossy())
        } else {
            write!(f, "{}", base_path.to_string_lossy())
        }
    }
}

impl From<&DerivedFile> for PathBuf {
    fn from(df: &DerivedFile) -> Self {
        let base_path = match &df.derived_path {
            SingleDerivedPath::Opaque(store_path) => store_path.path().clone(),
            SingleDerivedPath::Built(built_path) => built_path.placeholder(),
        };
        if let Some(rel_path) = &df.rel_path {
            base_path.join(rel_path)
        } else {
            base_path
        }
    }
}

/// Creates symlinks for derived files under the specified prefix.
///
/// For each derived file, creates a symlink at `prefix/${derived_file.build_path}`
/// pointing to the actual file at `derived_file.rel_path`.
pub fn create_symlinks(
    prefix: &std::path::Path,
    inputs: Vec<DerivedFile>,
    overwrite: bool,
) -> Result<()> {
    for input in inputs {
        let source_path = input.to_string();
        let dest_path = prefix.join(&input.build_path);

        // Create parent directories if they don't exist
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if !std::path::Path::new(&source_path).exists() {
            return Err(anyhow!(
                "nix-ninja-task: symlink source does not exist: {source_path}"
            ));
        }

        if overwrite && dest_path.exists() {
            fs::remove_file(&dest_path)?;
        }

        symlink(&source_path, &dest_path).map_err(|e| {
            anyhow!(
                "Failed to create symlink from {} to {}: {}",
                source_path,
                dest_path.display(),
                e
            )
        })?;
    }

    Ok(())
}
