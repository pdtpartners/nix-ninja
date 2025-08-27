use anyhow::{anyhow, Result};
use nix_libstore::derived_path::SingleDerivedPath;
use nix_libstore::store_path::StorePath;
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DerivedFile {
    pub path: SingleDerivedPath,
    pub build_path: PathBuf, // Where file appears in build dir (symlink destination)
    pub rel_path: Option<PathBuf>, // Where file appears within derived path (None for opaque)
}

impl DerivedFile {
    pub fn to_encoded(&self) -> String {
        let path_str = match &self.path {
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

    pub fn from_encoded(encoded: &str) -> Result<Self> {
        let mut parts = encoded.split(':');
        let store_path =
            StorePath::new(parts.next().ok_or_else(|| {
                anyhow!("Missing store path in encoded derived file: {encoded}")
            })?)?;
        let path = SingleDerivedPath::Opaque(store_path);
        let build_path = PathBuf::from(
            parts
                .next()
                .ok_or_else(|| anyhow!("Missing build path in encoded derived file: {encoded}"))?,
        );
        let rel_path = parts.next().filter(|s| !s.is_empty()).map(PathBuf::from);

        Ok(DerivedFile {
            path,
            build_path,
            rel_path,
        })
    }
}

impl fmt::Display for DerivedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base_path = match &self.path {
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
