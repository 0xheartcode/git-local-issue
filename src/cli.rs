//! Command-line surface: parse args, drive the store/cache, render output.

use crate::cache::Cache;
use crate::error::GliError;
use crate::fsck;
use crate::git::{CliBackend, GitBackend};
use crate::model::{OpKind, State, StateVal};
use crate::store::Store;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "gli",
    version,
    about = "git-local-issue: a distributed, offline-first issue tracker stored natively in Git",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set up issue tracking in the current repository.
    Init,

    /// Create a new issue.
    Create {
        /// Issue title.
        title: String,
        /// Add a label (repeatable).
        #[arg(short = 'l', long = "label")]
        label: Vec<String>,
        /// Assign to someone.
        #[arg(short = 'a', long = "assignee")]
        assignee: Option<String>,
        /// Set a priority.
        #[arg(short = 'p', long = "priority")]
        priority: Option<String>,
        /// Description body.
        #[arg(short = 'd', long = "desc")]
        desc: Option<String>,
    },

    /// List issues.
    #[command(visible_alias = "list")]
    Ls {
        /// Filter by state (open|closed).
        #[arg(long)]
        state: Option<String>,
        /// Filter by label.
        #[arg(short = 'l', long = "label")]
        label: Option<String>,
        /// Output detail (short|full).
        #[arg(long, default_value = "short")]
        format: String,
    },

    /// Show one issue with its comments.
    Show {
        /// Issue id: actor-nonce (alice-3) or a uuid prefix.
        id: String,
    },

    /// Add a comment to an issue.
    Comment {
        /// Issue id.
        id: String,
        /// Comment text.
        text: String,
    },

    /// Edit an issue's fields.
    Edit {
        /// Issue id.
        id: String,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// New description body.
        #[arg(long = "desc")]
        desc: Option<String>,
        /// Add a label (repeatable).
        #[arg(long = "add-label")]
        add_label: Vec<String>,
        /// Remove a label (repeatable).
        #[arg(long = "remove-label")]
        remove_label: Vec<String>,
        /// Set the assignee (empty string clears it).
        #[arg(short = 'a', long = "assignee")]
        assignee: Option<String>,
    },

    /// Open or close an issue.
    State {
        /// Issue id.
        id: String,
        /// Target state (open|closed).
        state: String,
        /// Reason for the state change.
        #[arg(long)]
        reason: Option<String>,
        /// Commit sha that fixed the issue.
        #[arg(long = "fixed-by")]
        fixed_by: Option<String>,
    },

    /// Repository health: renumber notices, integrity, and local-only issues.
    Status,

    /// Validate issue data integrity.
    Fsck,
}

/// Parse args and run. Returns a process exit code.
pub fn run() -> i32 {
    let cli = Cli::parse();
    match dispatch(cli.command) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("gli: {err:#}");
            1
        }
    }
}

fn backend() -> Result<CliBackend> {
    let cwd = std::env::current_dir().context("resolving current directory")?;
    Ok(CliBackend::discover(cwd)?)
}

fn dispatch(command: Command) -> Result<i32> {
    match command {
        Command::Init => cmd_init(),
        Command::Create {
            title,
            label,
            assignee,
            priority,
            desc,
        } => cmd_create(title, label, assignee, priority, desc),
        Command::Ls {
            state,
            label,
            format,
        } => cmd_ls(state, label, format),
        Command::Show { id } => cmd_show(id),
        Command::Comment { id, text } => cmd_comment(id, text),
        Command::Edit {
            id,
            title,
            desc,
            add_label,
            remove_label,
            assignee,
        } => cmd_edit(id, title, desc, add_label, remove_label, assignee),
        Command::State {
            id,
            state,
            reason,
            fixed_by,
        } => cmd_state(id, state, reason, fixed_by),
        Command::Status => cmd_status(),
        Command::Fsck => cmd_fsck(),
    }
}

fn cmd_init() -> Result<i32> {
    let backend = backend()?;
    backend.set_config(
        "gli.formatVersion",
        &crate::model::FORMAT_VERSION.to_string(),
    )?;
    let root = backend.repo_root()?;
    let cache_dir = root.join(".git").join("gli");
    // Best-effort: the cache is disposable, so a failure here is not fatal.
    let _ = std::fs::create_dir_all(&cache_dir);
    println!("Initialized gli issue tracking in {}", root.display());
    println!("Issues live under refs/issues/*. Back them up with:");
    println!("  git bundle create issues.bundle refs/issues/*");
    Ok(0)
}

fn cmd_create(
    title: String,
    labels: Vec<String>,
    assignee: Option<String>,
    priority: Option<String>,
    desc: Option<String>,
) -> Result<i32> {
    let backend = backend()?;
    let store = Store::new(&backend);
    let uuid = store.create(
        &title,
        desc.as_deref().unwrap_or(""),
        labels,
        assignee.filter(|s| !s.is_empty()),
        priority.filter(|s| !s.is_empty()),
    )?;
    let cache = Cache::build(&backend)?;
    let display = cache.display_of(&uuid);
    println!("Created issue {} ({})", display.nonce, display.short);
    Ok(0)
}

fn cmd_ls(state: Option<String>, label: Option<String>, format: String) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;

    let state_filter = match state.as_deref() {
        Some(s) => Some(
            State::parse(s).ok_or_else(|| anyhow::anyhow!("invalid state '{s}' (open|closed)"))?,
        ),
        None => None,
    };

    let mut rows: Vec<&crate::model::Issue> = cache
        .issues
        .iter()
        .filter(|i| match &state_filter {
            Some(want) => &i.state().state == want,
            None => true,
        })
        .filter(|i| match &label {
            Some(l) => i.labels.contains(l),
            None => true,
        })
        .collect();
    // Newest first for listing.
    rows.sort_by(|a, b| b.uuid.cmp(&a.uuid));

    if rows.is_empty() {
        println!("No matching issues.");
        return Ok(0);
    }

    for issue in rows {
        let d = cache.display_of(&issue.uuid);
        let mark = match issue.state().state {
            State::Open => "open",
            State::Closed => "closed",
        };
        if format == "full" {
            println!("{} ({})  [{}]  {}", d.nonce, d.short, mark, issue.title());
            if let Some(a) = issue.assignee() {
                println!("    assignee: {a}");
            }
            if let Some(p) = issue.priority() {
                println!("    priority: {p}");
            }
            let labels = issue.labels();
            if !labels.is_empty() {
                println!("    labels: {}", labels.join(", "));
            }
            println!("    comments: {}", issue.comments.len());
        } else {
            let labels = issue.labels();
            let label_str = if labels.is_empty() {
                String::new()
            } else {
                format!("  [{}]", labels.join(", "))
            };
            println!(
                "{:<12} {:<8} {:<7} {}{}",
                d.nonce,
                d.short,
                mark,
                issue.title(),
                label_str
            );
        }
    }
    Ok(0)
}

fn cmd_show(id: String) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let issue = cache
        .issue(&uuid)
        .ok_or_else(|| GliError::UnknownIssue(id.clone()))?;
    let d = cache.display_of(&uuid);

    println!("{}  ({})", d.nonce, uuid);
    println!("Title:    {}", issue.title());
    let st = issue.state();
    print!("State:    {}", st.state.as_str());
    if let Some(r) = &st.reason {
        print!(" (reason: {r})");
    }
    if let Some(f) = &st.fixed_by {
        print!(" (fixed-by: {f})");
    }
    println!();
    if let Some(a) = issue.assignee() {
        println!("Assignee: {a}");
    }
    if let Some(p) = issue.priority() {
        println!("Priority: {p}");
    }
    let labels = issue.labels();
    if !labels.is_empty() {
        println!("Labels:   {}", labels.join(", "));
    }
    println!("Creator:  {}", issue.creator);
    if !issue.description().is_empty() {
        println!("\n{}", issue.description());
    }
    if issue.comments.is_empty() {
        println!("\nNo comments.");
    } else {
        println!("\nComments ({}):", issue.comments.len());
        for (i, c) in issue.comments.iter().enumerate() {
            let who = c.author.clone().unwrap_or_else(|| c.actor.clone());
            println!("  #{} by {}:", i + 1, who);
            for line in c.text.lines() {
                println!("    {line}");
            }
        }
    }
    Ok(0)
}

fn cmd_comment(id: String, text: String) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let store = Store::new(&backend);
    store.append(&uuid, OpKind::Comment { text })?;
    println!("Added comment to {}", cache.display_of(&uuid).nonce);
    Ok(0)
}

fn cmd_edit(
    id: String,
    title: Option<String>,
    desc: Option<String>,
    add_label: Vec<String>,
    remove_label: Vec<String>,
    assignee: Option<String>,
) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let store = Store::new(&backend);

    let mut changes = 0;
    if let Some(title) = title {
        store.append(&uuid, OpKind::SetTitle { title })?;
        changes += 1;
    }
    if let Some(description) = desc {
        store.append(&uuid, OpKind::SetDescription { description })?;
        changes += 1;
    }
    for label in add_label {
        store.append(&uuid, OpKind::AddLabel { label })?;
        changes += 1;
    }
    for label in remove_label {
        store.append(&uuid, OpKind::RemoveLabel { label })?;
        changes += 1;
    }
    if let Some(a) = assignee {
        store.append(
            &uuid,
            OpKind::SetAssignee {
                assignee: Some(a).filter(|s| !s.is_empty()),
            },
        )?;
        changes += 1;
    }

    if changes == 0 {
        println!("Nothing to change. Pass --title, --desc, --add-label, --remove-label, or -a.");
    } else {
        println!(
            "Applied {changes} change(s) to {}",
            cache.display_of(&uuid).nonce
        );
    }
    Ok(0)
}

fn cmd_state(
    id: String,
    state: String,
    reason: Option<String>,
    fixed_by: Option<String>,
) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let target = State::parse(&state)
        .ok_or_else(|| anyhow::anyhow!("invalid state '{state}' (open|closed)"))?;
    let store = Store::new(&backend);
    store.append(
        &uuid,
        OpKind::SetState {
            state: target.clone(),
            reason,
            fixed_by,
        },
    )?;
    let _ = StateVal::default(); // keep StateVal in scope for future summaries
    println!(
        "Set {} to {}",
        cache.display_of(&uuid).nonce,
        target.as_str()
    );
    Ok(0)
}

fn cmd_status() -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;

    let total = cache.issues.len();
    let open = cache
        .issues
        .iter()
        .filter(|i| i.state().state == State::Open)
        .count();
    let closed = total - open;
    println!("Issues: {total} ({open} open, {closed} closed)");

    // Display numbers are recomputed each run and may drift after a sync; there
    // is nothing to reconcile because we never persist numbers as truth.
    let mut per_actor: std::collections::BTreeMap<String, usize> = Default::default();
    for issue in &cache.issues {
        *per_actor.entry(issue.creator.clone()).or_default() += 1;
    }
    let multi: Vec<_> = per_actor.iter().filter(|(_, n)| **n > 1).collect();
    if multi.is_empty() {
        println!("Display numbers: stable (no actor has multiple issues).");
    } else {
        println!(
            "Display numbers: drift-allowed, recomputed each run. Actors with multiple issues:"
        );
        for (actor, n) in multi {
            println!("  {actor}: {n} issues (numbered by UUIDv7 order)");
        }
    }

    // Local-only / unpushed: compare issue refs against any remote-tracking
    // copies. With no sync configured (v1), everything is local.
    let remote_refs = backend.refs_matching("refs/remotes/*/issues/*")?;
    if remote_refs.is_empty() {
        println!("Sync: no remote issue refs found. All {total} issue(s) are local-only.");
        println!(
            "      (Push/fetch sync arrives in v2; back up with `git bundle create issues.bundle refs/issues/*`.)"
        );
    } else {
        let synced: std::collections::HashSet<String> =
            remote_refs.iter().map(|r| r.tip.clone()).collect();
        let unpushed: Vec<_> = cache
            .issues
            .iter()
            .filter(|i| {
                cache
                    .tips
                    .get(&i.uuid)
                    .map(|tip| !synced.contains(tip))
                    .unwrap_or(true)
            })
            .collect();
        if unpushed.is_empty() {
            println!("Sync: all issues match a remote-tracking ref.");
        } else {
            println!(
                "Sync: {} issue(s) have local commits not on any remote:",
                unpushed.len()
            );
            for issue in unpushed {
                println!(
                    "  {}  {}",
                    cache.display_of(&issue.uuid).nonce,
                    issue.title()
                );
            }
        }
    }

    Ok(0)
}

fn cmd_fsck() -> Result<i32> {
    let backend = backend()?;
    let report = fsck::check(&backend)?;
    for line in &report.notes {
        println!("{line}");
    }
    if report.problems.is_empty() {
        println!("fsck: {} issue(s) OK.", report.issue_count);
        Ok(0)
    } else {
        for problem in &report.problems {
            eprintln!("fsck: {problem}");
        }
        eprintln!("fsck: {} problem(s) found.", report.problems.len());
        Ok(1)
    }
}
