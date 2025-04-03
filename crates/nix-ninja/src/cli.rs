use crate::build::{self, BuildConfig};
use anyhow::{anyhow, Result};
use clap::Parser;
use nix_libstore::store_path::StorePath;
use nix_ninja_task::derived_file::DerivedFile;
use nix_tool::{NixTool, StoreConfig};
use std::{env, fs, os::unix::fs::symlink, path::PathBuf, str};

#[derive(Parser)]
#[command(
    author,
    disable_version_flag = true,
    about = "nix-ninja: Incremental compilation of Ninja build files via Nix Dynamic Derivations"
)]
pub struct Cli {
    /// Change to DIR before doing anything else
    #[arg(short = 'C')]
    pub dir: Option<PathBuf>,

    /// Specify input build file [default=build.ninja]
    #[arg(short = 'f', default_value = "build.ninja")]
    pub build_filename: PathBuf,

    /// Run a subtool (use '-t list' to list subtools)
    #[arg(short = 't')]
    pub tool: Option<String>,

    /// Run N jobs in parallel (0 means infinity)
    #[arg(short = 'j', default_value = "0", hide = true)]
    pub jobs: usize,

    /// Do not start new jobs if the load average is greater than N
    #[arg(short = 'l', default_value = "0.0", hide = true)]
    pub load_average: f64,

    /// Show all command lines while building
    #[arg(short = 'v', long = "verbose", default_value = "false")]
    pub verbose: bool,

    /// Print ninja version
    #[arg(long = "version", default_value = "false")]
    pub print_version: bool,

    /// Specify the Nix store directory
    #[arg(long = "store-dir", default_value = "/nix/store", env = "NIX_STORE")]
    pub store_dir: PathBuf,

    /// Specify the Nix tool
    #[arg(long = "nix-tool", default_value = "nix", env = "NIX_TOOL")]
    pub nix_tool: String,

    #[arg(long, default_value = "false", env = "NIX_NINJA_DRV", hide = true)]
    pub is_output_derivation: bool,

    /// Until we dynamically create derivations that can infer C dependencies
    /// on derivation outputs, we have this hack to inject additional inputs
    /// that are inferred and source-linked into the nix-ninja-task
    /// environment.
    ///
    /// For example, Nix uses Bison to generate a parser-tab.cc from a
    /// .parser.y. The parser-tab.cc depends on finally.hh but we cannot
    /// determine it during nix-ninja build-time, only at nix-ninja-task
    /// build-time.
    #[arg(
        long = "extra-inputs",
        env = "NIX_NINJA_EXTRA_INPUTS",
        value_delimiter = ','
    )]
    pub extra_inputs: Vec<String>,

    /// Target to build (only used with certain subtools)
    #[arg(trailing_var_arg = true)]
    pub targets: Vec<String>,
}

pub fn run() -> Result<i32> {
    let cli = Cli::parse();

    if cli.print_version {
        // For compatibility with meson, it expects >= 1.8.2.
        println!("1.8.2");
        return Ok(0);
    }

    // Change directory if specified
    if let Some(dir) = &cli.dir {
        std::env::set_current_dir(dir)?;
    }

    // Handle subtool if specified
    if let Some(tool) = cli.tool.clone() {
        return subtool(&cli, &tool);
    }

    match build(&cli) {
        Ok(derived_file) => {
            if cli.is_output_derivation {
                let out = env::var("out").map_err(|_| anyhow!("Expected $out to be set"))?;
                fs::copy(&derived_file.path.store_path().path(), out)?;
            } else {
                nix_build(&cli, &derived_file)?;
            }
            Ok(0)
        }
        Err(err) => {
            println!("nix-ninja: {}", err);
            Ok(1)
        }
    }
}

fn build(cli: &Cli) -> Result<DerivedFile> {
    let build_dir = std::env::current_dir()?;
    let config = BuildConfig {
        build_dir,
        store_dir: cli.store_dir.clone(),
        nix_tool: cli.nix_tool.clone(),
        extra_inputs: cli.extra_inputs.clone(),
    };

    build::build(
        &cli.build_filename.to_string_lossy(),
        cli.targets.clone(),
        config,
    )
}

fn nix_build(cli: &Cli, derived_file: &DerivedFile) -> Result<()> {
    let nix = NixTool::new(StoreConfig {
        nix_tool: cli.nix_tool.clone(),
        extra_args: Vec::new(),
    });

    let output = nix.build(&derived_file.path)?;
    let stdout = str::from_utf8(&output.stdout)?;
    let drv_output = StorePath::new(stdout.trim())?;

    if derived_file.source.exists() {
        fs::remove_file(&derived_file.source)?;
    }
    symlink(&drv_output.path(), &derived_file.source)?;

    Ok(())
}

fn subtool(cli: &Cli, tool: &str) -> Result<i32> {
    match tool {
        "list" => {
            println!("nix-ninja subtools:");
            println!("  drv     show Nix derivation generated for a target");
        }
        "drv" => {
            let nix = NixTool::new(StoreConfig {
                nix_tool: cli.nix_tool.clone(),
                extra_args: Vec::new(),
            });

            let derived_file = build(cli)?;
            let output = nix.derivation_show(&derived_file.path.store_path())?;
            let stdout = str::from_utf8(&output.stdout)?;
            println!("{}", stdout);
        }
        // Meson compatibility tools.
        "restat" | "clean" | "cleandead" | "compdb" => {
            // TODO: Implement what's necessary, I think only compdb needs to
            // work and the rest can no-op.
        }
        _ => {
            println!(
                "Unknown subtool '{}'. Use '-t list' to get a list of available subtools.",
                tool
            );
            return Ok(1);
        }
    }
    Ok(0)
}
