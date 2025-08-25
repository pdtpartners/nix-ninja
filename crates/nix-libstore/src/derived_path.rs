use std::fmt;
use std::path::PathBuf;

use crate::placeholder::Placeholder;
use crate::store_path::StorePath;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SingleDerivedPath {
    Opaque(StorePath),
    Built(SingleDerivedPathBuilt),
}

impl SingleDerivedPath {
    pub fn store_path(&self) -> StorePath {
        match self {
            SingleDerivedPath::Opaque(store_path) => store_path.clone(),
            SingleDerivedPath::Built(built_path) => built_path.drv_path.clone(),
        }
    }
}

impl fmt::Display for SingleDerivedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SingleDerivedPath::Opaque(store_path) => write!(f, "{store_path}"),
            SingleDerivedPath::Built(built_path) => write!(f, "{built_path}"),
        }
    }
}

/// A single derived path that is built from a derivation.
/// Built derived paths are a pair of a derivation and an output name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SingleDerivedPathBuilt {
    pub drv_path: StorePath,
    pub output: String,
}

impl SingleDerivedPathBuilt {
    pub fn placeholder(&self) -> PathBuf {
        Placeholder::ca_output(&self.drv_path, &self.output).render()
    }
}

impl fmt::Display for SingleDerivedPathBuilt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}^{}", &self.drv_path, &self.output)
    }
}
