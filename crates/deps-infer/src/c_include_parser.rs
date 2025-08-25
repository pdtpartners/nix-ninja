use crate::gcc_include_parser;
use anyhow::Result;
use include_graph::dependencies::cparse;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

pub fn retrieve_c_includes(cmdline: &str, files: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let includes = gcc_include_parser::parse_include_dirs(cmdline)?;
    bfs_parse_includes(files, &includes)
}

/// Recursively collect all dependencies using BFS
fn bfs_parse_includes(files: Vec<PathBuf>, include_dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    let mut queue = VecDeque::new();

    // Initialize queue with starting files
    for file in files {
        if visited.insert(file.clone()) {
            queue.push_back(file.clone());
            result.push(file);
        }
    }

    // Process queue in batches until empty
    while !queue.is_empty() {
        // Get all files currently in the queue
        let current_batch: Vec<PathBuf> = queue.drain(..).collect();

        // Process all files in the current batch in parallel
        let sources_with_includes = cparse::all_sources_and_includes(
            current_batch.into_iter().map(Ok::<_, std::io::Error>),
            include_dirs,
        )?;

        // Process each source's includes
        for source in sources_with_includes {
            for include in source.includes {
                if visited.insert(include.clone()) {
                    queue.push_back(include.clone());
                    result.push(include);
                }
            }
        }
    }

    Ok(result)
}
