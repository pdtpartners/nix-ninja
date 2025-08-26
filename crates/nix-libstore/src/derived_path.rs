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
            SingleDerivedPath::Built(built_path) => built_path.derived_path.store_path(),
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
///
/// The derivation itself can be either a store path (Opaque) or another built derivation (Built),
/// allowing for higher-order/nested dynamic derivations.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SingleDerivedPathBuilt {
    pub derived_path: Box<SingleDerivedPath>,
    pub output: String,
}

impl SingleDerivedPathBuilt {
    /// Create a new SingleDerivedPathBuilt from a store path and output name
    pub fn new(drv_path: StorePath, output: String) -> Self {
        Self {
            derived_path: Box::new(SingleDerivedPath::Opaque(drv_path)),
            output,
        }
    }

    /// Create a new SingleDerivedPathBuilt from another SingleDerivedPath and output name
    pub fn from_derived_path(drv_path: SingleDerivedPath, output: String) -> Self {
        Self {
            derived_path: Box::new(drv_path),
            output,
        }
    }

    pub fn placeholder(&self) -> PathBuf {
        self.placeholder_recursive().render()
    }

    fn placeholder_recursive(&self) -> Placeholder {
        match self.derived_path.as_ref() {
            SingleDerivedPath::Opaque(store_path) => {
                // Base case: regular ca_output placeholder
                Placeholder::ca_output(store_path, &self.output)
            }
            SingleDerivedPath::Built(inner_built) => {
                // Recursive case: create dynamic_output placeholder
                let inner_placeholder = inner_built.placeholder_recursive();
                Placeholder::dynamic_output(&inner_placeholder, &self.output)
            }
        }
    }
}

impl fmt::Display for SingleDerivedPathBuilt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}^{}", &self.derived_path, &self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_store_path() -> StorePath {
        StorePath::new("/nix/store/abcdefghijklmnopqrstuvwxyz123456-test").unwrap()
    }

    #[test]
    fn opaque_path() {
        let store_path = sample_store_path();
        let path = SingleDerivedPath::Opaque(store_path.clone());

        assert_eq!(path.store_path(), store_path);
        assert_eq!(format!("{path}"), store_path.to_string());
    }

    #[test]
    fn built_path() {
        let store_path = sample_store_path();
        let built = SingleDerivedPathBuilt::new(store_path.clone(), "out".to_string());
        let path = SingleDerivedPath::Built(built);

        assert_eq!(path.store_path(), store_path);
        assert_eq!(format!("{path}"), format!("{}^out", store_path));
    }

    #[test]
    fn nested_path() {
        let store_path = sample_store_path();

        // Create inner derivation: store-path^inner
        let inner_built = SingleDerivedPathBuilt::new(store_path.clone(), "inner".to_string());
        let inner_path = SingleDerivedPath::Built(inner_built);

        // Create outer derivation: (store-path^inner)^outer
        let outer_built =
            SingleDerivedPathBuilt::from_derived_path(inner_path, "outer".to_string());
        let outer_path = SingleDerivedPath::Built(outer_built);

        // The store_path should resolve to the innermost store path
        assert_eq!(outer_path.store_path(), store_path);

        // The display should show the full nested structure
        assert_eq!(
            format!("{outer_path}"),
            format!("{}^inner^outer", store_path)
        );
    }
}
