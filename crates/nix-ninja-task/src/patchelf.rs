use crate::derived_file::DerivedFile;
use anyhow::{anyhow, Result};
use elf::endian::AnyEndian;
use elf::ElfBytes;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn fix_rpaths(store_dir: &Path, outputs: &[DerivedFile]) -> Result<()> {
    for output in outputs {
        let canonical_path = fs::canonicalize(&output.build_path)?;
        if is_elf_dynamic(&canonical_path)? {
            fix_rpath(store_dir, &canonical_path)?;
            println!(
                "nix-ninja-task: Fixed RPATH for {}",
                output.build_path.display()
            );
        }
    }

    Ok(())
}

// Check if this is an executable or shared library that can have RPATH.
// Skip object files (.o) or non-ELF files.
fn is_elf_dynamic(path: &Path) -> Result<bool> {
    let data = fs::read(path)?;

    let elf = match ElfBytes::<AnyEndian>::minimal_parse(&data) {
        Ok(elf) => elf,
        Err(_) => return Ok(false), // Not a valid ELF file
    };

    // Only process executables (ET_EXEC) and shared libraries (ET_DYN)
    // Skip object files (ET_REL) as they don't have RPATH
    match elf.ehdr.e_type {
        elf::abi::ET_EXEC | elf::abi::ET_DYN => Ok(true),
        _ => Ok(false),
    }
}

fn fix_rpath(store_dir: &Path, elf_path: &Path) -> Result<()> {
    if let Some(new_rpath) = compute_new_rpath(store_dir, elf_path)? {
        apply_rpath(elf_path, &new_rpath)?;
    }
    Ok(())
}

fn compute_new_rpath(store_dir: &Path, elf_path: &Path) -> Result<Option<Vec<String>>> {
    // Resolve RPATH entries with $ORIGIN expansion
    let current_rpath = get_rpath(elf_path)?;
    let resolved_rpath = resolve_rpath(&current_rpath, elf_path)?;

    // Get needed libraries and collect directories that need to be added to RPATH
    let mut new_rpath = Vec::new();
    let mut path_added = false;

    // Keep existing non-$ORIGIN paths
    for path in &current_rpath {
        if path.contains("$ORIGIN") {
            continue;
        }
        new_rpath.push(path.clone());
    }

    let needed_libs = get_needed_libs(elf_path)?;

    for lib_name in &needed_libs {
        let Some(lib_path) = resolve_needed(lib_name, &resolved_rpath, store_dir)? else {
            continue;
        };

        let Some(lib_dir) = lib_path.parent() else {
            continue;
        };

        let lib_str = lib_dir.to_string_lossy().to_string();
        if !new_rpath.contains(&lib_str) {
            new_rpath.push(lib_str);
            path_added = true;
        }
    }

    if !path_added {
        Ok(None)
    } else {
        Ok(Some(new_rpath))
    }
}

fn get_rpath(elf_path: &Path) -> Result<Vec<String>> {
    let output = Command::new("patchelf")
        .arg("--print-rpath")
        .arg(elf_path)
        .output()
        .map_err(|e| anyhow!("Failed to execute patchelf --print-rpath: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("patchelf --print-rpath failed: {stderr}"));
    }

    let rpath_str = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if rpath_str.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(rpath_str
            .split(':')
            .filter(|p| !p.is_empty())
            .map(|p| p.to_string())
            .collect())
    }
}

fn resolve_rpath(rpath: &[String], elf_path: &Path) -> Result<Vec<PathBuf>> {
    let mut resolved_paths = Vec::new();

    let origin = elf_path
        .parent()
        .ok_or_else(|| anyhow!("ELF path has no parent directory"))?
        .to_string_lossy();

    for entry in rpath {
        let expanded = entry.replace("$ORIGIN", &origin);
        resolved_paths.push(PathBuf::from(expanded));
    }

    Ok(resolved_paths)
}

fn get_needed_libs(elf_path: &Path) -> Result<Vec<String>> {
    let output = Command::new("patchelf")
        .arg("--print-needed")
        .arg(elf_path)
        .output()
        .map_err(|e| anyhow!("Failed to execute patchelf --print-needed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("patchelf --print-needed failed: {stderr}"));
    }

    let needed_str = String::from_utf8_lossy(&output.stdout);
    Ok(needed_str
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

fn resolve_needed(lib_name: &str, rpath: &[PathBuf], store_dir: &Path) -> Result<Option<PathBuf>> {
    // Search for the library in each rpath directory
    for search_dir in rpath {
        let lib_path = search_dir.join(lib_name);

        // If it's already in nix store, return None (don't add to rpath)
        if lib_path.starts_with(store_dir) {
            return Ok(None);
        }

        if lib_path.exists() {
            let canonical_path = fs::canonicalize(&lib_path)?;
            return Ok(Some(canonical_path));
        }
    }

    Err(anyhow!("Library {lib_name} not found in RPATH"))
}

fn apply_rpath(elf_path: &Path, new_paths: &[String]) -> Result<()> {
    let rpath_str = new_paths.join(":");
    let mut cmd = Command::new("patchelf");
    cmd.arg("--set-rpath").arg(&rpath_str).arg(elf_path);

    let output = cmd
        .output()
        .map_err(|e| anyhow!("Failed to execute patchelf: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("patchelf failed: {stderr}"));
    }

    Ok(())
}
