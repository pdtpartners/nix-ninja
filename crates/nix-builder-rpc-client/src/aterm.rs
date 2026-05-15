//! Re-exports harmonia's ATerm encoder for convenience, plus a small
//! helper for the calling-drv name check.

pub use harmonia_store_aterm::print_derivation_aterm;

/// `outputPathName(drvName, outputName)` from
/// `nix/src/libstore/derivations.cc`. Used to compute the canonical store
/// path *name* for an output of a given drv — needed for the rename trick
/// (re-upload bytes under this name so the daemon's name check passes).
pub fn output_path_name(drv_name: &str, output_name: &str) -> String {
    if output_name == "out" {
        drv_name.to_string()
    } else {
        format!("{drv_name}-{output_name}")
    }
}
