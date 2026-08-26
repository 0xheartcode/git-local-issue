use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(gli::cli::run() as u8)
}
