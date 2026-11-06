use std::process::ExitCode;

fn main() -> ExitCode {
    match t3bot::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("t3bot: {e:#}");
            ExitCode::FAILURE
        }
    }
}
