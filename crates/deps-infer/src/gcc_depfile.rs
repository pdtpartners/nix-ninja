use crate::gcc_depfile_parser::{spawn_gcc_generate_depfile, DepsConfig};
use anyhow::{anyhow, Result};
use n2::scanner;
use std::path::{Path, PathBuf};

pub fn retrieve_c_includes(cmdline: &str) -> Result<Vec<PathBuf>> {
    let depfile_path = Path::new("/tmp/foo.d");

    spawn_gcc_generate_depfile(
        cmdline,
        &DepsConfig {
            output_path: depfile_path.into(),
            include_system_headers: false,
        },
    )?;

    let buf = scanner::read_file_with_nul(depfile_path)?;
    let mut scanner = scanner::Scanner::new(&buf);

    let depfile = n2::depfile::parse(&mut scanner)
        .map_err(|err| anyhow!(scanner.format_parse_error(depfile_path, err)))?;

    let mut deps: Vec<PathBuf> = Vec::new();
    for (_, values) in depfile.iter() {
        for value in values {
            deps.push(value.into());
        }
    }

    Ok(deps)
}
