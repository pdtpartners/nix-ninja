use crate::derived_path::{SingleDerivedPath, SingleDerivedPathBuilt};
use crate::store_path::StorePath;
use anyhow::Result;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{HashMap, HashSet};

/// A Nix derivation, matching Nix's JSON derivation format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Derivation {
    /// The name of the derivation
    pub name: String,

    /// The system type (e.g., "x86_64-linux")
    pub system: String,

    /// The builder executable path
    pub builder: String,

    /// Arguments to pass to the builder
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables for the build
    #[serde(default, serialize_with = "serialize_hashmap_sorted")]
    pub env: HashMap<String, String>,

    /// Input derivations
    #[serde(
        default,
        rename = "inputDrvs",
        serialize_with = "serialize_hashmap_sorted"
    )]
    pub input_drvs: HashMap<String, InputDrv>,

    /// Input sources (store paths)
    #[serde(
        default,
        rename = "inputSrcs",
        serialize_with = "serialize_hashset_as_vec"
    )]
    pub input_srcs: HashSet<String>,

    /// Output specifications
    #[serde(serialize_with = "serialize_hashmap_sorted")]
    pub outputs: HashMap<String, Output>,
}

/// Input derivation specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputDrv {
    /// Outputs of the input derivation
    pub outputs: Vec<String>,

    /// Dynamic outputs for dynamic derivations
    #[serde(
        default,
        rename = "dynamicOutputs",
        serialize_with = "serialize_hashmap_sorted"
    )]
    pub dynamic_outputs: HashMap<String, DynamicOutput>,
}

/// Dynamic output specification for dynamic derivations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicOutput {
    /// Outputs of the dynamic derivation
    pub outputs: Vec<String>,

    /// Nested dynamic outputs
    #[serde(
        default,
        rename = "dynamicOutputs",
        serialize_with = "serialize_hashmap_sorted"
    )]
    pub dynamic_outputs: HashMap<String, DynamicOutput>,
}

/// Output specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    /// Hash algorithm for content-addressed derivations
    #[serde(skip_serializing_if = "Option::is_none", rename = "hashAlgo")]
    pub hash_algo: Option<HashAlgorithm>,

    /// Output hash mode for content-addressed derivations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<OutputHashMode>,

    /// Output hash for fixed-output derivations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// Hash algorithm used for Nix operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    #[serde(rename = "sha256")]
    Sha256,
    #[serde(rename = "sha512")]
    Sha512,
}

/// Output hash mode for derivations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputHashMode {
    #[serde(rename = "flat")]
    Flat,
    #[serde(rename = "nar")]
    Nar,
    #[serde(rename = "text")]
    Text,
}

impl Derivation {
    /// Create a new derivation
    pub fn new(name: &str, system: &str, builder: &str) -> Self {
        Self {
            name: name.to_string(),
            system: system.to_string(),
            builder: builder.to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            input_drvs: HashMap::new(),
            input_srcs: HashSet::new(),
            outputs: HashMap::new(),
        }
    }

    /// Add an argument to the builder
    pub fn add_arg(&mut self, arg: &str) -> &mut Self {
        self.args.push(arg.to_string());
        self
    }

    /// Set an environment variable
    pub fn set_env(&mut self, key: &str, value: &str) -> &mut Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    /// Add an output
    pub fn add_output(
        &mut self,
        name: &str,
        hash_algo: Option<HashAlgorithm>,
        method: Option<OutputHashMode>,
        hash: Option<String>,
    ) -> &mut Self {
        self.outputs.insert(
            name.to_string(),
            Output {
                hash_algo,
                method,
                hash,
            },
        );
        self
    }

    /// Add a content-addressed output
    pub fn add_ca_output(
        &mut self,
        name: &str,
        hash_algo: HashAlgorithm,
        method: OutputHashMode,
    ) -> &mut Self {
        self.outputs.insert(
            name.to_string(),
            Output {
                hash_algo: Some(hash_algo),
                method: Some(method),
                hash: None,
            },
        );
        self
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Serialize to pretty-printed JSON
    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Deserialize from JSON
    pub fn from_json(json: &str) -> Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    /// Add an input source
    pub fn add_input_src(&mut self, store_path: &StorePath) -> &mut Self {
        self.input_srcs.insert(store_path.to_string());
        self
    }

    /// Add a derived path as input (either source or derivation)
    pub fn add_derived_path(&mut self, derived_path: &SingleDerivedPath) -> &mut Self {
        match derived_path {
            SingleDerivedPath::Opaque(store_path) => {
                // For opaque paths, add as input source
                self.add_input_src(store_path);
            }
            SingleDerivedPath::Built(built) => {
                self.add_input_built(built);
            }
        }
        self
    }

    /// Add a built derivation path as an input to this derivation
    fn add_input_built(&mut self, built: &SingleDerivedPathBuilt) {
        let drv_store_path = built.derived_path.store_path().to_string();
        let input_drv = self
            .input_drvs
            .entry(drv_store_path)
            .or_insert_with(|| InputDrv {
                outputs: vec![],
                dynamic_outputs: HashMap::new(),
            });

        Self::add_built_nested(
            &mut input_drv.outputs,
            &mut input_drv.dynamic_outputs,
            built,
        );
    }

    /// Add a built path with potentially nested dynamic derivation structure
    fn add_built_nested(
        outputs: &mut Vec<String>,
        dynamic_outputs: &mut HashMap<String, DynamicOutput>,
        built: &SingleDerivedPathBuilt,
    ) {
        // Extract chain of output names from outermost to innermost
        let mut chain = Vec::new();
        let mut current = built;

        loop {
            chain.push(current.output.clone());
            match current.derived_path.as_ref() {
                SingleDerivedPath::Opaque(_) => break,
                SingleDerivedPath::Built(inner) => current = inner,
            }
        }

        // Reverse to process innermost to outermost
        chain.reverse();

        // Split into intermediate levels and final output
        let Some((final_output, intermediate_levels)) = chain.split_last() else {
            return;
        };

        // Navigate through intermediate levels, creating dynamic outputs
        let mut current_outputs = outputs;
        let mut current_dynamics = dynamic_outputs;

        for level in intermediate_levels {
            let dynamic_output =
                current_dynamics
                    .entry(level.clone())
                    .or_insert_with(|| DynamicOutput {
                        outputs: vec![],
                        dynamic_outputs: HashMap::new(),
                    });
            current_outputs = &mut dynamic_output.outputs;
            current_dynamics = &mut dynamic_output.dynamic_outputs;
        }

        // Add final output
        if !current_outputs.contains(final_output) {
            current_outputs.push(final_output.clone());
        }
    }
}

fn serialize_hashset_as_vec<S, T>(set: &HashSet<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize + Clone + Ord,
{
    let mut vec: Vec<T> = set.iter().cloned().collect();
    vec.sort();
    vec.serialize(serializer)
}

fn serialize_hashmap_sorted<S, K, V>(map: &HashMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    K: Serialize + Clone + Ord + std::hash::Hash,
    V: Serialize + Clone,
{
    use serde::ser::SerializeMap;
    let mut sorted_keys: Vec<&K> = map.keys().collect();
    sorted_keys.sort();

    let mut map_serializer = serializer.serialize_map(Some(map.len()))?;
    for key in sorted_keys {
        let value = &map[key];
        map_serializer.serialize_entry(key, value)?;
    }
    map_serializer.end()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derived_path::SingleDerivedPathBuilt;
    use crate::store_path::StorePath;

    #[test]
    fn derivation_serialization() {
        // Create a basic derivation
        let mut drv = Derivation::new(
            "hello",
            "x86_64-linux",
            "/nix/store/w7jl0h7mwrrrcy2kgvk9c9h9142f1ca0-bash/bin/bash",
        );

        // Add some basic properties
        drv.add_arg("-c")
            .add_arg("echo Hello > $out")
            .set_env(
                "PATH",
                "/nix/store/d1pzgj1pj3nk97vhm5x6n8szy4w3xhx7-coreutils/bin",
            )
            .add_output("out", None, None, None);

        // Serialize to JSON
        let json = drv.to_json().unwrap();

        // Deserialize back
        let drv2 = Derivation::from_json(&json).unwrap();

        // Check that they match
        assert_eq!(drv.name, drv2.name);
        assert_eq!(drv.system, drv2.system);
        assert_eq!(drv.builder, drv2.builder);
        assert_eq!(drv.args, drv2.args);
        assert_eq!(drv.outputs.len(), drv2.outputs.len());
    }

    #[test]
    fn ca_derivation() {
        let mut drv = Derivation::new(
            "ca-example",
            "x86_64-linux",
            "/nix/store/w7jl0h7mwrrrcy2kgvk9c9h9142f1ca0-bash/bin/bash",
        );

        drv.add_ca_output("out", HashAlgorithm::Sha256, OutputHashMode::Nar);

        let output = drv.outputs.get("out").unwrap();
        assert_eq!(output.hash_algo, Some(HashAlgorithm::Sha256));
        assert_eq!(output.method, Some(OutputHashMode::Nar));
        assert_eq!(output.hash, None);
    }

    #[test]
    fn add_opaque_path() {
        let mut drv = Derivation::new("test", "x86_64-linux", "/bin/bash");
        let store_path1 = sample_store_path();
        let store_path2 =
            StorePath::new("/nix/store/zyxwvutsrqponmlkjihgfedcba987654-other").unwrap();
        let path1 = SingleDerivedPath::Opaque(store_path1.clone());
        let path2 = SingleDerivedPath::Opaque(store_path2.clone());

        drv.add_derived_path(&path1);
        drv.add_derived_path(&path2);

        assert!(drv.input_srcs.contains(&store_path1.to_string()));
        assert!(drv.input_srcs.contains(&store_path2.to_string()));
        assert!(drv.input_drvs.is_empty());
    }

    #[test]
    fn add_built_path() {
        let mut drv = Derivation::new("test", "x86_64-linux", "/bin/bash");
        let store_path = sample_store_path();
        let built1 = SingleDerivedPathBuilt::new(store_path.clone(), "out".to_string());
        let built2 = SingleDerivedPathBuilt::new(store_path.clone(), "dev".to_string());
        let path1 = SingleDerivedPath::Built(built1);
        let path2 = SingleDerivedPath::Built(built2);

        drv.add_derived_path(&path1);
        drv.add_derived_path(&path2);

        assert!(drv.input_srcs.is_empty());
        let input_drv = drv.input_drvs.get(&store_path.to_string()).unwrap();
        let mut outputs = input_drv.outputs.clone();
        outputs.sort();
        assert_eq!(outputs, vec!["dev", "out"]);
        assert!(input_drv.dynamic_outputs.is_empty());
    }

    #[test]
    fn add_multiple_dynamic_outputs() {
        let mut drv = Derivation::new("test", "x86_64-linux", "/bin/bash");
        let store_path = sample_store_path();

        // Add first dynamic derivation: store_path^inner^output1
        let inner1 = SingleDerivedPathBuilt::new(store_path.clone(), "inner".to_string());
        let inner_path1 = SingleDerivedPath::Built(inner1);
        let outer1 = SingleDerivedPathBuilt::from_derived_path(inner_path1, "output1".to_string());
        let path1 = SingleDerivedPath::Built(outer1);

        drv.add_derived_path(&path1);

        // Check first dynamic path was added correctly
        assert!(drv.input_srcs.is_empty());
        let input_drv = drv.input_drvs.get(&store_path.to_string()).unwrap();
        assert!(input_drv.outputs.is_empty());
        let dynamic_output = input_drv.dynamic_outputs.get("inner").unwrap();
        assert_eq!(dynamic_output.outputs, vec!["output1"]);

        // Add second dynamic derivation with same inner output: store_path^inner^output2
        let inner2 = SingleDerivedPathBuilt::new(store_path.clone(), "inner".to_string());
        let inner_path2 = SingleDerivedPath::Built(inner2);
        let outer2 = SingleDerivedPathBuilt::from_derived_path(inner_path2, "output2".to_string());
        let path2 = SingleDerivedPath::Built(outer2);

        drv.add_derived_path(&path2);

        // Check aggregation: both outputs under same dynamic output
        let input_drv = drv.input_drvs.get(&store_path.to_string()).unwrap();
        let dynamic_output = input_drv.dynamic_outputs.get("inner").unwrap();
        let mut outputs = dynamic_output.outputs.clone();
        outputs.sort();
        assert_eq!(outputs, vec!["output1", "output2"]);
        assert!(dynamic_output.dynamic_outputs.is_empty());
    }

    #[test]
    fn add_nested_dynamic_output() {
        let mut drv = Derivation::new("test", "x86_64-linux", "/bin/bash");
        let store_path = sample_store_path();

        // Create deeply nested structure: store_path^level1^level2^level3^output
        let level1 = SingleDerivedPathBuilt::new(store_path.clone(), "level1".to_string());
        let level1_path = SingleDerivedPath::Built(level1);
        let level2 = SingleDerivedPathBuilt::from_derived_path(level1_path, "level2".to_string());
        let level2_path = SingleDerivedPath::Built(level2);
        let level3 = SingleDerivedPathBuilt::from_derived_path(level2_path, "level3".to_string());
        let level3_path = SingleDerivedPath::Built(level3);
        let final_output =
            SingleDerivedPathBuilt::from_derived_path(level3_path, "output".to_string());
        let path = SingleDerivedPath::Built(final_output);

        drv.add_derived_path(&path);

        // Should handle arbitrarily deep nesting
        assert!(drv.input_srcs.is_empty());
        let input_drv = drv.input_drvs.get(&store_path.to_string()).unwrap();
        assert!(input_drv.outputs.is_empty());

        // Navigate the nested structure
        let level1_dynamic = input_drv.dynamic_outputs.get("level1").unwrap();
        assert!(level1_dynamic.outputs.is_empty());

        let level2_dynamic = level1_dynamic.dynamic_outputs.get("level2").unwrap();
        assert!(level2_dynamic.outputs.is_empty());

        let level3_dynamic = level2_dynamic.dynamic_outputs.get("level3").unwrap();
        assert_eq!(level3_dynamic.outputs, vec!["output"]);
        assert!(level3_dynamic.dynamic_outputs.is_empty());
    }

    fn sample_store_path() -> StorePath {
        StorePath::new("/nix/store/abcdefghijklmnopqrstuvwxyz123456-test").unwrap()
    }
}
