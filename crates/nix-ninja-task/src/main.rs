use anyhow::{anyhow, Result};
use clap::command;
use clap::Parser;
use nix_ninja_task::derived_file::DerivedFile;
use nix_ninja_task::patchelf;
use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Parser)]
#[command(author, disable_version_flag = true)]
pub struct Cli {
    /// Specify the Nix store directory.
    #[arg(long = "store-dir", env = "NIX_STORE", default_value = "/nix/store")]
    pub store_dir: PathBuf,

    /// Directory prefix to recreate sources via symlinks.
    #[arg(long = "build-dir", default_value = "/build/source/build")]
    pub build_dir: PathBuf,

    /// Optional build target description.
    #[arg(long)]
    pub description: Option<String>,

    // Encoded derived files to prepare the source directory.
    #[arg(long, env = "NIX_NINJA_INPUTS")]
    pub inputs: String,

    // Encoded derived files that build outputs should be copied to.
    #[arg(long, env = "NIX_NINJA_OUTPUTS")]
    pub outputs: String,

    // Command to run.
    pub cmdline: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    fs::create_dir_all(&cli.build_dir)?;
    std::env::set_current_dir(&cli.build_dir)?;

    let mut inputs = Vec::new();
    for encoded in cli.inputs.split_whitespace() {
        // println!("Processing input {}", encoded);
        let input = DerivedFile::from_encoded(encoded)?;
        inputs.push(input);
    }

    let mut outputs = Vec::new();
    for encoded in cli.outputs.split_whitespace() {
        // println!("Processing output {}", encoded);
        let output = DerivedFile::from_encoded(encoded)?;
        outputs.push(output);
    }

    // The source directory of the derivation needs to have all build inputs
    // symlinked while preserving the original directory hierarchy of the
    // sources. This ensures relative includes and other path-dependent
    // references remain valid.
    create_symlinks(&cli.build_dir, inputs)?;
    println!(
        "nix-ninja-task: Setup source directory in {}",
        cli.build_dir.display()
    );

    create_parent_dirs(&outputs)?;

    if let Some(desc) = cli.description {
        println!("nix-ninja-task: {desc}");
    }

    // Spawn cmdline process via sh like ninja upstream does.
    println!("nix-ninja-task: Running: /bin/sh -c \"{}\"", cli.cmdline);
    let exit_code = spawn_process(cli.cmdline)?;
    if exit_code != 0 {
        println!("nix-ninja-task: Failed with exit code {exit_code}");
        std::process::exit(exit_code);
    }

    // Fix ELF RPATH to ensure it's linked against /nix/store paths rather
    // than relative path binaries in the build dir.
    patchelf::fix_rpaths(&cli.store_dir, &outputs)?;

    // Outputs must be created in build directory and then copied out because
    // ninja build rules can have implicit outputs that we have no way of
    // knowing. For example, a custom command that doesn't leverage the `$out`
    // implicit variable in the ninja evaluation context.
    println!(
        "nix-ninja-task: Finished! Copying {} build outputs to derivation output paths",
        outputs.len(),
    );
    for output in &outputs {
        let target_path = PathBuf::from(&output.to_string());
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&output.build_path, &target_path)?;
    }

    Ok(())
}

/// Creates symlinks for derived files under the specified prefix.
///
/// For each derived file, creates a symlink at `prefix/${derived_file.build_path}`
/// pointing to the actual file at `derived_file.path`.
fn create_symlinks(prefix: &std::path::Path, inputs: Vec<DerivedFile>) -> Result<()> {
    for input in inputs {
        let source_path = input.to_string();
        let dest_path = prefix.join(&input.build_path);

        // Create parent directories if they don't exist
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if !std::path::Path::new(&source_path).exists() {
            return Err(anyhow!(
                "nix-ninja-task: symlink source does not exist: {source_path}"
            ));
        }

        symlink(&source_path, &dest_path).map_err(|e| {
            anyhow!(
                "Failed to create symlink from {} to {}: {}",
                source_path,
                dest_path.display(),
                e
            )
        })?;
    }

    Ok(())
}

fn create_parent_dirs(outputs: &Vec<DerivedFile>) -> Result<()> {
    let mut dirs: Vec<&std::path::Path> = Vec::new();
    for output in outputs {
        if let Some(parent) = output.build_path.parent() {
            if dirs.contains(&parent) {
                continue;
            }
            std::fs::create_dir_all(parent)?;
            dirs.push(parent);
        }
    }
    Ok(())
}

fn spawn_process(cmdline: String) -> Result<i32> {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", &cmdline])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .envs(env::vars());

    let output = cmd.status()?;
    Ok(output.code().unwrap_or(1))
}
