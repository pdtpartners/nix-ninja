use anyhow::{anyhow, Result};
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

    /// Add an environment variable
    pub fn add_env(&mut self, key: &str, value: &str) -> &mut Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    /// Add an input source
    pub fn add_input_src(&mut self, path: &str) -> &mut Self {
        self.input_srcs.insert(path.to_string());
        self
    }

    /// Add an input derivation
    pub fn add_input_drv(&mut self, path: &str, outputs: Vec<String>) -> &mut Self {
        let input_drv = self
            .input_drvs
            .entry(path.to_string())
            .or_insert_with(|| InputDrv {
                outputs: vec![],
                dynamic_outputs: HashMap::new(),
            });
        input_drv.outputs.extend(outputs);
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

    /// Add a dynamic output to an input derivation
    pub fn add_dynamic_output(
        &mut self,
        drv_path: &str,
        output_name: &str,
        outputs: Vec<String>,
    ) -> Result<&mut Self> {
        self.add_input_drv(drv_path, vec![]);

        let input_drv = self
            .input_drvs
            .get_mut(drv_path)
            .ok_or_else(|| anyhow!("Input derivation not found: {}", drv_path))?;

        input_drv.dynamic_outputs.insert(
            output_name.to_string(),
            DynamicOutput {
                outputs,
                dynamic_outputs: HashMap::new(),
            },
        );

        Ok(self)
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

    #[test]
    fn test_derivation_serialization() {
        // Create a basic derivation
        let mut drv = Derivation::new(
            "hello",
            "x86_64-linux",
            "/nix/store/w7jl0h7mwrrrcy2kgvk9c9h9142f1ca0-bash/bin/bash",
        );

        // Add some basic properties
        drv.add_arg("-c")
            .add_arg("echo Hello > $out")
            .add_env(
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
    fn test_ca_derivation() {
        // Create a content-addressed derivation
        let mut drv = Derivation::new(
            "ca-example",
            "x86_64-linux",
            "/nix/store/w7jl0h7mwrrrcy2kgvk9c9h9142f1ca0-bash/bin/bash",
        );

        // Add a content-addressed output
        drv.add_ca_output("out", HashAlgorithm::Sha256, OutputHashMode::Nar);

        // Serialize to JSON
        let json = drv.to_json().unwrap();

        // Check that it contains the content-addressed output properties
        assert!(json.contains("sha256"));
        assert!(json.contains("nar"));
    }

    #[test]
    fn test_dynamic_derivation() {
        // Create a derivation with dynamic outputs
        let mut drv = Derivation::new(
            "dynamic-example",
            "x86_64-linux",
            "/nix/store/w7jl0h7mwrrrcy2kgvk9c9h9142f1ca0-bash/bin/bash",
        );

        // Add an input derivation
        drv.add_input_drv(
            "/nix/store/ac8da0sqpg4pyhzyr0qgl26d5dnpn7qp-ca-example.drv",
            vec![],
        );

        // Add a dynamic output
        drv.add_dynamic_output(
            "/nix/store/ac8da0sqpg4pyhzyr0qgl26d5dnpn7qp-ca-example.drv",
            "out",
            vec!["out".to_string()],
        )
        .unwrap();

        // Serialize to JSON
        let json = drv.to_json().unwrap();

        // Check that it contains the dynamic outputs
        assert!(json.contains("dynamicOutputs"));
    }
}
