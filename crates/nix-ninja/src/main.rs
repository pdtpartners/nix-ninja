use nix_ninja::cli;

fn main() {
    let exit_code = match cli::run() {
        Ok(code) => code,
        Err(err) => {
            println!("nix-ninja: err: {err}");
            1
        }
    };
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}
