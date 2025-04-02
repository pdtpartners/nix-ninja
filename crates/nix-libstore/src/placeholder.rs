use std::path::PathBuf;

use crate::store_path::StorePath;
use anyhow::anyhow;
use nix_base32;

/// A placeholder for a Nix store path
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// The hash of the placeholder
    hash: Vec<u8>,
}

impl Placeholder {
    /// Create a new placeholder from a hash
    fn new(hash: Vec<u8>) -> Self {
        Self { hash }
    }

    /// Render the placeholder as a string
    pub fn render(&self) -> PathBuf {
        PathBuf::from(format!("/{}", nix_base32::to_nix_base32(&self.hash)))
    }

    /// Generate a placeholder for a standard output
    pub fn standard_output(output_name: &str) -> Self {
        let clear_text = format!("nix-output:{}", output_name);
        let hash = sha256_hash(clear_text.as_bytes());
        Self::new(hash)
    }

    /// Generate a placeholder for a content-addressed derivation output
    pub fn ca_output(drv_path: &StorePath, output_name: &str) -> Self {
        let drv_name = drv_path.name();
        let drv_name = if drv_name.ends_with(".drv") {
            &drv_name[0..drv_name.len() - 4]
        } else {
            drv_name
        };

        // Format the output path name according to Nix conventions
        let output_path_name = output_path_name(drv_name, output_name);

        let clear_text = format!(
            "nix-upstream-output:{}:{}",
            drv_path.hash_part(),
            output_path_name
        );

        let hash = sha256_hash(clear_text.as_bytes());
        Self::new(hash)
    }

    /// Generate a placeholder for a dynamic derivation output
    pub fn dynamic_output(placeholder: &Placeholder, output_name: &str) -> Self {
        // Compress the hash according to Nix's implementation
        let compressed = compress_hash(&placeholder.hash, 20);

        let compressed_str = nix_base32::to_nix_base32(&compressed);
        let clear_text = format!("nix-computed-output:{}:{}", compressed_str, output_name);

        let hash = sha256_hash(clear_text.as_bytes());
        Self::new(hash)
    }
}

impl TryFrom<String> for Placeholder {
    type Error = anyhow::Error;

    fn try_from(str: String) -> Result<Self, Self::Error> {
        let hash = match nix_base32::from_nix_base32(&str) {
            Some(h) => h,
            None => {
                return Err(anyhow!("Not valid nix base32 string: {}", str));
            }
        };

        Ok(Placeholder::new(hash))
    }
}

/// Format an output path name according to Nix conventions
pub fn output_path_name(drv_name: &str, output_name: &str) -> String {
    if output_name == "out" {
        drv_name.to_string()
    } else {
        format!("{}-{}", drv_name, output_name)
    }
}

/// Compress a hash to a smaller size by XORing bytes
fn compress_hash(hash: &[u8], new_size: usize) -> Vec<u8> {
    if hash.is_empty() {
        return vec![];
    }

    let mut result = vec![0u8; new_size];

    for (i, &byte) in hash.iter().enumerate() {
        result[i % new_size] ^= byte;
    }

    result
}

/// Calculate SHA-256 hash of data
fn sha256_hash(data: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_placeholder() {
        let placeholder = Placeholder::standard_output("out");
        assert_eq!(
            placeholder.render(),
            PathBuf::from("/1rz4g4znpzjwh1xymhjpm42vipw92pr73vdgl6xs1hycac8kf2n9")
        );
    }

    #[test]
    fn test_ca_placeholder() {
        let store_path =
            StorePath::new("/nix/store/g1w7hy3qg1w7hy3qg1w7hy3qg1w7hy3q-foo.drv").unwrap();
        let placeholder = Placeholder::ca_output(&store_path, "out");
        assert_eq!(
            placeholder.render(),
            PathBuf::from("/0c6rn30q4frawknapgwq386zq358m8r6msvywcvc89n6m5p2dgbz")
        );
    }

    #[test]
    fn test_dynamic_placeholder() {
        let store_path =
            StorePath::new("/nix/store/g1w7hy3qg1w7hy3qg1w7hy3qg1w7hy3q-foo.drv.drv").unwrap();
        let placeholder = Placeholder::ca_output(&store_path, "out");
        let dynamic = Placeholder::dynamic_output(&placeholder, "out");
        assert_eq!(
            dynamic.render(),
            PathBuf::from("/0gn6agqxjyyalf0dpihgyf49xq5hqxgw100f0wydnj6yqrhqsb3w"),
        )
    }

    #[test]
    fn test_store_path_parsing() {
        let path = StorePath::new("/nix/store/ac8da0sqpg4pyhzyr0qgl26d5dnpn7qp-hello-2.10.tar.gz")
            .unwrap();
        assert_eq!(path.hash_part(), "ac8da0sqpg4pyhzyr0qgl26d5dnpn7qp");
        assert_eq!(path.name(), "hello-2.10.tar.gz");

        // Test with a derivation path
        let drv_path =
            StorePath::new("/nix/store/q3lv9bi7r4di3kxdjhy7kvwgvpmanfza-hello-2.10.drv").unwrap();
        assert_eq!(drv_path.hash_part(), "q3lv9bi7r4di3kxdjhy7kvwgvpmanfza");
        assert_eq!(drv_path.name(), "hello-2.10.drv");
        assert!(drv_path.is_derivation());
    }

    #[test]
    fn test_output_path_name() {
        // Test with "out" output
        assert_eq!(output_path_name("hello-2.10", "out"), "hello-2.10");

        // Test with non-"out" output
        assert_eq!(output_path_name("hello-2.10", "bin"), "hello-2.10-bin");
        assert_eq!(output_path_name("hello-2.10", "dev"), "hello-2.10-dev");
    }
}
