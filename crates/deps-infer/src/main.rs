use anyhow::{anyhow, bail, Result};
use clap::Parser;
use deps_infer::{c_include_parser, clang_infer, gcc_depfile};
use n2::{canon, load, scanner};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use tracing_subscriber::EnvFilter;

/// A tool to extract C/C++ include dependencies
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Change to DIR before doing anything else
    #[arg(short = 'C')]
    pub dir: Option<PathBuf>,

    /// Specify input build file [default=build.ninja]
    #[arg(short = 'f', default_value = "build.ninja")]
    pub build_filename: PathBuf,

    /// Mode of operation
    #[arg(long, default_value = "correctness")]
    pub mode: Mode,

    #[arg(long = "target")]
    pub target: Option<String>,
}

#[derive(Parser, Debug, Clone, clap::ValueEnum)]
enum Mode {
    /// Print out the includes found recursively for a given target.
    Scan,
    /// Compare c_includes with gcc_includes for correctness
    Correctness,
    /// Benchmark the performance of include extraction
    Benchmark,
}

pub struct Target {
    filename: String,
    cmdline: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Parse command line arguments
    let args = Args::parse();

    if let Some(dir) = args.dir {
        std::env::set_current_dir(dir)?;
    }

    let build_filename = args
        .build_filename
        .to_str()
        .ok_or_else(|| anyhow!("Invalid path"))?;

    let targets = load_targets(build_filename)?;

    match args.mode {
        Mode::Scan => {
            match args.target {
                Some(target_name) => {
                    for target in targets {
                        if target.filename == target_name {
                            return run_scan_mode(target);
                        }
                    }
                    Err(anyhow!("Failed to find target: {}", target_name))
                }
                None => {
                    // Process all targets when no specific target is given
                    for target in targets {
                        let filename = target.filename.clone();
                        println!("=== Scanning target: {} ===", filename);
                        if let Err(e) = run_scan_mode(target) {
                            eprintln!("Error scanning {}: {}", filename, e);
                        }
                        println!();
                    }
                    Ok(())
                }
            }
        }
        Mode::Benchmark => run_benchmark_mode(targets),
        Mode::Correctness => run_correctness_mode(targets),
    }
}

fn load_targets(build_filename: &str) -> Result<Vec<Target>> {
    let mut loader = load::Loader::new();

    let id = loader
        .graph
        .files
        .id_from_canonical(canon::to_owned_canon_path(build_filename));

    let path = loader.graph.file(id).path().to_path_buf();
    let bytes = match scanner::read_file_with_nul(&path) {
        Ok(b) => b,
        Err(e) => bail!("read {}: {}", path.display(), e),
    };

    loader.parse(path, &bytes)?;

    let mut targets: Vec<Target> = Vec::new();
    for fid in loader.graph.files.by_id.all_ids() {
        let file = &loader.graph.files.by_id[fid];

        let bid = match file.input {
            Some(bid) => bid,
            None => continue,
        };

        let build = &loader.graph.builds[bid];
        let cmdline = match &build.cmdline {
            Some(s) => s,
            None => {
                // phony
                continue;
            }
        };

        let primary_fid = match build.explicit_ins().iter().next() {
            Some(fid) => fid,
            None => {
                // input nothing?
                continue;
            }
        };

        let primary_file = &loader.graph.files.by_id[*primary_fid];

        let path = Path::new(&file.name);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext.as_str() == "o" {
            targets.push(Target {
                filename: primary_file.name.to_string(),
                cmdline: cmdline.to_string(),
            });
        }
    }

    Ok(targets)
}

fn run_scan_mode(target: Target) -> Result<()> {
    let c_include_includes = c_include_parser::retrieve_c_includes(&target.cmdline, vec![target.filename.clone().into()])?;
    println!("c_include_parser method:");
    for include in c_include_includes {
        println!("{}", include.display());
    }

    let clang_includes =
        clang_infer::retrieve_c_includes(&target.cmdline, vec![target.filename.clone().into()])?;
    println!("clang_infer method:");
    for include in clang_includes {
        println!("{}", include.display());
    }

    Ok(())
}

fn run_benchmark_mode(targets: Vec<Target>) -> Result<()> {
    println!("Benchmarking {} targets...", targets.len());
    
    // Benchmark c_include_parser method
    let c_include_start = Instant::now();
    for target in &targets {
        let _ = c_include_parser::retrieve_c_includes(&target.cmdline, vec![target.filename.clone().into()]);
    }
    let c_include_duration = c_include_start.elapsed();
    println!(
        "c_include_parser method: {} milliseconds",
        c_include_duration.as_millis()
    );

    // Benchmark clang_infer method
    let clang_start = Instant::now();
    for target in &targets {
        let _ = clang_infer::retrieve_c_includes(&target.cmdline, vec![target.filename.clone().into()]);
    }
    let clang_duration = clang_start.elapsed();
    println!(
        "clang_infer method: {} milliseconds",
        clang_duration.as_millis()
    );

    // Calculate and display performance comparison
    let c_include_ms = c_include_duration.as_millis() as f64;
    let clang_ms = clang_duration.as_millis() as f64;

    if c_include_ms > 0.0 && clang_ms > 0.0 {
        let ratio = c_include_ms / clang_ms;
        if ratio > 1.0 {
            println!("clang_infer is {:.2}x faster than c_include_parser", ratio);
        } else {
            println!("c_include_parser is {:.2}x faster than clang_infer", 1.0 / ratio);
        }
        
        println!("Performance ratio (c_include_parser / clang_infer): {:.2}", ratio);
    }

    Ok(())
}

fn run_correctness_mode(targets: Vec<Target>) -> Result<()> {
    let current_dir = std::env::current_dir()?;
    for target in targets {
        let mut clang_includes = clang_infer::retrieve_c_includes(
            &target.cmdline,
            vec![target.filename.clone().into()],
        )?;
        clang_includes = normalize_paths(clang_includes, &current_dir);

        let mut gcc_includes = gcc_depfile::retrieve_c_includes(&target.cmdline)?;
        gcc_includes = normalize_paths(gcc_includes, &current_dir);

        println!(
            "{}: clang {}, gcc {}",
            target.filename,
            clang_includes.len(),
            gcc_includes.len()
        );

        // Find items in gcc_includes but not in clang_includes
        let gcc_only: Vec<_> = gcc_includes
            .iter()
            .filter(|path| !clang_includes.contains(path))
            .collect();

        if !gcc_only.is_empty() {
            println!("Mismatch for {}", target.filename);

            // Find items in clang_includes but not in gcc_includes
            let clang_only: Vec<_> = clang_includes
                .iter()
                .filter(|path| !gcc_includes.contains(path))
                .collect();

            if !clang_only.is_empty() {
                println!("Found in clang_includes but missing from gcc_includes:");
                for path in clang_only {
                    println!("  + {}", path.display());
                }
            }

            if !gcc_only.is_empty() {
                println!("Found in gcc_includes but missing from clang_includes:");
                for path in gcc_only {
                    println!("  - {}", path.display());
                }
            }

            return Err(anyhow!("Include mismatch for {}", target.filename));
        }
    }

    println!("clang_infer is fully correct for {}", current_dir.display());

    Ok(())
}

// Helper function to normalize and canonicalize paths
fn normalize_paths(paths: Vec<PathBuf>, current_dir: &Path) -> Vec<PathBuf> {
    paths
        .into_iter()
        .map(|path| {
            let path = if path.is_absolute() {
                path
            } else {
                current_dir.join(path)
            };
            // Normalize the path to remove components like ".." and "."
            match path.canonicalize() {
                Ok(canonical) => canonical,
                Err(_) => path, // Keep original if canonicalization fails
            }
        })
        .collect()
}
