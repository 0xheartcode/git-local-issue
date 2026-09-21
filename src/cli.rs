//! Command-line surface: parse args, drive the store/cache, render output.

use crate::cache::Cache;
use crate::error::GliError;
use crate::fsck;
use crate::git::{CliBackend, GitBackend};
use crate::model::{OpKind, State, StateVal};
use crate::store::Store;
use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "gli",
    // Stamped with the git commit by build.rs (deterministic; falls back to the
    // plain crate version when .git is absent).
    version = env!("GLI_VERSION"),
    about = "git-local-issue: a distributed, offline-first issue tracker stored natively in Git",
    long_about = "git-local-issue (gli) stores issues natively in Git under refs/issues/*: no \
external database, no server. Issues are distributed and offline-first, and sync by ordinary \
`git push` and `git fetch` (sync itself arrives in v2; this build is the local core).\n\n\
An <ID> is either an actor-scoped nonce like `alice-3` or a UUIDv7 prefix like `018f2a1c`. \
Nonces are recomputed on read and may drift; the UUIDv7 (prefix) is the permanent handle.",
    // Print the version in the help header too (git-bug #1535).
    help_template = "\
{name} {version}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}",
    after_help = "EXAMPLES:\n  \
gli init\n  \
gli create \"Login button does nothing\" -l bug -p high -a alice\n  \
gli ls --state open -l bug\n  \
gli show alice-1\n  \
gli comment alice-1 \"Reproduced on Firefox\"\n  \
gli edit alice-1 --desc \"Updated repro steps\" --add-label triaged\n  \
gli state alice-1 closed --reason \"fixed\" --fixed-by 1a2b3c4\n\n\
Run `gli <command> --help` for command-specific help.",
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Shared explanation of the <ID> argument, used on every command that takes one.
const ID_HELP: &str = "Issue id: an actor-scoped nonce (e.g. `alice-3`, case-insensitive) or a \
UUIDv7 prefix (e.g. `018f2a1c`). Prefixes are expanded automatically; an ambiguous prefix lists \
the candidates so you can retype a longer one.";

#[derive(Subcommand)]
enum Command {
    /// Set up issue tracking in the current repository.
    #[command(
        long_about = "Set up issue tracking in the current repository. Stamps the format \
version in git config and creates the disposable cache directory under .git/gli. Issues \
themselves are created lazily by `gli create`; you can run this in any existing git working \
tree."
    )]
    Init,

    /// Create a new issue.
    #[command(
        long_about = "Create a new issue. The issue is minted with a fresh UUIDv7 and a \
`create` operation commit under refs/issues/<uuid>, and starts in the open state. Labels, \
assignee and priority are optional and can be changed later with `gli edit`.",
        after_help = "\
EXAMPLES:\n  gli create \"Crash on startup\"\n  \
gli create \"Slow query\" -l perf -l db -p high -a bob -d \"p95 is 3s on /search\""
    )]
    Create {
        /// Issue title (a short one-line summary).
        title: String,
        /// Add a label; repeat -l for several (labels cannot contain commas).
        #[arg(short = 'l', long = "label")]
        label: Vec<String>,
        /// Assign the issue to someone (free-form name or handle).
        #[arg(short = 'a', long = "assignee")]
        assignee: Option<String>,
        /// Set a priority (free-form, e.g. low|medium|high).
        #[arg(short = 'p', long = "priority")]
        priority: Option<String>,
        /// Longer description body (may be multiple lines).
        #[arg(short = 'd', long = "desc")]
        desc: Option<String>,
    },

    /// List issues.
    #[command(
        visible_alias = "list",
        long_about = "List issues, newest first. All filters combine with AND. Archived issues \
are hidden unless --all or --archived is given. `--format full` adds assignee, priority, labels \
and comment counts; `--format json` emits a machine-readable array.",
        after_help = "EXAMPLES:\n  gli ls --state open -l bug -l frontend\n  \
gli ls --assignee alice --priority high\n  \
gli ls --search \"login\" --format full\n  \
gli ls --all --format json"
    )]
    Ls {
        /// Only show issues in this state: open or closed.
        #[arg(long, value_name = "open|closed")]
        state: Option<String>,
        /// Only show issues carrying this label; repeat for AND.
        #[arg(short = 'l', long = "label")]
        label: Vec<String>,
        /// Only show issues assigned to this person.
        #[arg(long)]
        assignee: Option<String>,
        /// Only show issues with this priority.
        #[arg(long)]
        priority: Option<String>,
        /// Only show issues created by this actor.
        #[arg(long, visible_alias = "author")]
        creator: Option<String>,
        /// Free-text search over title, description, and comments.
        #[arg(short = 's', long)]
        search: Option<String>,
        /// Show only archived issues.
        #[arg(long)]
        archived: bool,
        /// Include archived issues alongside active ones.
        #[arg(long)]
        all: bool,
        /// Sort order: newest (default), oldest, or title.
        #[arg(long, default_value = "newest", value_name = "newest|oldest|title")]
        sort: String,
        /// Output detail: short (one line each), full, or json.
        #[arg(long, value_name = "short|full|json")]
        format: Option<String>,
    },

    /// Show one issue with its comments.
    #[command(
        long_about = "Show one issue in full: title, state, assignee, priority, labels, \
description, and every comment in order."
    )]
    Show {
        #[arg(long_help = ID_HELP)]
        id: String,
        /// Print the raw operation log instead of the folded issue.
        #[arg(long)]
        ops: bool,
        /// Emit the issue as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },

    /// Add a comment to an issue.
    #[command(
        long_about = "Add a comment to an issue. Comments are a grow-only log: each gets \
a stable id so it never duplicates or reorders, even after a future sync."
    )]
    Comment {
        #[arg(long_help = ID_HELP)]
        id: String,
        /// The comment text. If omitted, $EDITOR opens to compose it.
        text: Option<String>,
    },

    /// Edit an issue's fields.
    #[command(
        long_about = "Edit an issue's fields. Any combination of flags may be given; each \
becomes its own operation. Editing the description (--desc) is supported, not just the title \
(git-bug #1488).",
        after_help = "EXAMPLES:\n  gli edit alice-1 --title \"Clearer title\"\n  \
gli edit alice-1 --add-label triaged --remove-label needs-info\n  \
gli edit alice-1 -p high        # change the priority\n  \
gli edit alice-1 -a \"\"   # clear the assignee"
    )]
    Edit {
        #[arg(long_help = ID_HELP)]
        id: String,
        /// Replace the title.
        #[arg(long)]
        title: Option<String>,
        /// Replace the description body.
        #[arg(long = "desc")]
        desc: Option<String>,
        /// Add a label; repeat for several.
        #[arg(long = "add-label")]
        add_label: Vec<String>,
        /// Remove a label; repeat for several.
        #[arg(long = "remove-label")]
        remove_label: Vec<String>,
        /// Set the sole assignee; pass an empty string ("") to clear all.
        #[arg(short = 'a', long = "assignee")]
        assignee: Option<String>,
        /// Add an assignee (issues support several); repeat for several.
        #[arg(long = "add-assignee")]
        add_assignee: Vec<String>,
        /// Remove an assignee; repeat for several.
        #[arg(long = "remove-assignee")]
        remove_assignee: Vec<String>,
        /// Set the priority (free-form, e.g. low|medium|high); pass "" to clear.
        #[arg(short = 'p', long = "priority")]
        priority: Option<String>,
    },

    /// Open or close an issue.
    #[command(
        long_about = "Open or close an issue, optionally recording why and the commit that \
fixed it.",
        after_help = "EXAMPLES:\n  gli state alice-1 closed --reason \"duplicate of alice-2\"\n  \
gli state alice-1 closed --fixed-by 1a2b3c4\n  gli state alice-1 open"
    )]
    State {
        #[arg(long_help = ID_HELP)]
        id: String,
        /// Target state: open or closed.
        #[arg(value_name = "open|closed")]
        state: String,
        /// Free-text reason for the change.
        #[arg(long)]
        reason: Option<String>,
        /// Commit sha that fixed the issue.
        #[arg(long = "fixed-by")]
        fixed_by: Option<String>,
    },

    /// Close an issue (shorthand for `state <id> closed`).
    #[command(
        long_about = "Close an issue. Shorthand for `gli state <id> closed`, with the same \
optional --reason and --fixed-by."
    )]
    Close {
        #[arg(long_help = ID_HELP)]
        id: String,
        /// Free-text reason for closing.
        #[arg(long)]
        reason: Option<String>,
        /// Commit sha that fixed the issue.
        #[arg(long = "fixed-by")]
        fixed_by: Option<String>,
    },

    /// Reopen an issue (shorthand for `state <id> open`).
    #[command(long_about = "Reopen an issue. Shorthand for `gli state <id> open`.")]
    Reopen {
        #[arg(long_help = ID_HELP)]
        id: String,
        /// Free-text reason for reopening.
        #[arg(long)]
        reason: Option<String>,
    },

    /// Archive an issue (hide it from listings; reversible).
    #[command(
        visible_alias = "rm",
        long_about = "Archive an issue: hide it from default listings without destroying \
anything. This is a reversible operation (see `restore`), and the whole op log is preserved. \
To permanently delete the underlying ref instead, pass --purge (irreversible, not sync-safe)."
    )]
    Archive {
        #[arg(long_help = ID_HELP)]
        id: String,
        /// Permanently delete the issue ref instead of archiving (irreversible).
        #[arg(long)]
        purge: bool,
    },

    /// Restore a previously archived issue.
    #[command(long_about = "Restore an archived issue so it appears in listings again.")]
    Restore {
        #[arg(long_help = ID_HELP)]
        id: String,
    },

    /// Repository health: renumber notices, integrity, and local-only issues.
    #[command(
        long_about = "Report repository health: total open/closed counts, whether any \
display numbers may drift, whether any actor slug is shared by two identities, and which issues \
have local commits not yet on a remote (git-bug #1566). Read-only."
    )]
    Status,

    /// Validate issue data integrity (and optionally repair).
    #[command(
        long_about = "Validate issue data integrity: every chain has exactly one root Create, \
ops parse, op-ids are unique, and the log folds cleanly. Exits non-zero if any problem is found. \
Read-only unless --fix is given.\n\nWith --fix, refs that are not usable issues (empty chain, \
unparseable, or will not fold) are quarantined: moved to refs/gli-quarantine/* so they stop \
breaking listings. This is non-destructive (the commits are preserved under the new ref) and \
recovers a repo that one bad ref had bricked.",
        after_help = "EXAMPLES:\n  gli fsck          # report only\n  gli fsck --fix    # quarantine unusable refs"
    )]
    Fsck {
        /// Quarantine unusable refs to refs/gli-quarantine/* (non-destructive).
        #[arg(long)]
        fix: bool,
    },

    /// Get, set, or list per-repository defaults.
    #[command(
        long_about = "Manage per-repository defaults, stored in git config under `gli.default.*`. \
Recognized keys: priority, assignee, labels (comma-separated), format. `create` and `ls` fall \
back to these when the matching flag is not given.",
        after_help = "EXAMPLES:\n  gli config set priority medium\n  \
gli config set labels triage,needs-info\n  gli config get priority\n  gli config list"
    )]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Generate a shell completion script (print to stdout).
    #[command(
        long_about = "Print a shell completion script to stdout. Redirect it into the location \
your shell loads completions from.",
        after_help = "EXAMPLES:\n  gli completions bash > ~/.local/share/bash-completion/completions/gli\n  \
gli completions zsh  > ~/.zfunc/_gli"
    )]
    Completions {
        /// Target shell.
        shell: clap_complete::Shell,
    },

    /// Print the man page (roff) to stdout.
    #[command(
        long_about = "Render the gli man page in roff format to stdout. Save it as `gli.1` on \
your MANPATH, e.g. gli man > /usr/local/share/man/man1/gli.1"
    )]
    Man,

    /// Get, set, list, or remove custom fields on an issue.
    #[command(
        long_about = "Custom fields are optional, free-form key/value metadata on an issue \
(milestone, type, severity, sprint, anything). Each key holds one value (last write wins). \
Nothing is enforced.",
        after_help = "EXAMPLES:\n  gli field set alice-1 milestone v0.2\n  \
gli field set alice-1 severity high\n  gli field get alice-1 milestone\n  \
gli field list alice-1\n  gli field rm alice-1 severity"
    )]
    Field {
        #[command(subcommand)]
        action: FieldAction,
    },

    /// Attach, detach, annotate, or list related files on an issue.
    #[command(
        long_about = "Related files are optional pointers to repo paths relevant to an issue, \
each with an optional note. They are metadata (pointers), not attachments. `note` edits an \
existing file's note.",
        after_help = "EXAMPLES:\n  gli file add alice-1 src/parser.rs -n \"bug is here\"\n  \
gli file note alice-1 src/parser.rs \"fixed in this file\"\n  gli file list alice-1\n  \
gli file rm alice-1 src/parser.rs"
    )]
    File {
        #[command(subcommand)]
        action: FileAction,
    },
}

/// Actions for `gli field`.
#[derive(Subcommand)]
enum FieldAction {
    /// Set (or overwrite) a field's value.
    Set {
        #[arg(long_help = ID_HELP)]
        id: String,
        key: String,
        value: String,
    },
    /// Print a field's value (nothing if unset).
    Get {
        #[arg(long_help = ID_HELP)]
        id: String,
        key: String,
    },
    /// Remove a field.
    Rm {
        #[arg(long_help = ID_HELP)]
        id: String,
        key: String,
    },
    /// List all fields on an issue.
    List {
        #[arg(long_help = ID_HELP)]
        id: String,
    },
}

/// Actions for `gli file`.
#[derive(Subcommand)]
enum FileAction {
    /// Attach a related file path (with an optional note).
    Add {
        #[arg(long_help = ID_HELP)]
        id: String,
        path: String,
        /// Optional note about why the file is relevant.
        #[arg(short = 'n', long)]
        note: Option<String>,
    },
    /// Edit the note on an already-attached file.
    Note {
        #[arg(long_help = ID_HELP)]
        id: String,
        path: String,
        /// The new note (empty clears it).
        note: String,
    },
    /// Detach a related file path.
    Rm {
        #[arg(long_help = ID_HELP)]
        id: String,
        path: String,
    },
    /// List related files on an issue.
    List {
        #[arg(long_help = ID_HELP)]
        id: String,
    },
}

/// Actions for `gli config`.
#[derive(Subcommand)]
enum ConfigAction {
    /// Print a default's value (nothing if unset).
    Get {
        /// One of: priority, assignee, labels, format.
        key: String,
    },
    /// Set a default.
    Set {
        /// One of: priority, assignee, labels, format.
        key: String,
        /// The value to store.
        value: String,
    },
    /// List all defaults currently set.
    List,
}

/// The config keys `gli config` accepts (stored as `gli.default.<key>`).
const CONFIG_KEYS: [&str; 4] = ["priority", "assignee", "labels", "format"];

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
            assignee,
            priority,
            creator,
            search,
            archived,
            all,
            sort,
            format,
        } => cmd_ls(LsArgs {
            state,
            labels: label,
            assignee,
            priority,
            creator,
            search,
            archived,
            all,
            sort,
            format,
        }),
        Command::Show { id, ops, json } => cmd_show(id, ops, json),
        Command::Comment { id, text } => cmd_comment(id, text),
        Command::Edit {
            id,
            title,
            desc,
            add_label,
            remove_label,
            assignee,
            add_assignee,
            remove_assignee,
            priority,
        } => cmd_edit(EditArgs {
            id,
            title,
            desc,
            add_label,
            remove_label,
            assignee,
            add_assignee,
            remove_assignee,
            priority,
        }),
        Command::State {
            id,
            state,
            reason,
            fixed_by,
        } => cmd_state(id, state, reason, fixed_by),
        Command::Close {
            id,
            reason,
            fixed_by,
        } => cmd_state(id, "closed".to_string(), reason, fixed_by),
        Command::Reopen { id, reason } => cmd_state(id, "open".to_string(), reason, None),
        Command::Archive { id, purge } => cmd_archive(id, purge),
        Command::Restore { id } => cmd_restore(id),
        Command::Status => cmd_status(),
        Command::Fsck { fix } => cmd_fsck(fix),
        Command::Config { action } => cmd_config(action),
        Command::Completions { shell } => {
            cmd_completions(shell);
            Ok(0)
        }
        Command::Man => cmd_man(),
        Command::Field { action } => cmd_field(action),
        Command::File { action } => cmd_file(action),
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
    println!("  git bundle create issues.bundle --glob='refs/issues/*'");
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

    // Fall back to per-repo defaults (gli.default.*) when a flag is absent.
    let default = |key: &str| backend.config(&format!("gli.default.{key}"));
    let labels = if labels.is_empty() {
        default("labels")
            .map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        labels
    };
    let assignee = assignee
        .filter(|s| !s.is_empty())
        .or_else(|| default("assignee"));
    let priority = priority
        .filter(|s| !s.is_empty())
        .or_else(|| default("priority"));

    let store = Store::new(&backend);
    let uuid = store.create(
        &title,
        desc.as_deref().unwrap_or(""),
        labels,
        assignee,
        priority,
    )?;
    let cache = Cache::build(&backend)?;
    let display = cache.display_of(&uuid);
    println!("Created issue {} ({})", display.nonce, display.short);
    Ok(0)
}

/// Parsed arguments for `ls`, grouped to keep the handler signature small.
struct LsArgs {
    state: Option<String>,
    labels: Vec<String>,
    assignee: Option<String>,
    priority: Option<String>,
    creator: Option<String>,
    search: Option<String>,
    archived: bool,
    all: bool,
    sort: String,
    format: Option<String>,
}

fn cmd_ls(args: LsArgs) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;

    // Format precedence: explicit flag, then gli.default.format, then "short".
    let format = args
        .format
        .clone()
        .or_else(|| backend.config("gli.default.format"))
        .unwrap_or_else(|| "short".to_string());

    let state_filter = match args.state.as_deref() {
        Some(s) => Some(
            State::parse(s).ok_or_else(|| anyhow::anyhow!("invalid state '{s}' (open|closed)"))?,
        ),
        None => None,
    };
    let search = args.search.as_deref().map(str::to_lowercase);

    let mut rows: Vec<&crate::model::Issue> = cache
        .issues
        .iter()
        // Archived visibility: hidden by default, only-archived with --archived,
        // both with --all.
        .filter(|i| {
            if args.all {
                true
            } else if args.archived {
                i.archived()
            } else {
                !i.archived()
            }
        })
        .filter(|i| match &state_filter {
            Some(want) => &i.state().state == want,
            None => true,
        })
        // Every requested label must be present (AND).
        .filter(|i| args.labels.iter().all(|l| i.labels.contains(l)))
        .filter(|i| match &args.assignee {
            Some(a) => i.assignees().iter().any(|x| x == a),
            None => true,
        })
        .filter(|i| match &args.priority {
            Some(p) => i.priority() == Some(p.as_str()),
            None => true,
        })
        .filter(|i| match &args.creator {
            Some(c) => &i.creator == c,
            None => true,
        })
        .filter(|i| match &search {
            Some(term) => issue_matches_search(i, term),
            None => true,
        })
        .collect();

    match args.sort.as_str() {
        "oldest" => rows.sort_by(|a, b| a.uuid.cmp(&b.uuid)),
        "title" => rows.sort_by_key(|a| a.title().to_lowercase()),
        // "newest" (default): UUIDv7 is time-ordered, so reverse-uuid is newest-first.
        _ => rows.sort_by(|a, b| b.uuid.cmp(&a.uuid)),
    }

    if format == "json" {
        println!("{}", crate::output::render_issues(&cache, &rows));
        return Ok(0);
    }

    if rows.is_empty() {
        println!("No matching issues.");
        return Ok(0);
    }

    for issue in rows {
        let d = cache.display_of(&issue.uuid);
        let mark = issue.state().state.as_str();
        let arch = if issue.archived() { " (archived)" } else { "" };
        if format == "full" {
            println!(
                "{} ({})  [{}]{}  {}",
                d.nonce,
                d.short,
                mark,
                arch,
                issue.title()
            );
            let assignees = issue.assignees();
            if !assignees.is_empty() {
                println!("    assignees: {}", assignees.join(", "));
            }
            if let Some(p) = issue.priority() {
                println!("    priority: {p}");
            }
            let labels = issue.labels();
            if !labels.is_empty() {
                println!("    labels: {}", labels.join(", "));
            }
            let fields = issue.fields();
            if !fields.is_empty() {
                let joined = fields
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("    fields: {joined}");
            }
            let files = issue.files();
            if !files.is_empty() {
                println!(
                    "    files: {}",
                    files
                        .iter()
                        .map(|(p, _)| p.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
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
                "{:<12} {:<8} {:<7} {}{}{}",
                d.nonce,
                d.short,
                mark,
                issue.title(),
                arch,
                label_str
            );
        }
    }
    Ok(0)
}

/// Case-insensitive substring match over an issue's title, description, labels,
/// and comment text. `term` is expected already lowercased.
fn issue_matches_search(issue: &crate::model::Issue, term: &str) -> bool {
    if issue.title().to_lowercase().contains(term)
        || issue.description().to_lowercase().contains(term)
    {
        return true;
    }
    if issue
        .labels()
        .iter()
        .any(|l| l.to_lowercase().contains(term))
    {
        return true;
    }
    issue
        .comments
        .iter()
        .any(|c| c.text.to_lowercase().contains(term))
}

fn cmd_show(id: String, ops: bool, json: bool) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let d = cache.display_of(&uuid);

    if json {
        let issue = cache
            .issue(&uuid)
            .ok_or_else(|| GliError::UnknownIssue(id.clone()))?;
        println!("{}", crate::output::render_issue(&cache, issue));
        return Ok(0);
    }

    if ops {
        // Raw operation log: the ground truth the folded view is computed from.
        let log = cache
            .ops
            .get(&uuid)
            .ok_or_else(|| GliError::UnknownIssue(id.clone()))?;
        println!(
            "{}  ({})  operation log ({} ops):",
            d.nonce,
            uuid,
            log.len()
        );
        for op in log {
            let commit = op.commit.as_deref().unwrap_or("-");
            let short = &commit[..7.min(commit.len())];
            println!(
                "  [lamport {:>4}] {:<15} op-id {}  ({})",
                op.lamport,
                op.kind.slug(),
                op.id,
                short
            );
        }
        return Ok(0);
    }

    let issue = cache
        .issue(&uuid)
        .ok_or_else(|| GliError::UnknownIssue(id.clone()))?;

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
    let assignees = issue.assignees();
    if !assignees.is_empty() {
        println!("Assignees: {}", assignees.join(", "));
    }
    if let Some(p) = issue.priority() {
        println!("Priority: {p}");
    }
    let labels = issue.labels();
    if !labels.is_empty() {
        println!("Labels:   {}", labels.join(", "));
    }
    for (k, v) in issue.fields() {
        println!("Field:    {k} = {v}");
    }
    println!("Creator:  {}", issue.creator);
    if issue.archived() {
        println!("Archived: yes");
    }
    let files = issue.files();
    if !files.is_empty() {
        println!("Files:");
        for (path, note) in files {
            match note {
                Some(n) => println!("  {path}  ({n})"),
                None => println!("  {path}"),
            }
        }
    }
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

fn cmd_comment(id: String, text: Option<String>) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;

    let text = match text {
        Some(t) => t,
        None => edit_in_editor("# Write your comment above. Lines starting with # are ignored.")?,
    };
    if text.trim().is_empty() {
        println!("Empty comment, nothing added.");
        return Ok(0);
    }

    let store = Store::new(&backend);
    store.append(&uuid, OpKind::Comment { text })?;
    println!("Added comment to {}", cache.display_of(&uuid).nonce);
    Ok(0)
}

/// Open `$EDITOR` (or `$VISUAL`, else `vi`) on a temporary file seeded with
/// `template`, and return the saved content with comment lines (`#`) and
/// surrounding blank lines stripped.
fn edit_in_editor(template: &str) -> Result<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    // A unique-enough temp path without pulling wall-clock into the model:
    // the process id plus the git dir keeps concurrent editors from clashing.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("gli-COMMENT-{}.md", std::process::id()));
    std::fs::write(&path, format!("\n{template}\n")).context("writing editor template")?;

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("launching editor '{editor}'"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Err(anyhow::anyhow!("editor '{editor}' exited without saving"));
    }

    let raw = std::fs::read_to_string(&path).context("reading edited file")?;
    let _ = std::fs::remove_file(&path);
    let body = raw
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(body.trim().to_string())
}

/// Parsed arguments for `edit`, grouped to keep the handler signature small.
struct EditArgs {
    id: String,
    title: Option<String>,
    desc: Option<String>,
    add_label: Vec<String>,
    remove_label: Vec<String>,
    assignee: Option<String>,
    add_assignee: Vec<String>,
    remove_assignee: Vec<String>,
    priority: Option<String>,
}

fn cmd_edit(args: EditArgs) -> Result<i32> {
    let EditArgs {
        id,
        title,
        desc,
        add_label,
        remove_label,
        assignee,
        add_assignee,
        remove_assignee,
        priority,
    } = args;
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let store = Store::new(&backend);
    let nonce = cache.display_of(&uuid).nonce;

    // Each field is its own durable operation (operation-sourcing). We apply
    // them in a fixed order and report each as it lands, so if one fails the
    // user can see exactly which changes were already applied.
    let mut applied = 0;
    if let Some(title) = title {
        store.append(
            &uuid,
            OpKind::SetTitle {
                title: title.clone(),
            },
        )?;
        println!("  set title: {}", first_line(&title));
        applied += 1;
    }
    if let Some(description) = desc {
        store.append(&uuid, OpKind::SetDescription { description })?;
        println!("  set description");
        applied += 1;
    }
    for label in add_label {
        store.append(
            &uuid,
            OpKind::AddLabel {
                label: label.clone(),
            },
        )?;
        println!("  add label: {label}");
        applied += 1;
    }
    for label in remove_label {
        store.append(
            &uuid,
            OpKind::RemoveLabel {
                label: label.clone(),
            },
        )?;
        println!("  remove label: {label}");
        applied += 1;
    }
    if let Some(a) = assignee {
        let cleared = a.is_empty();
        store.append(
            &uuid,
            OpKind::SetAssignee {
                assignee: Some(a.clone()).filter(|s| !s.is_empty()),
            },
        )?;
        if cleared {
            println!("  clear assignees");
        } else {
            println!("  set sole assignee: {a}");
        }
        applied += 1;
    }
    for who in add_assignee {
        store.append(
            &uuid,
            OpKind::AddAssignee {
                assignee: who.clone(),
            },
        )?;
        println!("  add assignee: {who}");
        applied += 1;
    }
    for who in remove_assignee {
        store.append(
            &uuid,
            OpKind::RemoveAssignee {
                assignee: who.clone(),
            },
        )?;
        println!("  remove assignee: {who}");
        applied += 1;
    }
    if let Some(p) = priority {
        let cleared = p.is_empty();
        store.append(
            &uuid,
            OpKind::SetPriority {
                priority: Some(p.clone()).filter(|s| !s.is_empty()),
            },
        )?;
        if cleared {
            println!("  clear priority");
        } else {
            println!("  set priority: {p}");
        }
        applied += 1;
    }

    if applied == 0 {
        println!(
            "Nothing to change. Pass --title, --desc, --add-label, --remove-label, -a, --add-assignee, --remove-assignee, or -p."
        );
    } else {
        println!("Applied {applied} change(s) to {nonce}");
    }
    Ok(0)
}

/// First line of a multi-line string, for compact echoing.
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("").trim()
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

fn cmd_archive(id: String, purge: bool) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let nonce = cache.display_of(&uuid).nonce;

    if purge {
        // Irreversible: delete the ref outright. Nothing else references it.
        backend.delete_ref(&crate::store::refname(&uuid))?;
        println!("Permanently deleted {nonce} ({uuid}).");
        return Ok(0);
    }

    let store = Store::new(&backend);
    store.append(&uuid, OpKind::SetArchived { archived: true })?;
    println!("Archived {nonce}. Restore it with `gli restore {nonce}`.");
    Ok(0)
}

fn cmd_restore(id: String) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let uuid = cache.resolve(&id)?;
    let store = Store::new(&backend);
    store.append(&uuid, OpKind::SetArchived { archived: false })?;
    println!("Restored {}.", cache.display_of(&uuid).nonce);
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

    // Actor-slug collisions: two distinct git identities can slug to the same
    // actor (e.g. `al@x` and `al@y` both -> `al`), silently sharing a nonce
    // namespace. Surface it from the create-op author identities we read back.
    let mut identities: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        Default::default();
    for ops in cache.ops.values() {
        if let Some(create) = ops
            .iter()
            .find(|o| matches!(o.kind, crate::model::OpKind::Create { .. }))
            && let Some(author) = &create.author
        {
            identities
                .entry(create.actor.clone())
                .or_default()
                .insert(author.clone());
        }
    }
    let collisions: Vec<_> = identities.iter().filter(|(_, ids)| ids.len() > 1).collect();
    if !collisions.is_empty() {
        println!("Actor slugs shared by multiple identities (nonce namespaces overlap):");
        for (actor, ids) in collisions {
            println!(
                "  {actor}: {}",
                ids.iter().cloned().collect::<Vec<_>>().join(", ")
            );
        }
    }

    // Local-only / unpushed: compare issue refs against any remote-tracking
    // copies. With no sync configured (v1), everything is local.
    let remote_refs = backend.refs_matching("refs/remotes/*/issues/*")?;
    if remote_refs.is_empty() {
        println!("Sync: no remote issue refs found. All {total} issue(s) are local-only.");
        println!(
            "      (Push/fetch sync arrives in v2; back up with `git bundle create issues.bundle --glob='refs/issues/*'`.)"
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

fn cmd_fsck(fix: bool) -> Result<i32> {
    let backend = backend()?;
    let mut report = fsck::check(&backend)?;
    for line in &report.notes {
        println!("{line}");
    }

    if fix && !report.repairable.is_empty() {
        let actions = fsck::quarantine(&backend, &report.repairable)?;
        for action in &actions {
            println!("fsck: {action}");
        }
        println!("fsck: quarantined {} unusable ref(s).", actions.len());
        // Re-check so the exit code reflects the post-repair state.
        report = fsck::check(&backend)?;
    } else if !fix && !report.repairable.is_empty() {
        println!(
            "fsck: {} unusable ref(s) can be quarantined with `gli fsck --fix`.",
            report.repairable.len()
        );
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

/// Validate that a config key is one gli understands.
fn check_config_key(key: &str) -> Result<()> {
    if CONFIG_KEYS.contains(&key) {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "unknown config key '{key}' (known: {})",
            CONFIG_KEYS.join(", ")
        ))
    }
}

fn cmd_config(action: ConfigAction) -> Result<i32> {
    let backend = backend()?;
    match action {
        ConfigAction::Get { key } => {
            check_config_key(&key)?;
            if let Some(v) = backend.config(&format!("gli.default.{key}")) {
                println!("{v}");
            }
        }
        ConfigAction::Set { key, value } => {
            check_config_key(&key)?;
            backend.set_config(&format!("gli.default.{key}"), &value)?;
            println!("Set default.{key} = {value}");
        }
        ConfigAction::List => {
            let mut any = false;
            for key in CONFIG_KEYS {
                if let Some(v) = backend.config(&format!("gli.default.{key}")) {
                    println!("{key} = {v}");
                    any = true;
                }
            }
            if !any {
                println!("No defaults set. Use `gli config set <key> <value>`.");
            }
        }
    }
    Ok(0)
}

fn cmd_completions(shell: clap_complete::Shell) {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "gli", &mut std::io::stdout());
}

fn cmd_man() -> Result<i32> {
    let man = clap_mangen::Man::new(Cli::command());
    man.render(&mut std::io::stdout())
        .context("rendering man page")?;
    Ok(0)
}

fn cmd_field(action: FieldAction) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let store = Store::new(&backend);
    match action {
        FieldAction::Set { id, key, value } => {
            let uuid = cache.resolve(&id)?;
            store.append(
                &uuid,
                OpKind::SetField {
                    key: key.clone(),
                    value: value.clone(),
                },
            )?;
            println!("Set {key} = {value} on {}", cache.display_of(&uuid).nonce);
        }
        FieldAction::Get { id, key } => {
            let uuid = cache.resolve(&id)?;
            let issue = cache
                .issue(&uuid)
                .ok_or_else(|| GliError::UnknownIssue(id.clone()))?;
            if let Some((_, v)) = issue.fields().into_iter().find(|(k, _)| k == &key) {
                println!("{v}");
            }
        }
        FieldAction::Rm { id, key } => {
            let uuid = cache.resolve(&id)?;
            // Clearing = set the field to an empty value.
            store.append(
                &uuid,
                OpKind::SetField {
                    key: key.clone(),
                    value: String::new(),
                },
            )?;
            println!("Removed field {key} from {}", cache.display_of(&uuid).nonce);
        }
        FieldAction::List { id } => {
            let uuid = cache.resolve(&id)?;
            let issue = cache
                .issue(&uuid)
                .ok_or_else(|| GliError::UnknownIssue(id.clone()))?;
            let fields = issue.fields();
            if fields.is_empty() {
                println!("No fields set.");
            } else {
                for (k, v) in fields {
                    println!("{k} = {v}");
                }
            }
        }
    }
    Ok(0)
}

fn cmd_file(action: FileAction) -> Result<i32> {
    let backend = backend()?;
    let cache = Cache::build(&backend)?;
    let store = Store::new(&backend);
    match action {
        FileAction::Add { id, path, note } => {
            let uuid = cache.resolve(&id)?;
            store.append(
                &uuid,
                OpKind::AddFile {
                    path: path.clone(),
                    note: note.unwrap_or_default(),
                },
            )?;
            println!("Attached {path} to {}", cache.display_of(&uuid).nonce);
        }
        FileAction::Note { id, path, note } => {
            let uuid = cache.resolve(&id)?;
            store.append(
                &uuid,
                OpKind::SetFileNote {
                    path: path.clone(),
                    note,
                },
            )?;
            println!("Updated note on {path}");
        }
        FileAction::Rm { id, path } => {
            let uuid = cache.resolve(&id)?;
            store.append(&uuid, OpKind::RemoveFile { path: path.clone() })?;
            println!("Detached {path} from {}", cache.display_of(&uuid).nonce);
        }
        FileAction::List { id } => {
            let uuid = cache.resolve(&id)?;
            let issue = cache
                .issue(&uuid)
                .ok_or_else(|| GliError::UnknownIssue(id.clone()))?;
            let files = issue.files();
            if files.is_empty() {
                println!("No related files.");
            } else {
                for (path, note) in files {
                    match note {
                        Some(n) => println!("{path}  ({n})"),
                        None => println!("{path}"),
                    }
                }
            }
        }
    }
    Ok(0)
}
