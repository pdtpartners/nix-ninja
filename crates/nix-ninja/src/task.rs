use crate::local;
use crate::relative_from::relative_from;
use crate::subtool::dynamic_task;
use anyhow::{anyhow, Error, Result};
use deps_infer::c_include_parser;
use n2::{
    canon,
    graph::{self, Build, BuildDependencies, BuildId, File, FileId},
};
use nix_libstore::prelude::*;
use nix_ninja_task::derived_file::DerivedFile;
use nix_tool::NixTool;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};
use walkdir::WalkDir;
use which::which;

#[derive(Clone)]
pub struct Tools {
    pub cc: StorePath,
    pub coreutils: StorePath,
    pub nix: StorePath,
    pub nix_tool: NixTool,
    pub nix_ninja: StorePath,
    pub nix_ninja_task: StorePath,
    pub patchelf: StorePath,
}

impl Tools {
    pub fn new(nix_tool: NixTool) -> Result<Self> {
        Ok(Tools {
            cc: which_store_path("cc")?,
            coreutils: which_store_path("coreutils")?,
            nix: which_store_path("nix")?,
            nix_tool,
            nix_ninja: which_store_path("nix-ninja")?,
            nix_ninja_task: which_store_path("nix-ninja-task")?,
            patchelf: which_store_path("patchelf")?,
        })
    }
}

/// Task represents a fully evaluated Ninja build target.
///
/// A task contains all the context to generate a Nix derivation for the build
/// target.
#[derive(Clone)]
struct Task {
    name: String,
    system: String,
    wrapper_vars: HashMap<String, String>,
    input_srcs: Vec<StorePath>,

    build_dir: PathBuf,
    build_deps: BuildDependencies,
    store_dir: PathBuf,

    cmdline: Option<String>,
    desc: Option<String>,
    deps: Option<String>,

    files: HashMap<FileId, File>,
    inputs: Vec<DerivedFile>,
    outputs: Vec<PathBuf>,
}

impl Deref for Task {
    type Target = BuildDependencies;

    fn deref(&self) -> &Self::Target {
        &self.build_deps
    }
}

/// BuildResult is the output of a Task.
pub struct BuildResult {
    pub bid: BuildId,
    pub derived_path: Option<SingleDerivedPath>,
    pub derived_files: Vec<DerivedFile>,
    pub err: Option<Error>,
}

#[derive(Clone)]
pub struct RunnerConfig {
    pub system: String,
    pub build_dir: PathBuf,
    pub store_dir: PathBuf,
    pub is_output_derivation: bool,
}

/// Runner is an async runtime that spawns threads for each task.
pub struct Runner {
    pub derived_files: HashMap<FileId, DerivedFile>,
    build_dir_inputs: HashMap<FileId, DerivedFile>,

    tx: mpsc::Sender<BuildResult>,
    rx: mpsc::Receiver<BuildResult>,
    tools: Tools,
    config: RunnerConfig,
    wrapper_vars: HashMap<String, String>,
    wrapper_store_paths: Vec<StorePath>,
    store_regex: Regex,
    nix_build_lock: Arc<Mutex<()>>,
}

impl Runner {
    pub fn new(tools: Tools, config: RunnerConfig) -> Result<Self> {
        let store_dir_str = config.store_dir.to_string_lossy();
        let pattern = format!(
            r"{}\/[a-z0-9]{{32}}-[0-9a-zA-Z\+\-\._\?=]+",
            regex::escape(&store_dir_str)
        );
        let store_regex = Regex::new(&pattern)?;

        let mut wrapper_vars = HashMap::new();
        for (key, value) in env::vars() {
            if ["NIX_LDFLAGS", "NIX_CFLAGS_COMPILE"].contains(&key.as_str())
                || key.starts_with("NIX_CC_WRAPPER")
                || key.starts_with("NIX_BINTOOLS_WRAPPER")
            {
                wrapper_vars.insert(key, value);
            }
        }

        // Remove -frandom-seed from NIX_CFLAGS_COMPILE as we'll calculate it
        // per task derivation. Otherwise this will be different every time
        // breaking incrementality.
        if let Some(cflags) = wrapper_vars.get_mut("NIX_CFLAGS_COMPILE") {
            *cflags = remove_frandom_seed(cflags);
        }

        // Extract store paths from wrapper variables once
        let mut wrapper_store_paths = Vec::new();
        for value in wrapper_vars.values() {
            let found_store_paths = extract_store_paths(&store_regex, value)?;
            wrapper_store_paths.extend(found_store_paths);
        }

        let (tx, rx) = mpsc::channel();
        Ok(Runner {
            derived_files: HashMap::new(),
            build_dir_inputs: HashMap::new(),
            tx,
            rx,
            tools,
            config,
            wrapper_vars,
            wrapper_store_paths,
            store_regex,
            nix_build_lock: Arc::new(Mutex::new(())),
        })
    }

    // Build systems like Meson may generate files via `configure_file` that are
    // not listed as implicit inputs in the build.ninja file. So we must read
    // the build directory and consider them implict inputs for all tasks.
    pub fn read_build_dir(&mut self, files: &mut graph::GraphFiles) -> Result<()> {
        for entry in WalkDir::new(&self.config.build_dir)
            .into_iter()
            .filter_entry(|e| {
                // Skip directories that start with "meson-" as they contain
                // non-deterministic internal data from meson
                !e.file_name().to_string_lossy().starts_with("meson-")
            })
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.into_path();
            let derived_file =
                new_opaque_file(&self.tools.nix_tool, &self.config.build_dir, path.clone())?;
            let fid = self.add_derived_file(files, derived_file.clone());
            self.build_dir_inputs.insert(fid, derived_file);
        }
        Ok(())
    }

    pub fn start(
        &mut self,
        files: &mut graph::GraphFiles,
        bid: BuildId,
        build: &Build,
    ) -> Result<()> {
        let tx = self.tx.clone();

        let tools = self.tools.clone();
        let task = self.new_task(files, build)?;

        let config = self.config.clone();
        let nix_build_lock = self.nix_build_lock.clone();
        std::thread::spawn(move || {
            let (derived_path, err) =
                match build_task_derivation(tools.clone(), task.clone()) {
                    Ok(drv) => match handle_derivation_result(
                        tools.clone(),
                        task.clone(),
                        drv.clone(),
                        &config,
                        nix_build_lock,
                    ) {
                        Ok(final_derived_path) => (Some(final_derived_path), None),
                        Err(err) => (None, Some(err.context(format!("Failed to handle derivation result for task '{}' (derivation: {})\nDerivation JSON:\n{}", task.name, drv.name, drv.to_json_pretty().unwrap_or_else(|_| "Failed to serialize derivation".to_string()))))),
                    },
                    Err(err) => (None, Some(err.context(format!("Failed to build task derivation for task '{}'", task.name)))),
                };

            // Create DerivedFiles for all outputs if successful
            let derived_files = if let Some(ref final_derived_path) = derived_path {
                let mut drv_outputs: Vec<DerivedFile> = Vec::new();
                for fid in task.outs() {
                    let file = &task.files[fid];
                    let built_file =
                        new_built_file(final_derived_path.clone(), file.name.clone().into());
                    drv_outputs.push(built_file);
                }
                drv_outputs
            } else {
                Vec::new()
            };

            let result = BuildResult {
                bid,
                derived_path,
                derived_files,
                err,
            };
            let _ = tx.send(result);
        });

        Ok(())
    }

    pub fn wait(&mut self, files: &mut graph::GraphFiles) -> Result<BuildId> {
        let result = self.rx.recv().unwrap();
        if let Some(err) = result.err {
            eprintln!("Error: {err}");

            eprintln!("Caused by:");
            for cause in err.chain().skip(1) {
                eprintln!("    {cause}");
            }

            eprintln!("Backtrace: {}", err.backtrace());

            let debug_info = if let Some(derived_path) = &result.derived_path {
                format!("derivation: {derived_path}")
            } else {
                format!("build_id: {:?}", result.bid)
            };

            return Err(anyhow!(
                "Failed to build task derivation for {}: {}",
                debug_info,
                err
            ));
        }

        for derived_file in result.derived_files {
            self.add_derived_file(files, derived_file.clone());
        }

        Ok(result.bid)
    }

    fn add_derived_file(
        &mut self,
        files: &mut graph::GraphFiles,
        derived_file: DerivedFile,
    ) -> FileId {
        let path_str = derived_file.build_path.to_string_lossy().into_owned();
        let fid = match files.lookup(&path_str) {
            Some(fid) => fid,
            None => files.id_from_canonical(path_str),
        };

        self.derived_files.entry(fid).or_insert(derived_file);

        fid
    }

    fn new_task(&mut self, files: &mut graph::GraphFiles, build: &Build) -> Result<Task> {
        let store_dir = self.config.store_dir.to_string_lossy().into_owned();

        // Provide the task access to all the original files for explicit
        // inputs and implicit/explicit outputs.
        let mut build_files: HashMap<FileId, File> = HashMap::new();
        for fid in build.ordering_ins().iter().chain(build.outs()) {
            build_files.insert(*fid, files.by_id[*fid].clone());
        }

        // Iterate over all explict, implicit and order-only dependencies as
        // they must all be linked into the derivation's source directory.
        let mut input_set: HashMap<PathBuf, DerivedFile> = HashMap::new();
        for fid in build.ordering_ins() {
            // TODO: what about phony inputs?
            let input = match self.derived_files.get(fid) {
                Some(df) => df.to_owned(),
                None => {
                    let file = &files.by_id[*fid];
                    if file.name.starts_with(&store_dir) {
                        // TODO: Perhaps need to add this as inputSrc? But
                        // will also have to change DerivedFile to have source
                        // Option<PathBuf>, because we don't want it to be
                        // added to $NIX_NINJA_INPUTS.
                        // DerivedFile {
                        //     path: SingleDerivedPath::Opaque(StorePath::new(file.name)),
                        //     source: &file.name,
                        // }
                        continue;
                    }

                    let input = new_opaque_file(
                        &self.tools.nix_tool,
                        &self.config.build_dir,
                        file.name.clone().into(),
                    )?;
                    self.add_derived_file(files, input.clone().to_owned());
                    input.to_owned()
                }
            };
            input_set.insert(input.build_path.clone(), input.clone());
        }

        let Some(primary_fid) = build.outs().iter().next() else {
            return Err(anyhow!("Build has no outputs"));
        };
        let primary_file = &files.by_id[*primary_fid];
        let name = normalize_output(&primary_file.name);

        let mut outputs: Vec<PathBuf> = Vec::new();
        for fid in build.outs() {
            let file = &files.by_id[*fid];
            outputs.push(PathBuf::from(&file.name));
        }

        // TODO: Can we avoid this? Technically the build rule isn't complete.
        //
        // The command may reference a file pre-generated by the configuration
        // step. We tracked files that existed in the build directory
        // beforehand, so we can see if there's anything that matches and add
        // it as an explicit input.
        if let Some(cmdline) = &build.cmdline {
            let args = shell_words::split(cmdline)?;
            for arg in args {
                let Some(fid) = files.lookup(&arg) else {
                    continue;
                };
                let input = match self.derived_files.get(&fid) {
                    Some(derived_file) => derived_file,
                    None => match self.build_dir_inputs.get(&fid) {
                        Some(derived_file) => derived_file,
                        None => {
                            continue;
                        }
                    },
                };
                input_set.insert(input.build_path.clone(), input.clone());
            }
        }

        // TODO: Can we avoid this? Technically the build rule isn't complete.
        //
        // Currently need this because there are rules that depend on
        // configuration phase generated files in Cpp Nix for example
        // `src/libutil/config-util.hh` which has a command like:
        // `-Isrc/libutil -include config-util.hh`.
        //
        // One way is to parse all the includes, then add it to our search
        // path above.
        for input in self.build_dir_inputs.values() {
            input_set.insert(input.build_path.clone(), input.clone());
        }

        let mut inputs: Vec<DerivedFile> = input_set.into_values().collect();
        inputs.sort();

        // Extract store paths from cmdline and add pre-extracted wrapper store paths
        let mut input_srcs = self.wrapper_store_paths.clone();
        if let Some(cmdline) = &build.cmdline {
            let found_store_paths = extract_store_paths(&self.store_regex, cmdline)?;
            input_srcs.extend(found_store_paths);
        }

        Ok(Task {
            name: format!("ninja-build-{name}"),
            system: self.config.system.clone(),
            wrapper_vars: self.wrapper_vars.clone(),
            input_srcs,
            build_dir: self.config.build_dir.clone(),
            build_deps: build.dependencies.clone(),
            store_dir: self.config.store_dir.clone(),
            cmdline: build.cmdline.clone(),
            desc: build.desc.clone(),
            deps: build.deps.clone(),
            files: build_files,
            inputs,
            outputs,
        })
    }
}

fn build_task_derivation(tools: Tools, task: Task) -> Result<Derivation> {
    let cmdline = match &task.cmdline {
        Some(c) => c,
        None => {
            return Err(anyhow!("Phony tasks not yet supported"));
        }
    };

    let mut drv = Derivation::new(
        &task.name,
        &task.system,
        &format!("{}/bin/nix-ninja-task", tools.nix_ninja_task),
    );
    drv.add_arg(cmdline);

    if let Some(desc) = &task.desc {
        drv.add_arg(&format!("--description={desc}"));
    }

    // Propagate wrapper environment variables to the task.
    for (key, value) in &task.wrapper_vars {
        let final_value = if key == "NIX_CFLAGS_COMPILE" {
            // Also add a deterministic random seed based on the task's
            // cmdline for reproducible builds.
            let deterministic_seed = generate_frandom_seed(cmdline);
            format!("{value} -frandom-seed={deterministic_seed}")
        } else {
            value.clone()
        };
        drv.set_env(key, &final_value);
    }

    // Add pre-extracted store paths from cmdline and wrapper vars
    for store_path in &task.input_srcs {
        drv.add_input_src(store_path);
    }

    // Needed by all tasks.
    drv.add_input_src(&tools.cc)
        .add_input_src(&tools.coreutils)
        .add_input_src(&tools.nix_ninja_task)
        .add_input_src(&tools.patchelf);

    // Add all ninja build inputs.
    let mut input_set: HashSet<String> = HashSet::new();
    for input in &task.inputs {
        // Declare input for derivation.
        drv.add_derived_path(&input.derived_path);

        // Encode input for nix-ninja-task.
        let encoded = &input.to_encoded();
        input_set.insert(encoded.clone());
    }

    // Handle when rule's dep = gcc, which means we need to find all the
    // implicit header dependencies normally handled by gcc's depfiles.
    let mut discovered_inputs: Vec<DerivedFile> = Vec::new();
    if let Some(deps) = &task.deps {
        if deps == "gcc" {
            // Only opaque inputs are processed by gcc
            let files: Vec<PathBuf> = task
                .inputs
                .iter()
                .filter_map(|input| match input.derived_path {
                    SingleDerivedPath::Opaque(_) => Some(input.build_path.clone()),
                    SingleDerivedPath::Built(_) => None, // Will be filled in by dynamic task derivation
                })
                .collect();

            let (discovered_deps, discovered_store_paths) = discover_c_includes(
                &tools.nix_tool,
                &task.store_dir,
                &task.build_dir,
                cmdline,
                files,
                None,
            )?;

            // Add discovered store paths as input sources only
            for store_path in discovered_store_paths {
                drv.add_input_src(&store_path);
            }

            // Add discovered deps to NIX_NINJA_INPUTS and derivation
            for derived_file in discovered_deps {
                let encoded = derived_file.to_encoded();

                // Skip if already in input_set
                if input_set.contains(&encoded) {
                    continue;
                }

                input_set.insert(encoded);
                drv.add_derived_path(&derived_file.derived_path);
                discovered_inputs.push(derived_file);
            }
        }
    }

    // Sort NIX_NINJA_INPUTS to ensure determinism.
    let mut inputs: Vec<String> = input_set.into_iter().collect();
    inputs.sort();

    drv.set_env("NIX_NINJA_INPUTS", &inputs.join(" "));

    // Add all ninja build outputs.
    let mut outputs: Vec<String> = Vec::new();
    for output_path in &task.outputs {
        // Declare a content addressed output.
        let normalized_name = normalize_output(&output_path.to_string_lossy());
        drv.add_ca_output(&normalized_name, HashAlgorithm::Sha256, OutputHashMode::Nar);

        // Create a placeholder and encode output for nix-ninja-task.
        let placeholder = Placeholder::standard_output(&normalized_name);
        let encoded = format!(
            "{}:{}:{}",
            &placeholder.render().display(),
            &output_path.display(),
            &output_path.display()
        );
        outputs.push(encoded);
    }
    drv.set_env("NIX_NINJA_OUTPUTS", &outputs.join(" "));

    {
        // Prepare $PATH to have coreutils.
        let mut path: Vec<String> = vec![
            format!("{}/bin", tools.cc),
            format!("{}/bin", tools.coreutils),
            format!("{}/bin", tools.patchelf),
        ];

        let cmdline_binary = cmdline
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("No command found in cmdline"))?;

        // TODO: If you don't find it it's ok, e.g. ./generated_binary
        let cmdline_path = which_store_path(cmdline_binary)?;

        drv.add_input_src(&cmdline_path);
        path.push(format!("{cmdline_path}/bin"));
        drv.set_env("PATH", &path.join(":"));
    }

    // For debugging purposes:
    // let json = &drv.to_json_pretty()?;
    // println!("Derivation:\n{json}");

    Ok(drv)
}

// For dynamic tasks, we generate an intermediary derivation that will then
// generate the final derivation with any discovered dependencies from its
// dependencies.
//
// For example, if a task derivation depends on generated.cc, we also want
// to depend on any headers generated.cc includes but we don't know that
// without the derivation that built generated.cc also scanned for includes
// and wrote that to its $deps output.
fn build_dynamic_task_derivation(
    tools: Tools,
    input_drv: Derivation,
    built_inputs: Vec<DerivedFile>,
) -> Result<Derivation> {
    let mut drv = Derivation::new(
        &format!("{}.drv", input_drv.name),
        &input_drv.system,
        &format!("{}/bin/nix-ninja", tools.nix_ninja),
    );
    drv.add_input_src(&tools.nix_ninja)
        .add_input_src(&tools.nix);

    // Add built inputs as dependencies so the dynamic task has access to them for scanning
    for built_input in &built_inputs {
        drv.add_derived_path(&built_input.derived_path);
    }

    // Encode built inputs for NIX_NINJA_INPUTS so dynamic task can process them
    let mut inputs: Vec<String> = built_inputs
        .iter()
        .map(|input| input.to_encoded())
        .collect();
    inputs.sort();
    drv.set_env("NIX_NINJA_INPUTS", &inputs.join(" "));

    drv.add_ca_output("out", HashAlgorithm::Sha256, OutputHashMode::Text);
    drv.set_env(
        "out",
        &Placeholder::standard_output("out")
            .render()
            .to_string_lossy(),
    );

    // Add the dynamic-task subtool argument
    drv.add_arg("-t").add_arg("dynamic-task");

    // Propagate sources to dynamic task for it discover inputs.
    let src = env::var("src").map_err(|_| anyhow!("Expected $src to be set"))?;
    drv.set_env("src", &src);
    let src_store_path = StorePath::new(src.clone())?;
    drv.add_input_src(&src_store_path);

    // Set up PATH to include nix binary
    let path = format!("{}/bin", tools.nix);
    drv.set_env("PATH", &path);

    // Requires extra experimental features to add our derivations.
    drv.set_env(
        "NIX_CONFIG",
        "extra-experimental-features = nix-command ca-derivations dynamic-derivations",
    );

    // Require recursive-nix to allow nix commands inside the build
    drv.set_env("requiredSystemFeatures", "recursive-nix");

    // Serialize the derivation to a temporary file and add to nix store
    let drv_json = input_drv.to_json()?;
    let temp_file = std::env::temp_dir().join(format!("drv-{}.json", input_drv.name));
    fs::write(&temp_file, &drv_json)?;
    let drv_json_path = tools.nix_tool.store_add(&temp_file)?;

    // Add derivation.json as input dependency and argument
    drv.add_input_src(&drv_json_path);
    drv.add_arg(&drv_json_path.to_string());

    Ok(drv)
}

/// Handles the result of build_task_derivation, deciding whether to wrap with
/// a dynamic task derivation or use the derivation directly.
fn handle_derivation_result(
    tools: Tools,
    task: Task,
    mut drv: Derivation,
    config: &RunnerConfig,
    nix_build_lock: Arc<Mutex<()>>,
) -> Result<SingleDerivedPath> {
    // Collect built inputs when deps == "gcc" for dynamic dependency discovery
    let built_inputs: Vec<DerivedFile> = if task.deps.as_ref() == Some(&"gcc".to_string()) {
        task.inputs
            .iter()
            .filter(|input| matches!(input.derived_path, SingleDerivedPath::Built(_)))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    if !built_inputs.is_empty() {
        // If we're in Nix sandbox, create a dynamic derivation to handle
        // dynamic dependencies.
        if config.is_output_derivation {
            let dynamic_drv = build_dynamic_task_derivation(tools.clone(), drv, built_inputs)?;
            let dynamic_drv_path = tools.nix_tool.derivation_add(&dynamic_drv)?;
            Ok(SingleDerivedPath::Built(SingleDerivedPathBuilt::new(
                dynamic_drv_path,
                "out".to_string(),
            )))
        } else {
            // Otherwise, symlink these built_inputs into build_dir and do
            // dependency discovery locally.

            let built_paths = {
                // Serialize nix build calls to prevent log output interleaving
                // when multiple tasks with dynamic dependencies run concurrently
                //
                // TODO: This isn't ideal, perhaps we buffer the logs or emit
                // JSON events for log aggregation.
                let _lock = nix_build_lock.lock().unwrap();
                local::build_derived_files(&tools.nix_tool, &built_inputs)?
            };

            let (discovered_deps, discovered_store_paths) =
                dynamic_task::discover_dynamic_dependencies(
                    &tools.nix_tool,
                    &config.store_dir,
                    &config.build_dir,
                    &drv,
                    built_paths,
                )?;

            dynamic_task::update_derivation_with_discoveries(
                &mut drv,
                discovered_deps,
                discovered_store_paths,
            )?;

            let drv_path = tools.nix_tool.derivation_add(&drv)?;
            Ok(SingleDerivedPath::Opaque(drv_path))
        }
    } else {
        let drv_path = tools.nix_tool.derivation_add(&drv)?;
        Ok(SingleDerivedPath::Opaque(drv_path))
    }
}

pub fn which_store_path(binary_name: &str) -> Result<StorePath> {
    let binary_path =
        which(binary_name).map_err(|err| anyhow!("Failed to find {}: {}", binary_name, err))?;

    // Canonicalize will resolve all symlinks and return an absolute path
    let canonical_path = std::fs::canonicalize(binary_path)?;

    let store_path = canonical_path
        .parent() // Get bin/ directory
        .and_then(|p| p.parent()) // Get the store path ($out)
        .ok_or_else(|| anyhow!("Cannot determine store path from binary: {}", binary_name))?;

    StorePath::new(store_path)
}

fn extract_store_paths(store_regex: &Regex, s: &str) -> Result<Vec<StorePath>> {
    let mut store_paths = Vec::new();
    for cap in store_regex.find_iter(s) {
        let store_path = StorePath::new(cap.as_str())?;
        if store_path.is_derivation() {
            continue;
        }
        if !store_path.path().exists() {
            continue;
        }
        store_paths.push(store_path);
    }
    Ok(store_paths)
}

fn new_opaque_file(
    nix: &NixTool,
    build_dir: &std::path::Path,
    path: PathBuf,
) -> Result<DerivedFile> {
    let relative_path = relative_from(&path, build_dir).unwrap_or(path);
    let mut path = relative_path.to_string_lossy().into_owned();
    canon::canonicalize_path(&mut path);

    let canonical_path = fs::canonicalize(&path)?;
    let store_path = nix.store_add(&canonical_path)?;
    Ok(DerivedFile {
        derived_path: SingleDerivedPath::Opaque(store_path.clone()),
        build_path: relative_path,
        rel_path: None, // None for opaque files - store path points directly to file
    })
}

fn new_built_file(derived_path: SingleDerivedPath, build_path: PathBuf) -> DerivedFile {
    let output_name = normalize_output(&build_path.to_string_lossy());
    let derived_built = SingleDerivedPathBuilt::from_derived_path(derived_path, output_name);
    DerivedFile {
        derived_path: SingleDerivedPath::Built(derived_built),
        build_path: build_path.clone(),
        rel_path: Some(build_path), // For built files, rel_path same as build_path
    }
}

// Derivation outputs cannot have `/` in them as its suffixed to the derivation
// store path.
fn normalize_output(output: &str) -> String {
    output.replace('/', "-")
}

/// Discovers C include dependencies from a command line and input files.
/// Returns (discovered_deps, discovered_store_paths) where:
/// - discovered_deps: DerivedFiles that need to be encoded and added to NIX_NINJA_INPUTS
/// - discovered_store_paths: Store paths that only need to be added as input sources
pub fn discover_c_includes(
    nix_tool: &NixTool,
    store_dir: &Path,
    build_dir: &Path,
    cmdline: &str,
    files: Vec<PathBuf>,
    virtual_paths: Option<HashMap<PathBuf, PathBuf>>,
) -> Result<(Vec<DerivedFile>, Vec<StorePath>)> {
    let c_includes = c_include_parser::retrieve_c_includes(cmdline, files.clone(), virtual_paths)?;
    let mut discovered_deps = Vec::new();
    let mut discovered_store_paths = Vec::new();

    // Convert input files to a set for filtering
    let input_files: HashSet<PathBuf> = files.into_iter().collect();

    for include in c_includes {
        // Skip input files - we only want to discover new dependencies
        if input_files.contains(&include) {
            continue;
        }

        // Check if include is from Nix store or a regular file
        if let Ok(relative) = include.strip_prefix(store_dir) {
            if let Some(hash_path) = relative.components().next().map(|c| c.as_os_str()) {
                let store_path = StorePath::new(store_dir.join(hash_path))?;
                discovered_store_paths.push(store_path);
                continue;
            }
        }

        // Regular file, add to nix store and treat as derived dependency
        let derived_file = new_opaque_file(nix_tool, build_dir, include)?;
        discovered_deps.push(derived_file);
    }

    Ok((discovered_deps, discovered_store_paths))
}

/// Removes -frandom-seed flag from a string of CFLAGS.
fn remove_frandom_seed(flags: &str) -> String {
    flags
        .split_whitespace()
        .filter(|flag| !flag.starts_with("-frandom-seed="))
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Generates -frandom-seed based on the task's cmdline.
fn generate_frandom_seed(cmdline: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(cmdline.as_bytes());
    let result = hasher.finalize();
    format!("{result:x}")[..16].to_string()
}
