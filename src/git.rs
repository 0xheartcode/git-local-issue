//! The git access seam.
//!
//! All git access flows through the [`GitBackend`] trait so swapping an
//! operation (e.g. moving local reads onto gix) is a one-file change. v1 ships
//! a single [`CliBackend`] that shells out to `git`; per the locked decisions,
//! network ops will stay on the CLI permanently (go-git/gix transport keeps
//! breaking on SSH config/ports/known_hosts), while local reads may move to gix
//! later behind this same trait.

use crate::error::{GliError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One entry under `refs/issues/`.
#[derive(Clone, Debug)]
pub struct RefEntry {
    /// The issue uuid (the ref's leaf name).
    pub uuid: String,
    /// The commit the ref points at (chain tip).
    pub tip: String,
}

/// A raw commit read back from git, before op parsing.
#[derive(Clone, Debug)]
pub struct RawCommit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub message: String,
}

/// The git identity to stamp on a new op commit.
#[derive(Clone, Debug)]
pub struct Author {
    pub name: String,
    pub email: String,
}

/// The one seam for all git access.
pub trait GitBackend {
    /// Absolute path of the repository working tree root.
    fn repo_root(&self) -> Result<PathBuf>;

    /// Read a single git config value (`None` if unset or empty).
    fn config(&self, key: &str) -> Option<String>;

    /// List every `refs/issues/*` ref with its tip commit.
    fn list_issue_refs(&self) -> Result<Vec<RefEntry>>;

    /// Read an issue's whole commit chain, oldest first.
    fn read_chain(&self, tip: &str) -> Result<Vec<RawCommit>>;

    /// Create a commit with the empty tree, optional parent, and the given
    /// message and author. Returns the new commit sha.
    fn commit_op(&self, parent: Option<&str>, message: &str, author: &Author) -> Result<String>;

    /// Point a ref at `new`, optionally guarded by its expected `old` value
    /// (compare-and-swap; pass `None` when creating a new ref).
    fn update_ref(&self, refname: &str, new: &str, old: Option<&str>) -> Result<()>;

    /// Delete a ref outright (used by `archive --purge`; irreversible).
    fn delete_ref(&self, refname: &str) -> Result<()>;
}

/// A [`GitBackend`] that shells out to the `git` CLI.
pub struct CliBackend {
    /// Directory to run git in (any path inside the working tree).
    cwd: PathBuf,
}

impl CliBackend {
    /// Create a backend rooted at `cwd`, verifying it is inside a git repo.
    pub fn discover(cwd: impl AsRef<Path>) -> Result<CliBackend> {
        let backend = CliBackend {
            cwd: cwd.as_ref().to_path_buf(),
        };
        backend.repo_root()?;
        Ok(backend)
    }

    /// Run `git <args>` and return trimmed stdout as text.
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String> {
        let bytes = self.exec(args, stdin.map(|s| s.as_bytes()), &[])?;
        Ok(String::from_utf8_lossy(&bytes).trim_end().to_string())
    }

    /// The single git exec primitive: run `git <args>` with optional stdin and
    /// extra env vars, returning raw stdout bytes (never lossy) or a `Git`
    /// error carrying stderr. All other helpers wrap this.
    fn exec(&self, args: &[&str], stdin: Option<&[u8]>, env: &[(&str, &str)]) -> Result<Vec<u8>> {
        use std::io::Write;
        use std::process::Stdio;

        let mut cmd = Command::new("git");
        cmd.current_dir(&self.cwd)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });

        let mut child = cmd
            .spawn()
            .map_err(|e| GliError::Git(format!("failed to run git: {e}")))?;
        if let Some(data) = stdin {
            child
                .stdin
                .take()
                .expect("stdin piped")
                .write_all(data)
                .map_err(|e| GliError::Git(format!("writing to git stdin: {e}")))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| GliError::Git(format!("waiting on git: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(GliError::Git(format!(
                "`git {}` failed: {stderr}",
                args.join(" ")
            )));
        }
        Ok(out.stdout)
    }

    /// The sha of the empty tree (algorithm-agnostic; computed, not hardcoded).
    fn empty_tree(&self) -> Result<String> {
        self.run(&["hash-object", "-t", "tree", "--stdin"], Some(""))
    }

    /// Set a local git config value (used by `gli init` to stamp state).
    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        self.run(&["config", key, value], None)?;
        Ok(())
    }

    /// List refs matching a glob pattern with their tips (helper for status).
    pub fn refs_matching(&self, pattern: &str) -> Result<Vec<RefEntry>> {
        let out = self.run(
            &[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)",
                pattern,
            ],
            None,
        )?;
        let mut entries = Vec::new();
        for line in out.lines() {
            if let Some((refname, tip)) = line.split_once('\u{0}') {
                entries.push(RefEntry {
                    uuid: refname.to_string(),
                    tip: tip.to_string(),
                });
            }
        }
        Ok(entries)
    }
}

impl GitBackend for CliBackend {
    fn repo_root(&self) -> Result<PathBuf> {
        match self.run(&["rev-parse", "--show-toplevel"], None) {
            Ok(path) if !path.is_empty() => Ok(PathBuf::from(path)),
            _ => Err(GliError::NotAGitRepo),
        }
    }

    fn config(&self, key: &str) -> Option<String> {
        self.run(&["config", "--get", key], None)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    fn list_issue_refs(&self) -> Result<Vec<RefEntry>> {
        let out = self.run(
            &[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)",
                "refs/issues/",
            ],
            None,
        )?;
        let mut entries = Vec::new();
        for line in out.lines() {
            if line.is_empty() {
                continue;
            }
            let (refname, tip) = match line.split_once('\u{0}') {
                Some(x) => x,
                None => continue,
            };
            let uuid = refname
                .strip_prefix("refs/issues/")
                .unwrap_or(refname)
                .to_string();
            entries.push(RefEntry {
                uuid,
                tip: tip.to_string(),
            });
        }
        Ok(entries)
    }

    fn read_chain(&self, tip: &str) -> Result<Vec<RawCommit>> {
        // Get the chain shas oldest-first, then read each commit object with
        // `cat-file --batch`. The batch format is length-framed (`<sha> <type>
        // <size>\n<size bytes>\n`), so a message containing ANY byte (NUL, RS,
        // arbitrary control chars) is parsed unambiguously. A delimiter-based
        // `git log --format` cannot make that guarantee.
        let shas = self.run(&["rev-list", "--reverse", tip], None)?;
        if shas.is_empty() {
            return Ok(Vec::new());
        }
        let stdin = format!("{shas}\n");
        let out = self.exec(&["cat-file", "--batch"], Some(stdin.as_bytes()), &[])?;
        parse_batch(&out)
    }

    fn commit_op(&self, parent: Option<&str>, message: &str, author: &Author) -> Result<String> {
        let tree = self.empty_tree()?;
        let mut args: Vec<String> = vec!["commit-tree".into(), tree];
        if let Some(p) = parent {
            args.push("-p".into());
            args.push(p.to_string());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        // Stamp author and committer explicitly so this works even when the
        // repo has no user.name/user.email configured.
        let out = self.exec(
            &arg_refs,
            Some(message.as_bytes()),
            &[
                ("GIT_AUTHOR_NAME", &author.name),
                ("GIT_AUTHOR_EMAIL", &author.email),
                ("GIT_COMMITTER_NAME", &author.name),
                ("GIT_COMMITTER_EMAIL", &author.email),
            ],
        )?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    fn update_ref(&self, refname: &str, new: &str, old: Option<&str>) -> Result<()> {
        match old {
            Some(old) => self.run(&["update-ref", refname, new, old], None)?,
            None => self.run(&["update-ref", refname, new], None)?,
        };
        Ok(())
    }

    fn delete_ref(&self, refname: &str) -> Result<()> {
        self.run(&["update-ref", "-d", refname], None)?;
        Ok(())
    }
}

/// Parse the output of `git cat-file --batch` into raw commits.
///
/// Each record is `<sha> <type> <size>\n` followed by exactly `<size>` bytes of
/// object content and a trailing `\n`. Length framing means the commit message
/// can contain any byte without breaking the parse.
fn parse_batch(bytes: &[u8]) -> Result<Vec<RawCommit>> {
    let mut commits = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        // Read the header line up to the first newline.
        let nl = match bytes[pos..].iter().position(|&b| b == b'\n') {
            Some(i) => pos + i,
            None => break,
        };
        let header = String::from_utf8_lossy(&bytes[pos..nl]);
        let parts: Vec<&str> = header.split(' ').collect();
        if parts.len() != 3 {
            // "<sha> missing" or malformed; stop parsing defensively.
            break;
        }
        let sha = parts[0].to_string();
        let size: usize = parts[2]
            .trim()
            .parse()
            .map_err(|_| GliError::Git(format!("bad cat-file size in `{header}`")))?;
        let content_start = nl + 1;
        let content_end = content_start + size;
        if content_end > bytes.len() {
            return Err(GliError::Git("truncated cat-file output".into()));
        }
        let content = &bytes[content_start..content_end];
        commits.push(parse_commit_object(sha, content));
        // Skip the content and its trailing newline.
        pos = content_end + 1;
    }
    Ok(commits)
}

/// Parse a raw commit object (`tree`/`parent`/`author`/`committer` headers, a
/// blank line, then the message) into a [`RawCommit`].
fn parse_commit_object(sha: String, content: &[u8]) -> RawCommit {
    // Headers and message are separated by the first blank line ("\n\n").
    let text = String::from_utf8_lossy(content);
    let (headers, message) = match text.split_once("\n\n") {
        Some((h, m)) => (h, m.to_string()),
        None => (text.as_ref(), String::new()),
    };

    let mut parents = Vec::new();
    let mut author_name = String::new();
    let mut author_email = String::new();
    let mut author_time = 0i64;
    for line in headers.lines() {
        if let Some(p) = line.strip_prefix("parent ") {
            parents.push(p.trim().to_string());
        } else if let Some(a) = line.strip_prefix("author ") {
            let (name, email, time) = parse_identity(a);
            author_name = name;
            author_email = email;
            author_time = time;
        }
    }

    RawCommit {
        sha,
        parents,
        author_name,
        author_email,
        author_time,
        message,
    }
}

/// Parse a git identity line body: `Name <email> <unix-time> <tz>`.
fn parse_identity(s: &str) -> (String, String, i64) {
    let lt = s.find('<');
    let gt = s.find('>');
    let (name, email) = match (lt, gt) {
        (Some(lt), Some(gt)) if lt < gt => (s[..lt].trim().to_string(), s[lt + 1..gt].to_string()),
        _ => (s.trim().to_string(), String::new()),
    };
    let time = gt
        .and_then(|gt| s[gt + 1..].split_whitespace().next())
        .and_then(|t| t.parse::<i64>().ok())
        .unwrap_or(0);
    (name, email, time)
}
