use std::io;
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    match cli::run(std::env::args_os(), &mut stdout, &mut stderr) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            if writeln!(stderr, "error: {error:#}").is_err() {
                return ExitCode::FAILURE;
            }

            ExitCode::FAILURE
        }
    }
}
