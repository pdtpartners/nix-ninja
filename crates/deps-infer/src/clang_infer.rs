use anyhow::{anyhow, Result};
use clang_sys::*;
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uint, c_void};
use std::path::PathBuf;
use std::ptr;

struct VisitorData<'a> {
    includes: &'a mut HashSet<PathBuf>,
    tu: CXTranslationUnit,
}

pub fn retrieve_c_includes(cmdline: &str, files: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut all_includes = HashSet::new();

    // Build compiler arguments from cmdline once
    let args = build_clang_args(cmdline)?;

    // Initialize clang index with diagnostics suppressed and reuse it
    let index = unsafe { clang_createIndex(0, 0) };
    if index.is_null() {
        return Err(anyhow!("Failed to create clang index"));
    }

    // Try to parse all files in a single translation unit for better performance
    if files.len() == 1 {
        // Single file - use the existing approach
        match parse_file_includes(index, &files[0], &args) {
            Ok(file_includes) => all_includes.extend(file_includes),
            Err(_) => {
                // Skip files that can't be parsed
            }
        }
    } else {
        // Multiple files - could potentially batch them, but for now keep separate
        // The real win would be to reuse parsed headers, but that's complex with current API
        for file in files {
            match parse_file_includes(index, &file, &args) {
                Ok(file_includes) => all_includes.extend(file_includes),
                Err(_) => {
                    // Skip files that can't be parsed (e.g., missing generated headers)
                    continue;
                }
            }
        }
    }

    // Cleanup
    unsafe {
        clang_disposeIndex(index);
    }

    let mut result: Vec<PathBuf> = all_includes.into_iter().collect();
    result.sort();
    Ok(result)
}

fn build_clang_args(cmdline: &str) -> Result<Vec<CString>> {
    // Split the command line using POSIX shell semantics
    let mut cmdline_args =
        shell_words::split(cmdline).map_err(|e| anyhow!("Invalid command line syntax: {}", e))?;

    // Skip the compiler executable (first argument)
    if !cmdline_args.is_empty() {
        cmdline_args.remove(0);
    }

    // Filter out compilation-specific flags that libclang doesn't need
    let mut filtered_args = Vec::new();
    let mut i = 0;
    while i < cmdline_args.len() {
        let arg = &cmdline_args[i];

        // Skip source files (they'll be passed separately to clang_parseTranslationUnit)
        if arg.ends_with(".cpp")
            || arg.ends_with(".c")
            || arg.ends_with(".cc")
            || arg.ends_with(".cxx")
        {
            i += 1;
            continue;
        }

        // Keep all other arguments (include paths, defines, warnings, etc.)
        filtered_args.push(arg.clone());
        i += 1;
    }

    // Add flags to suppress system header diagnostics and use dependency mode
    filtered_args.push("-w".to_string()); // Suppress all warnings
    filtered_args.push("-Wno-error".to_string()); // Don't treat warnings as errors
    filtered_args.push("-fsyntax-only".to_string()); // Skip code generation

    // Convert to CStrings
    filtered_args
        .into_iter()
        .map(|arg| CString::new(arg).map_err(|e| anyhow!("Invalid argument: {}", e)))
        .collect()
}

fn parse_file_includes(
    index: CXIndex,
    file_path: &PathBuf,
    args: &[CString],
) -> Result<HashSet<PathBuf>> {
    let mut includes = HashSet::new();

    let file_str = file_path.to_str().unwrap();
    let file_cstring = CString::new(file_str)?;

    // Convert args to pointers
    let arg_ptrs: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).collect();

    // Minimal parsing - just enough to get include information
    let tu = unsafe {
        clang_parseTranslationUnit(
            index,
            file_cstring.as_ptr(),
            arg_ptrs.as_ptr(),
            arg_ptrs.len() as i32,
            ptr::null_mut(),
            0,
            CXTranslationUnit_None,  // Try with no special flags first
        )
    };

    if tu.is_null() {
        return Err(anyhow!("Failed to parse translation unit for {}", file_str));
    }

    // Collect all inclusions using clang_getInclusions
    let mut visitor_data = VisitorData {
        includes: &mut includes,
        tu,
    };
    unsafe {
        clang_getInclusions(
            tu,
            inclusion_visitor,
            &mut visitor_data as *mut VisitorData as *mut c_void,
        );
    }

    // Cleanup
    unsafe {
        clang_disposeTranslationUnit(tu);
    }

    Ok(includes)
}

extern "C" fn inclusion_visitor(
    file: CXFile,
    _inclusion_stack: *mut CXSourceLocation,
    _include_len: c_uint,
    client_data: CXClientData,
) {
    if file.is_null() {
        return;
    }

    let visitor_data = unsafe { &mut *(client_data as *mut VisitorData) };

    unsafe {
        // Check if this is a system header using clang's built-in functionality
        let location = clang_getLocationForOffset(visitor_data.tu, file, 0);
        if clang_Location_isInSystemHeader(location) != 0 {
            return; // Skip system headers
        }

        let file_name = clang_getFileName(file);
        let file_name_ptr = clang_getCString(file_name);
        if !file_name_ptr.is_null() {
            if let Ok(file_path_str) = CStr::from_ptr(file_name_ptr).to_str() {
                visitor_data.includes.insert(PathBuf::from(file_path_str));
            }
        }
        clang_disposeString(file_name);
    }
}
