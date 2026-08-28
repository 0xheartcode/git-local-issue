//! Build script: stamp the version string with the git commit for `--version`.
//!
//! We embed the short commit SHA and the COMMIT date (not a wall-clock build
//! time), so the stamp is deterministic per commit and builds stay reproducible.
//! When `.git` is unavailable (for example a crates.io tarball), we fall back to
//! the plain crate version.

use std::process::Command;

fn main() {
    // Rebuild when HEAD moves so the stamp stays current.
    println!("cargo:rerun-if-changed=.git/HEAD");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let sha = git(&["rev-parse", "--short=10", "HEAD"]);
    let date = git(&["show", "-s", "--format=%cd", "--date=short", "HEAD"]);

    let full = match (sha, date) {
        (Some(sha), Some(date)) => format!("{version} ({sha} {date})"),
        (Some(sha), None) => format!("{version} ({sha})"),
        _ => version,
    };
    println!("cargo:rustc-env=GLI_VERSION={full}");
}

/// Run `git <args>` and return trimmed stdout, or `None` on any failure (git
/// missing, not a repo, empty output).
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
