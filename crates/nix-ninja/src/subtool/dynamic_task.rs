use anyhow::{anyhow, Result};
use harmonia_store_derivation::derivation::Derivation;
use harmonia_store_derivation::derived_path::{OutputName, SingleDerivedPath};
use harmonia_store_path::{StoreDir, StorePath};
use nix_builder_rpc_client::BuilderRpcClient;
use nix_ninja_task::derived_file::DerivedFile;
use std::sync::Arc;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use crate::task::discover_c_includes;

pub fn run(store_dir: &StoreDir, targets: Vec<String>) -> Result<()> {
    let input_drv = targets
        .first()
        .ok_or_else(|| anyhow!("Expected derivation path as argument"))?;

    let drv_json = fs::read_to_string(input_drv)?;
    let mut drv: Derivation = serde_json::from_str(&drv_json)?;
    println!("nix-ninja-dynamic-task: Processing derivation {}", drv.name);

    let rpc_client = Arc::new(BuilderRpcClient::connect_from_env()?);

    // Stage 1: Prepare build environment
    let (build_dir, built_paths) = prepare_build_environment(store_dir)?;

    // Stage 2: Discover dynamic dependencies
    let (discovered_deps, discovered_store_paths) =
        discover_dynamic_dependencies(&rpc_client, store_dir, &build_dir, &drv, built_paths)?;

    // Stage 3: Update derivation with discovered dependencies
    let new_deps = update_derivation_with_discoveries(
        &mut drv,
        discovered_deps,
        discovered_store_paths,
        store_dir,
    )?;

    // Print discovery results
    if !new_deps.is_empty() {
        for dep in &new_deps {
            println!(
                "nix-ninja-dynamic-task: Discovered dependency: {}",
                dep.derived_path.root_path()
            );
        }
    } else {
        println!("nix-ninja-dynamic-task: No new dependencies discovered");
    }

    let drv_path = rpc_client.add_drv_to_store(store_dir, &drv)?;

    rpc_client.submit_output(
        &SingleDerivedPath::Opaque(drv_path.clone()),
        &OutputName::default(),
    )?;

    println!("nix-ninja-dynamic-task: Added derivation to store: {drv_path}");
    Ok(())
}

/// Stage 1: Prepare build environment by setting up directories, copying source,
/// and building derived files
fn prepare_build_environment(store_dir: &StoreDir) -> Result<(PathBuf, HashMap<PathBuf, PathBuf>)> {
    // Set up build directory using NIX_BUILD_TOP
    let build_top =
        env::var("NIX_BUILD_TOP").map_err(|_| anyhow!("Expected $NIX_BUILD_TOP to be set"))?;
    let source_dir = PathBuf::from(build_top).join("source");
    let build_dir = source_dir.join("build");
    fs::create_dir_all(&build_dir)?;
    env::set_current_dir(&build_dir)?;

    // Copy $src into source_dir so we can discover dependencies from $src.
    let src = env::var("src").map_err(|_| anyhow!("Expected $src to be set"))?;
    copy_dir_all(PathBuf::from(src), &source_dir)?;

    // Get NIX_NINJA_INPUTS from process environment, these are the built
    // inputs to a derivation that may have discovered inputs and should be
    // scanned.
    let inputs = env::var("NIX_NINJA_INPUTS")
        .map_err(|_| anyhow!("NIX_NINJA_INPUTS not found in process environment"))?;

    // Get built inputs for dynamic dependency discovery
    let derived_files: Vec<DerivedFile> = inputs
        .split_whitespace()
        .filter_map(|encoded| DerivedFile::from_encoded(store_dir, encoded).ok())
        .collect();

    // In derivation mode, built files are already available as store paths
    // Create the virtual paths mapping from the derived files
    let built_paths: HashMap<PathBuf, PathBuf> = derived_files
        .iter()
        .map(|df| (df.build_path.clone(), df.absolute_path(store_dir)))
        .collect();

    Ok((build_dir, built_paths))
}

/// Stage 2: Discover dynamic dependencies by analyzing built files for includes
pub fn discover_dynamic_dependencies(
    rpc_client: &Arc<BuilderRpcClient>,
    store_dir: &StoreDir,
    build_dir: &Path,
    drv: &Derivation,
    built_paths: HashMap<PathBuf, PathBuf>,
) -> Result<(Vec<DerivedFile>, Vec<StorePath>)> {
    let cmdline_bytes = drv
        .args
        .first()
        .ok_or_else(|| anyhow!("No command line found in derivation"))?;
    let cmdline = std::str::from_utf8(cmdline_bytes)?;

    let files: Vec<PathBuf> = built_paths.keys().cloned().collect();

    discover_c_includes(
        rpc_client,
        store_dir,
        build_dir,
        cmdline,
        files,
        Some(built_paths),
    )
}

/// Stage 3: Update derivation with discovered dependencies and store paths
/// Returns the list of new dependencies that were added
pub fn update_derivation_with_discoveries(
    drv: &mut Derivation,
    discovered_deps: Vec<DerivedFile>,
    discovered_store_paths: Vec<StorePath>,
    store_dir: &StoreDir,
) -> Result<Vec<DerivedFile>> {
    for store_path in &discovered_store_paths {
        drv.inputs
            .insert(SingleDerivedPath::Opaque(store_path.clone()));
    }

    // Get NIX_NINJA_INPUTS from derivation environment, these are the existing
    // inputs of the derivation without the discovered inputs.
    let key = b"NIX_NINJA_INPUTS";
    let drv_inputs = drv
        .env
        .iter()
        .find(|(k, _)| k.as_ref() == key)
        .map_or("", |(_, v)| std::str::from_utf8(v).unwrap());

    // Parse existing derivation inputs into a HashSet for deduplication
    let mut input_set: HashSet<String> = drv_inputs
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    let mut new_deps = Vec::new();
    for derived_file in discovered_deps {
        let encoded = derived_file.to_encoded(store_dir);

        // Skip if already in input set
        if input_set.contains(&encoded) {
            continue;
        }

        new_deps.push(derived_file.clone());
        input_set.insert(encoded);
        drv.inputs.insert(derived_file.derived_path.clone());
    }

    if !new_deps.is_empty() {
        // Update NIX_NINJA_INPUTS with sorted list
        let mut inputs: Vec<String> = input_set.into_iter().collect();
        inputs.sort();
        drv.env.insert(
            b"NIX_NINJA_INPUTS"[..].into(),
            inputs.join(" ").into_bytes().into(),
        );
    }

    Ok(new_deps)
}

/// Recursively copies a directory and all its contents
fn copy_dir_all(src: PathBuf, dst: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;
    use walkdir::WalkDir;

    for entry in WalkDir::new(&src) {
        let entry = entry?;

        let relative_path = entry.path().strip_prefix(&src)?;
        let dest_path = dst.join(relative_path);

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file_type = entry.file_type();
        if file_type.is_dir() {
            fs::create_dir_all(&dest_path)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(entry.path())?;
            symlink(target, dest_path)?;
        } else {
            fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}
