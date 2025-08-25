use std::path::{Path, PathBuf};
use std::process::Command;

/// Error types for dependency extraction
#[derive(Debug)]
pub enum DepsError {
    ParseError(String),
    ExecutionError(std::io::Error),
    UnsupportedCompiler(String),
    ProcessFailed(i32, String),
}

impl From<std::io::Error> for DepsError {
    fn from(error: std::io::Error) -> Self {
        DepsError::ExecutionError(error)
    }
}

impl std::fmt::Display for DepsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DepsError::ParseError(msg) => write!(f, "Failed to parse command: {msg}"),
            DepsError::ExecutionError(err) => write!(f, "Execution error: {err}"),
            DepsError::UnsupportedCompiler(compiler) => {
                write!(f, "Unsupported compiler: {compiler}")
            }
            DepsError::ProcessFailed(code, output) => {
                write!(f, "Process failed with exit code {code}: {output}")
            }
        }
    }
}

impl std::error::Error for DepsError {}

/// Configuration for dependency extraction
pub struct DepsConfig {
    /// Path where the dependency file should be written
    pub output_path: PathBuf,

    /// Whether to include system headers in dependencies
    pub include_system_headers: bool,
}

impl Default for DepsConfig {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("deps.d"),
            include_system_headers: false,
        }
    }
}

/// List of supported GCC-compatible compilers
static SUPPORTED_COMPILERS: &[&str] = &[
    "gcc", "g++", "clang", "clang++", "cc", "c++", "emcc", "em++",
];

/// Creates a command that will only generate dependencies from a compiler command
pub fn create_deps_command(cmdline: &str, config: &DepsConfig) -> Result<Command, DepsError> {
    // Parse the command using shellwords
    let args = match shell_words::split(cmdline) {
        Ok(args) => args,
        Err(e) => return Err(DepsError::ParseError(e.to_string())),
    };

    if args.is_empty() {
        return Err(DepsError::ParseError("Empty command".to_string()));
    }

    // Check if compiler is supported
    let compiler = &args[0];
    let compiler_name = Path::new(compiler)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(compiler);

    if !SUPPORTED_COMPILERS
        .iter()
        .any(|&c| compiler_name == c || compiler_name.contains(c))
    {
        return Err(DepsError::UnsupportedCompiler(compiler.clone()));
    }

    let mut cmd = Command::new(compiler);

    let mut include_flags = Vec::new();
    let mut std_flag = None;
    let mut define_flags = Vec::new();
    let mut input_file = None;

    // Process arguments
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];

        // Handle include paths
        if arg.starts_with("-I") {
            if arg.len() > 2 {
                include_flags.push(arg.clone());
            } else if i + 1 < args.len() {
                include_flags.push(format!("-I{}", args[i + 1]));
                i += 1;
            }
        }
        // Handle system include paths
        else if arg.starts_with("-isystem") {
            if arg.len() > 8 {
                include_flags.push(arg.clone());
            } else if i + 1 < args.len() {
                include_flags.push(format!("-isystem{}", args[i + 1]));
                i += 1;
            }
        }
        // Handle language standard
        else if arg.starts_with("-std=") {
            std_flag = Some(arg.clone());
        }
        // Handle preprocessor definitions
        else if arg.starts_with("-D") {
            if arg.len() > 2 {
                define_flags.push(arg.clone());
            } else if i + 1 < args.len() {
                define_flags.push(format!("-D{}", args[i + 1]));
                i += 1;
            }
        }
        // Find input file
        else if !arg.starts_with("-") && arg.contains(".") {
            input_file = Some(arg.clone());
        }
        // Skip output file specification
        else if (arg == "-o" || arg == "-MF" || arg == "-MQ") && i + 1 < args.len() {
            i += 1; // Skip the argument too
        }

        i += 1;
    }

    // Make sure we identified an input file
    let input_file = match input_file {
        Some(file) => file,
        None => {
            return Err(DepsError::ParseError(
                "Could not identify input file".to_string(),
            ))
        }
    };

    for flag in &include_flags {
        cmd.arg(flag);
    }
    if let Some(flag) = std_flag {
        cmd.arg(flag);
    }
    for flag in &define_flags {
        cmd.arg(flag);
    }

    // Add dependency generation flags
    if config.include_system_headers {
        cmd.arg("-M");
    } else {
        cmd.arg("-MM");
    }
    cmd.arg("-MF").arg(&config.output_path);
    cmd.arg(input_file);

    Ok(cmd)
}

/// Spawn a process that will only generate gcc-style dependency information
/// without compiling
pub fn spawn_gcc_generate_depfile(cmdline: &str, config: &DepsConfig) -> Result<(), DepsError> {
    let mut cmd = create_deps_command(cmdline, config)?;
    let output = cmd.output()?;

    if !output.status.success() {
        let error_output = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(DepsError::ProcessFailed(
            output.status.code().unwrap_or(-1),
            error_output,
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to get command as a string for testing
    fn cmd_to_string(cmd: &Command) -> String {
        let program = cmd.get_program().to_string_lossy();
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        format!("{} {}", program, args.join(" "))
    }

    struct TestCase {
        name: &'static str,
        input: &'static str,
        config: DepsConfig,
        expected: Result<&'static str, DepsError>,
    }

    #[test]
    fn test_create_deps_command() {
        let test_cases = vec![
            TestCase {
                name: "basic command",
                input: "g++ -Iinclude -I. -Wall -O2 -std=c++14 -DDEBUG -o output.o -c src/main.cpp",
                config: DepsConfig::default(),
                expected: Ok("g++ -Iinclude -I. -std=c++14 -DDEBUG -MM -MF deps.d src/main.cpp"),
            },
            TestCase {
                name: "spaces in include paths",
                input: "g++ -I include -I . -I /usr/include -std=c++14 -c main.cpp",
                config: DepsConfig::default(),
                expected: Ok("g++ -Iinclude -I. -I/usr/include -std=c++14 -MM -MF deps.d main.cpp"),
            },
            TestCase {
                name: "unsupported compiler",
                input: "rustc -c file.rs",
                config: DepsConfig::default(),
                expected: Err(DepsError::UnsupportedCompiler("rustc".to_string())),
            },
            TestCase {
                name: "include system headers",
                input: "g++ -isystem /usr/include/boost -c file.cpp",
                config: DepsConfig {
                    output_path: PathBuf::from("system.d"),
                    include_system_headers: true,
                },
                expected: Ok("g++ -isystem/usr/include/boost -M -MF system.d file.cpp"),
            },
            TestCase {
                name: "MQ MF flags removal",
                input: "g++ -c file.cpp -MQ file.o -MF file.d",
                config: DepsConfig::default(),
                expected: Ok("g++ -MM -MF deps.d file.cpp"),
            },
            TestCase {
                name: "real world example",
                input: "g++ -Ihello.p -I. -I.. -I/nix/store/b2zcd1z08y0bgiiradpk34g03ny5765y-boost-1.87.0-dev/include -fdiagnostics-color=always -D_GLIBCXX_ASSERTIONS=1 -D_FILE_OFFSET_BITS=64 -Wall -Winvalid-pch -std=c++14 -O0 -g -DBOOST_ALL_NO_LIB -MD -MQ hello.p/main.cpp.o -MF hello.p/main.cpp.o.d -o hello.p/main.cpp.o -c ../main.cpp",
                config: DepsConfig::default(),
                expected: Ok("g++ -Ihello.p -I. -I.. -I/nix/store/b2zcd1z08y0bgiiradpk34g03ny5765y-boost-1.87.0-dev/include -std=c++14 -D_GLIBCXX_ASSERTIONS=1 -D_FILE_OFFSET_BITS=64 -DBOOST_ALL_NO_LIB -MM -MF deps.d ../main.cpp"),
            },
            TestCase {
                name: "escaped quotes and spaces",
                input: "g++ -I\"path with spaces\" -D\"MACRO=\\\"value with spaces\\\"\" -c file.cpp",
                config: DepsConfig::default(),
                expected: Ok("g++ -Ipath with spaces -DMACRO=\"value with spaces\" -MM -MF deps.d file.cpp"),
            },
        ];

        for tc in test_cases {
            println!("Testing: {}", tc.name);

            let result = create_deps_command(tc.input, &tc.config);

            match (&tc.expected, &result) {
                (Ok(expected_cmd), Ok(cmd)) => {
                    let cmd_str = cmd_to_string(cmd);
                    assert_eq!(cmd_str, *expected_cmd, "Test '{}' failed", tc.name);
                }
                (Ok(_), Err(err)) => {
                    panic!(
                        "Test '{}' failed: expected success, got error: {}",
                        tc.name, err
                    );
                }
                (Err(_), Ok(cmd)) => {
                    panic!(
                        "Test '{}' failed: expected error, but got success: {}",
                        tc.name,
                        cmd_to_string(cmd)
                    );
                }
                (Err(expected_err), Err(err)) => {
                    let expected_str = expected_err.to_string();
                    let actual_str = err.to_string();
                    assert_eq!(
                        actual_str, expected_str,
                        "Test '{}' failed: error mismatch",
                        tc.name
                    );
                }
            }
        }
    }
}
