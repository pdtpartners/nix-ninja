use anyhow::{anyhow, Result};
use nix_libstore::store_path::StorePath;
use nix_libstore::{derived_path::SingleDerivedPath, prelude::Placeholder};
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DerivedFile {
    pub path: SingleDerivedPath,
    pub source: PathBuf,
}

impl DerivedFile {
    pub fn to_encoded(&self) -> String {
        format!(
            "{}:{}",
            self.path.to_input().display(),
            &self.source.to_string_lossy()
        )
    }

    pub fn from_encoded(encoded: &str) -> Result<Self> {
        // Split by colon to separate path from source
        let parts: Vec<&str> = encoded.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!(
                "Expected one ':' in encoded derived file but got {}",
                encoded
            ));
        }

        let store_path = StorePath::new(parts[0])?;
        let path = SingleDerivedPath::Opaque(store_path);
        let source = PathBuf::from(parts[1]);

        Ok(DerivedFile { path, source })
    }
}

impl fmt::Display for DerivedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)
    }
}

pub struct DerivedOutput {
    pub placeholder: Placeholder,
    pub source: PathBuf,
}

impl DerivedOutput {
    pub fn to_encoded(&self) -> String {
        format!(
            "{}:{}",
            &self.placeholder.render().display(),
            &self.source.display()
        )
    }
}
