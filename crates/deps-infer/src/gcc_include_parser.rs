use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// Parse include directories from a gcc cmdline.
pub fn parse_include_dirs(cmdline: &str) -> Result<Vec<PathBuf>> {
    // Split the command line respecting quotes and escapes
    let args = match shell_words::split(cmdline) {
        Ok(args) => args,
        Err(e) => return Err(anyhow!("Invalid command line syntax: {}", e)),
    };

    let mut include_dirs = Vec::<PathBuf>::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        // Case 1: -Idir (no space)
        if arg.starts_with("-I") && arg.len() > 2 && !arg[2..].starts_with('=') {
            include_dirs.push(arg[2..].to_string().into());
        }
        // Case 2: -I dir (with space)
        else if arg == "-I" && i + 1 < args.len() {
            include_dirs.push(args[i + 1].to_string().into());
            i += 1; // Skip the next argument as we've consumed it
        }
        // Case 3: -I=dir (with equals sign)
        else if let Some(stripped) = arg.strip_prefix("-I=") {
            include_dirs.push(stripped.to_string().into());
        }

        i += 1;
    }

    Ok(include_dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to convert string slices to PathBufs
    fn paths(dirs: &[&str]) -> Vec<PathBuf> {
        dirs.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn test_basic_cases() {
        assert_eq!(
            parse_include_dirs("g++ -Idir1 file.cpp").unwrap(),
            paths(&["dir1"])
        );
        assert_eq!(
            parse_include_dirs("g++ -I dir2 file.cpp").unwrap(),
            paths(&["dir2"])
        );
        assert_eq!(
            parse_include_dirs("g++ -I=dir3 file.cpp").unwrap(),
            paths(&["dir3"])
        );
    }

    #[test]
    fn test_multiple_includes() {
        assert_eq!(
            parse_include_dirs("g++ -Idir1 -Idir2 -I dir3 file.cpp").unwrap(),
            paths(&["dir1", "dir2", "dir3"])
        );
    }

    #[test]
    fn test_paths_with_spaces() {
        assert_eq!(
            parse_include_dirs("g++ -I\"dir with spaces\" file.cpp").unwrap(),
            paths(&["dir with spaces"])
        );
        assert_eq!(
            parse_include_dirs("g++ -I 'dir with spaces' file.cpp").unwrap(),
            paths(&["dir with spaces"])
        );
        assert_eq!(
            parse_include_dirs("g++ -I=dir\\ with\\ spaces file.cpp").unwrap(),
            paths(&["dir with spaces"])
        );
    }

    #[test]
    fn test_multiple_spaces() {
        assert_eq!(
            parse_include_dirs("g++ -I   dir4 file.cpp").unwrap(),
            paths(&["dir4"])
        );
    }

    #[test]
    fn test_mixed_with_other_options() {
        assert_eq!(
            parse_include_dirs("g++ -Wall -Wextra -O2 -Idir1 -I dir2 -I=dir3 -c file.cpp").unwrap(),
            paths(&["dir1", "dir2", "dir3"])
        );
    }

    #[test]
    fn test_absolute_paths() {
        assert_eq!(
            parse_include_dirs("g++ -I/usr/include -I /opt/include file.cpp").unwrap(),
            paths(&["/usr/include", "/opt/include"])
        );
    }

    #[test]
    fn test_relative_paths() {
        assert_eq!(
            parse_include_dirs("g++ -I../include -I ./local/include file.cpp").unwrap(),
            paths(&["../include", "./local/include"])
        );
    }

    #[test]
    fn test_paths_with_special_chars() {
        assert_eq!(
            parse_include_dirs("g++ -I/path/to/my-includes -I=/path/to/your_includes file.cpp")
                .unwrap(),
            paths(&["/path/to/my-includes", "/path/to/your_includes"])
        );
    }

    #[test]
    fn test_invalid_syntax() {
        // Test with unmatched quotes
        assert!(parse_include_dirs("g++ -I\"unclosed quote file.cpp").is_err());
    }
}
