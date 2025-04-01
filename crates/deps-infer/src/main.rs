use anyhow::{anyhow, bail, Result};
use clap::Parser;
use deps_infer::{c_include_parser, gcc_depfile};
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
            let target_name = args.target.unwrap();

            for target in targets {
                if target.filename == target_name {
                    return run_scan_mode(target);
                }
            }
            Err(anyhow!("Failed to find target: {}", target_name))
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
        match ext.as_str() {
            "o" => {
                targets.push(Target {
                    filename: primary_file.name.to_string(),
                    cmdline: cmdline.to_string(),
                });
            }
            _ => {}
        }
    }

    Ok(targets)
}

fn run_scan_mode(target: Target) -> Result<()> {
    let gcc_includes = gcc_depfile::retrieve_c_includes(&target.cmdline)?;
    println!("GCC depfile method:");
    for include in gcc_includes {
        println!("{}", include.display());
    }

    // Benchmark c_include_parser method
    let c_includes = c_include_parser::retrieve_c_includes(
        &target.cmdline,
        vec![target.filename.clone().into()],
    )?;
    println!("C include parser method:");
    for include in c_includes {
        println!("{}", include.display());
    }

    Ok(())
}

fn run_benchmark_mode(targets: Vec<Target>) -> Result<()> {
    // Benchmark gcc_depfile method
    let gcc_start = Instant::now();
    for target in &targets {
        gcc_depfile::retrieve_c_includes(&target.cmdline)?;
    }
    let gcc_duration = gcc_start.elapsed();
    println!(
        "GCC depfile method: {} milliseconds",
        gcc_duration.as_millis()
    );

    // Benchmark c_include_parser method
    let c_start = Instant::now();
    for target in &targets {
        c_include_parser::retrieve_c_includes(
            &target.cmdline,
            vec![target.filename.clone().into()],
        )?;
    }
    let c_duration = c_start.elapsed();
    println!(
        "C include parser method: {} milliseconds",
        c_duration.as_millis()
    );

    // Calculate and display percentage difference
    let gcc_ms = gcc_duration.as_millis() as f64;
    let c_ms = c_duration.as_millis() as f64;

    if gcc_ms > 0.0 && c_ms > 0.0 {
        let percentage_diff = (gcc_ms / c_ms) * 100.0;
        if percentage_diff > 0.0 {
            println!(
                "C include parser is {:.2}% faster than GCC depfile method",
                percentage_diff
            );
        } else {
            println!(
                "C include parser is {:.2}% slower than GCC depfile method",
                percentage_diff
            );
        }
    }

    Ok(())
}

fn run_correctness_mode(targets: Vec<Target>) -> Result<()> {
    let current_dir = std::env::current_dir()?;
    for target in targets {
        let mut c_includes = c_include_parser::retrieve_c_includes(
            &target.cmdline,
            vec![target.filename.clone().into()],
        )?;
        c_includes = normalize_paths(c_includes, &current_dir);

        let mut gcc_includes = gcc_depfile::retrieve_c_includes(&target.cmdline)?;
        gcc_includes = normalize_paths(gcc_includes, &current_dir);

        println!(
            "{}: c {}, gcc {}",
            target.filename,
            c_includes.len(),
            gcc_includes.len()
        );

        // Find items in gcc_includes but not in c_includes
        let gcc_only: Vec<_> = gcc_includes
            .iter()
            .filter(|path| !c_includes.contains(path))
            .collect();

        if gcc_only.len() > 0 {
            println!("Mismatch for {}", target.filename);

            // Find items in c_includes but not in gcc_includes
            let c_only: Vec<_> = c_includes
                .iter()
                .filter(|path| !gcc_includes.contains(path))
                .collect();

            if !c_only.is_empty() {
                println!("Found in c_includes but missing from gcc_includes:");
                for path in c_only {
                    println!("  + {}", path.display());
                }
            }

            if !gcc_only.is_empty() {
                println!("Found in gcc_includes but missing from c_includes:");
                for path in gcc_only {
                    println!("  - {}", path.display());
                }
            }

            return Err(anyhow!("Include mismatch for {}", target.filename));
        }
    }

    println!(
        "c_include_parser is fully correct for {}",
        current_dir.display()
    );

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
