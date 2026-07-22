use crate::build::{self, BuildConfig};
use crate::local;
use crate::subtool::dynamic_task;
use anyhow::{anyhow, Context as _, Result};
use clap::Parser;
use harmonia_store_path::StoreDir;
use nix_ninja_task::derived_file::DerivedFile;
use nix_tool::{NixTool, StoreConfig};
use std::{
    env, fs,
    path::{Path, PathBuf},
    str,
};

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
    pub store_dir: StoreDir,

    /// Specify the Nix tool
    #[arg(long = "nix-tool", default_value = "nix", env = "NIX_TOOL")]
    pub nix_tool: String,

    #[arg(long, default_value = "false", env = "NIX_NINJA_DRV", hide = true)]
    pub is_output_derivation: bool,

    /// Target to build (only used with certain subtools)
    #[arg(trailing_var_arg = true)]
    pub targets: Vec<String>,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    if cli.print_version {
        // For compatibility with meson, it expects >= 1.8.2.
        println!("1.8.2");
        return Ok(());
    }

    // Change directory if specified
    if let Some(dir) = &cli.dir {
        std::env::set_current_dir(dir)?;
    }
    let build_dir = std::env::current_dir()?;

    let nix_tool = NixTool::new(StoreConfig {
        nix_tool: cli.nix_tool.clone(),
        extra_args: Vec::new(),
    });

    // Handle subtool if specified
    if let Some(tool) = cli.tool.clone() {
        return subtool(
            nix_tool,
            &build_dir,
            &cli.store_dir,
            &tool,
            cli.targets.clone(),
        );
    }

    let derived_file = build(&cli, &build_dir)?;
    if cli.is_output_derivation {
        let out = env::var("out").map_err(|_| anyhow!("Expected $out to be set"))?;
        fs::copy(
            derived_file
                .derived_path
                .root_path()
                .to_absolute_path(&cli.store_dir),
            out,
        )?;
    } else {
        local::symlink_derived_files(&nix_tool, &cli.store_dir, &build_dir, &[derived_file])?;
    }
    Ok(())
}

fn build(cli: &Cli, build_dir: &Path) -> Result<DerivedFile> {
    let config = BuildConfig {
        build_dir: build_dir.to_path_buf(),
        store_dir: cli.store_dir.clone(),
        nix_tool: cli.nix_tool.clone(),
        is_output_derivation: cli.is_output_derivation,
    };

    build::build(
        cli.build_filename
            .to_str()
            .context("Filename was not valid UTF-8")?,
        cli.targets.clone(),
        config,
    )
}

fn subtool(
    nix_tool: NixTool,
    build_dir: &Path,
    store_dir: &StoreDir,
    subtool_name: &str,
    targets: Vec<String>,
) -> Result<()> {
    match subtool_name {
        "list" => {
            println!("nix-ninja subtools:");
            println!("  drv           show Nix derivation generated for a target");
            println!("  dynamic-task  generate task derivation from task + discovered deps");
            Ok(())
        }
        "drv" => {
            let cli = Cli::parse();
            let derived_file = build(&cli, build_dir)?;
            let output =
                nix_tool.derivation_show(store_dir, derived_file.derived_path.root_path())?;
            let stdout = str::from_utf8(&output.stdout)?;
            println!("{stdout}");
            Ok(())
        }
        "dynamic-task" => dynamic_task::run(nix_tool, store_dir, targets),
        // Meson compatibility tools.
        "restat" | "clean" | "cleandead" | "compdb" => {
            // TODO: Implement what's necessary, I think only compdb needs to
            // work and the rest can no-op.
            Ok(())
        }
        _ => {
            anyhow::bail!(
                "Unknown subtool '{subtool_name}'. Use '-t list' to get a list of available subtools."
            );
        }
    }
}
