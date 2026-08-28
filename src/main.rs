use std::process::ExitCode;

/// Restore the default SIGPIPE disposition on Unix.
///
/// Rust installs SIG_IGN for SIGPIPE, which turns a closed downstream pipe
/// (`gli ls | head`) into a write error that `println!` escalates to a panic.
/// Resetting to SIG_DFL makes the process die quietly on the signal, exactly
/// like `git` and other Unix tools.
#[cfg(unix)]
fn reset_sigpipe() {
    // Safety: setting a signal handler to the default disposition is sound and
    // is the standard fix for this well-known Rust gotcha.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() -> ExitCode {
    reset_sigpipe();
    ExitCode::from(gli::cli::run() as u8)
}
