use crate::relative_from::relative_from;
use anyhow::{anyhow, Error, Result};
use deps_infer::c_include_parser;
use n2::{
    canon,
    graph::{self, Build, BuildDependencies, BuildId, File, FileId},
};
use nix_libstore::prelude::*;
use nix_ninja_task::derived_file::{DerivedFile, DerivedOutput};
use nix_tool::NixTool;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    ops::Deref,
    path::PathBuf,
    sync::mpsc,
};
use walkdir::WalkDir;
use which::which;

#[derive(Clone)]
pub struct Tools {
    pub nix: NixTool,
    pub coreutils: StorePath,
    pub nix_ninja_task: StorePath,
}

/// Task represents a fully evaluated Ninja build target.
///
/// A task contains all the context to generate a Nix derivation for the build
/// target.
struct Task {
    name: String,
    system: String,
    env_vars: HashMap<String, String>,

    build_dir: PathBuf,
    build_deps: BuildDependencies,
    store_dir: PathBuf,
    store_regex: Regex,

    cmdline: Option<String>,
    desc: Option<String>,
    deps: Option<String>,

    files: HashMap<FileId, File>,
    inputs: Vec<DerivedFile>,
    outputs: Vec<DerivedOutput>,
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
    pub derived_files: Vec<DerivedFile>,
    pub err: Option<Error>,
}

pub struct RunnerConfig {
    pub system: String,
    pub build_dir: PathBuf,
    pub store_dir: PathBuf,
}

/// Runner is an async runtime that spawns threads for each task.
pub struct Runner {
    pub derived_files: HashMap<FileId, DerivedFile>,
    build_dir_inputs: HashMap<FileId, DerivedFile>,
    extra_inputs: HashMap<BuildId, Vec<DerivedFile>>,

    tx: mpsc::Sender<BuildResult>,
    rx: mpsc::Receiver<BuildResult>,
    tools: Tools,
    config: RunnerConfig,
    env_vars: HashMap<String, String>,
    store_regex: Regex,
}

impl Runner {
    pub fn new(tools: Tools, config: RunnerConfig) -> Result<Self> {
        let store_dir_str = config.store_dir.to_string_lossy();
        let pattern = format!(
            r"{}\/[a-z0-9]{{32}}-[0-9a-zA-Z\+\-\._\?=]+",
            regex::escape(&store_dir_str)
        );
        let store_regex = Regex::new(&pattern)?;

        let mut env_vars = HashMap::new();
        for (key, value) in env::vars() {
            env_vars.insert(key, value);
        }

        let (tx, rx) = mpsc::channel();
        Ok(Runner {
            derived_files: HashMap::new(),
            build_dir_inputs: HashMap::new(),
            extra_inputs: HashMap::new(),
            tx,
            rx,
            tools,
            config,
            env_vars,
            store_regex,
        })
    }

    // Build systems like Meson may generate files via `configure_file that are
    // not listed as implicit inputs in the build.ninja file. So we must read
    // the build directory and consider them implict inputs for all tasks.
    pub fn read_build_dir(&mut self, files: &mut graph::GraphFiles) -> Result<()> {
        for entry in WalkDir::new(&self.config.build_dir) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.into_path();
            let relative_path = match relative_from(&path, &self.config.build_dir) {
                Some(p) => p,
                None => path,
            };

            let derived_file = new_opaque_file(&self.tools.nix, relative_path.clone())?;
            let fid = self.add_derived_file(files, derived_file.clone(), &relative_path);
            self.build_dir_inputs.insert(fid, derived_file);
        }
        Ok(())
    }

    pub fn add_extra_inputs(
        &mut self,
        files: &mut graph::GraphFiles,
        encoded_inputs: Vec<String>,
    ) -> Result<()> {
        for encoded in encoded_inputs {
            // Split by colon to separate path from source
            let parts: Vec<&str> = encoded.split(':').collect();
            if parts.len() != 2 {
                return Err(anyhow!(
                    "Expected one ':' in encoded input but got {}",
                    encoded
                ));
            }
            let (target, extra_input_path) = (parts[0], PathBuf::from(parts[1]));

            let Some(fid) = files.lookup(target) else {
                return Err(anyhow!("Could not find target in extra input: {}", target));
            };

            let file = &files.by_id[fid];
            let Some(bid) = file.input else {
                return Err(anyhow!(
                    "Target in extra input is not an output of a build: {}",
                    target
                ));
            };

            let mut extra_inputs = match self.extra_inputs.get(&bid) {
                Some(extra_inputs) => extra_inputs.to_owned(),
                None => Vec::new(),
            };

            let derived_file = new_opaque_file(&self.tools.nix, extra_input_path.clone())?;
            self.add_derived_file(files, derived_file.clone(), &extra_input_path);

            extra_inputs.push(derived_file);
            self.extra_inputs.insert(bid, extra_inputs);
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
        let task = self.new_task(files, bid, build)?;

        std::thread::spawn(move || {
            let (derived_files, err) = match build_task_derivation(tools, task) {
                Ok(derived_files) => (derived_files, None),
                Err(err) => (Vec::new(), Some(err)),
            };

            let result = BuildResult {
                bid,
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
            eprintln!("Error: {}", err);

            eprintln!("Caused by:");
            for cause in err.chain().skip(1) {
                eprintln!("    {}", cause);
            }

            eprintln!("Backtrace: {}", err.backtrace());
            return Err(anyhow!(
                "Failed to build task derivation for {:?}: {}",
                result.bid,
                err
            ));
        }

        for derived_file in result.derived_files {
            self.add_derived_file(files, derived_file.clone(), &derived_file.source);
        }

        Ok(result.bid)
    }

    fn add_derived_file(
        &mut self,
        files: &mut graph::GraphFiles,
        derived_file: DerivedFile,
        path: &PathBuf,
    ) -> FileId {
        let mut path_str = path.to_string_lossy().into_owned();
        canon::canonicalize_path(&mut path_str);

        let fid = match files.lookup(&path_str) {
            Some(fid) => fid,
            None => files.id_from_canonical(path_str),
        };

        if let None = self.derived_files.get(&fid) {
            self.derived_files.insert(fid, derived_file);
        }

        fid
    }

    fn new_task(
        &mut self,
        files: &mut graph::GraphFiles,
        bid: BuildId,
        build: &Build,
    ) -> Result<Task> {
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

                    let input = new_opaque_file(&self.tools.nix, file.name.clone().into())?;
                    self.add_derived_file(
                        files,
                        input.clone().to_owned(),
                        &file.name.clone().into(),
                    );
                    input.to_owned()
                }
            };
            input_set.insert(input.source.clone(), input.clone());
        }

        let Some(primary_fid) = build.outs().iter().next() else {
            return Err(anyhow!("Build has no outputs"));
        };
        let primary_file = &files.by_id[*primary_fid];
        let name = normalize_output(&primary_file.name);

        let mut outputs: Vec<DerivedOutput> = Vec::new();
        for fid in build.outs() {
            let file = &files.by_id[*fid];
            let normalized_name = normalize_output(&file.name);
            let placeholder = Placeholder::standard_output(&normalized_name);
            let output = DerivedOutput {
                placeholder,
                source: PathBuf::from(&file.name),
            };
            outputs.push(output);
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
                input_set.insert(input.source.clone(), input.clone());
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
        for (_, input) in &self.build_dir_inputs {
            input_set.insert(input.source.clone(), input.clone());
        }

        if let Some(extra_inputs) = self.extra_inputs.get(&bid) {
            for input in extra_inputs {
                input_set.insert(input.source.clone(), input.clone());
            }
        }

        let mut inputs: Vec<DerivedFile> = input_set.into_values().collect();
        inputs.sort();

        Ok(Task {
            name: format!("ninja-build-{}", name),
            system: self.config.system.clone(),
            env_vars: self.env_vars.clone(),
            build_dir: self.config.build_dir.clone(),
            build_deps: build.dependencies.clone(),
            store_dir: self.config.store_dir.clone(),
            store_regex: self.store_regex.clone(),
            cmdline: build.cmdline.clone(),
            desc: build.desc.clone(),
            deps: build.deps.clone(),
            files: build_files,
            inputs,
            outputs,
        })
    }
}

fn build_task_derivation(tools: Tools, task: Task) -> Result<Vec<DerivedFile>> {
    let cmdline = match &task.cmdline {
        Some(c) => c,
        None => {
            return process_phony(tools, task);
        }
    };

    let mut drv = Derivation::new(
        &task.name,
        &task.system,
        &format!("{}/bin/nix-ninja-task", tools.nix_ninja_task.to_string()),
    );
    drv.add_arg(&cmdline);

    if let Some(desc) = &task.desc {
        drv.add_arg(&format!("--description={}", &desc));
    }

    // Propagate env var from build environment to the task.
    for (key, value) in &task.env_vars {
        // TODO: Currently necessary because we're using a gcc wrapped by
        // nixpkgs that has implicit deps inside env vars like NIX_LDFLAGS,
        // NIX_CFLAGS_COMPILE. Is there a better way?
        if !vec!["NIX_LDFLAGS".to_string(), "NIX_CFLAGS_COMPILE".to_string()].contains(key)
            && !key.starts_with("NIX_CC_WRAPPER")
        {
            continue;
        }

        drv.add_env(key, value);
        let found_store_paths = extract_store_paths(&task.store_regex, &value)?;
        for store_path in found_store_paths {
            drv.add_input_src(&store_path.to_string());
        }
    }

    // Needed by all tasks.
    drv.add_input_src(&tools.coreutils.to_string())
        .add_input_src(&tools.nix_ninja_task.to_string());

    // Add all ninja build inputs.
    let mut inputs: Vec<String> = Vec::new();
    for input in &task.inputs {
        // Declare input for derivation.
        add_derived_path(&mut drv, input);

        // Encode input for nix-ninja-task.
        let encoded = &input.to_encoded();
        inputs.push(encoded.clone());
    }

    // Handle when rule's dep = gcc, which means we need to find all the
    // implicit header dependencies normally handled by gcc's depfiles.
    let mut discovered_inputs: Vec<DerivedFile> = Vec::new();
    if let Some(deps) = &task.deps {
        if deps == "gcc" {
            let mut file_set: HashSet<PathBuf> = HashSet::new();
            // Only explict inputs are processed by gcc.
            for input in &task.inputs {
                let source = match input.path {
                    SingleDerivedPath::Opaque(_) => input.source.clone(),
                    SingleDerivedPath::Built(_) => {
                        continue;
                    }
                };
                file_set.insert(source);
            }

            let files: Vec<PathBuf> = file_set.clone().into_iter().collect();
            let c_includes = c_include_parser::retrieve_c_includes(&cmdline, files)?;

            for include in c_includes {
                if let Ok(relative) = include.strip_prefix(&task.store_dir) {
                    if let Some(hash_path) = relative.components().next().map(|c| c.as_os_str()) {
                        let store_path = task.store_dir.join(hash_path);
                        drv.add_input_src(&store_path.to_string_lossy());
                        continue;
                    }
                }

                // Make it relative to the build directory.
                let relative_include = match relative_from(&include, &task.build_dir) {
                    Some(p) => p,
                    None => include,
                };
                let mut path = relative_include.to_string_lossy().into_owned();
                canon::canonicalize_path(&mut path);

                // Skip paths that are already in the task inputs.
                if file_set.contains(&PathBuf::from(path.clone())) {
                    continue;
                }

                let derived_file = new_opaque_file(&tools.nix, path.into())?;
                let encoded = &derived_file.to_encoded();
                // Should be source-linked.
                inputs.push(encoded.clone());
                // Should be included as an input to derivation.
                add_derived_path(&mut drv, &derived_file);
                // Should be returned back to the Runner as a discovered input.
                discovered_inputs.push(derived_file);
            }
        }
    }
    drv.add_env("NIX_NINJA_INPUTS", &inputs.join(" "));

    // Add all ninja build outputs.
    let mut outputs: Vec<String> = Vec::new();
    for output in &task.outputs {
        // Declare a content addressed output.
        let normalized_name = normalize_output(&output.source.to_string_lossy());
        drv.add_ca_output(&normalized_name, HashAlgorithm::Sha256, OutputHashMode::Nar);

        // Encode output for nix-ninja-task.
        let encoded = &output.to_encoded();
        outputs.push(encoded.clone());
    }
    drv.add_env("NIX_NINJA_OUTPUTS", &outputs.join(" "));

    {
        // Prepare $PATH to have coreutils.
        let mut path: Vec<String> = vec![format!("{}/bin", tools.coreutils.to_string())];

        let cmdline_binary = cmdline
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("No command found in cmdline"))?;

        // TODO: If you don't find it it's ok, e.g. ./generated_binary
        let cmdline_path = which_store_path(&cmdline_binary)?;

        drv.add_input_src(&cmdline_path.to_string());
        path.push(format!("{}/bin", cmdline_path.to_string()));
        drv.add_env("PATH", &path.join(":"));
    }

    // The cmdline may refer to hardcoded store paths as they were found
    // by the build.ninja generator (e.g. meson). We need to extract them
    // and add as inputSrcs.
    let found_store_paths = extract_store_paths(&task.store_regex, &cmdline)?;
    for store_path in found_store_paths {
        drv.add_input_src(&store_path.to_string());
    }

    // let json = &drv.to_json_pretty()?;
    // println!("Derivation:\n{}", json);

    // Add the derivation to the Nix store.
    let drv_path = tools.nix.derivation_add(&drv)?;

    // Collect all the built outputs of the derivation so it can be referenced
    // as inputs by dependent builds.
    let mut drv_outputs: Vec<DerivedFile> = Vec::new();
    for fid in task.outs() {
        let file = &task.files[fid];
        let built_file = new_built_file(&drv_path, file.name.clone().into());
        drv_outputs.push(built_file);
    }

    // Return both discovered inputs & derivation outputs.
    discovered_inputs.extend(drv_outputs);
    Ok(discovered_inputs)
}

fn process_phony(_: Tools, _: Task) -> Result<Vec<DerivedFile>> {
    Err(anyhow!("Unimplemented"))
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

fn new_opaque_file(nix: &NixTool, path: PathBuf) -> Result<DerivedFile> {
    let canonical_path = fs::canonicalize(&path)?;
    let store_path = nix.store_add(&canonical_path)?;
    Ok(DerivedFile {
        path: SingleDerivedPath::Opaque(store_path.clone()),
        source: path,
    })
}

fn new_built_file(drv_path: &StorePath, path: PathBuf) -> DerivedFile {
    let derived_built = SingleDerivedPathBuilt {
        drv_path: drv_path.clone(),
        output: normalize_output(&path.to_string_lossy()),
    };
    DerivedFile {
        path: SingleDerivedPath::Built(derived_built),
        source: path,
    }
}

fn add_derived_path(drv: &mut Derivation, derived_file: &DerivedFile) {
    match &derived_file.path {
        SingleDerivedPath::Opaque(store_path) => {
            drv.add_input_src(&store_path.to_string());
        }
        SingleDerivedPath::Built(derived_built) => {
            drv.add_input_drv(
                &derived_built.drv_path.to_string(),
                vec![derived_built.output.clone()],
            );
        }
    }
}

// Derivation outputs cannot have `/` in them as its suffixed to the derivation
// store path.
fn normalize_output(output: &str) -> String {
    output.replace('/', "-")
}
