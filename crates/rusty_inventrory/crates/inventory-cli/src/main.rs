//! `inv` — the whole capability set from the terminal.

mod render;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use inventory_core::keychain::StaticKey;
use inventory_core::{format, Inventory, Retention, SearchQuery, SourceId};
use render::{paint, ACCENT, BOLD, DIM, WARN};

#[derive(Parser)]
#[command(
    name = "inv",
    version,
    about = "One private index for every AI conversation on your machine",
    long_about = "Indexes the conversation history Claude Code, Codex, Cursor, Zed, Kiro and \
Antigravity already write to disk, and searches it by keyword and by meaning.\n\n\
Nothing leaves this machine. The index is a single encrypted file you can delete at any time."
)]
struct Cli {
    /// Use a specific index file instead of the default location.
    #[arg(long, global = true, value_name = "PATH")]
    index: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read every installed source and update the index.
    Index {
        /// Re-read every file, ignoring the unchanged-file cache.
        #[arg(long)]
        full: bool,
    },
    /// Search across every indexed tool at once.
    Search {
        /// What you remember about the conversation.
        query: Vec<String>,
        /// Restrict to one or more sources.
        #[arg(long, short, value_name = "SOURCE")]
        source: Vec<String>,
        #[arg(long, short, default_value_t = 10)]
        limit: usize,
        /// Keyword only — the ⌘M toggle, off.
        #[arg(long)]
        no_meaning: bool,
        /// Only conversations from the last N days.
        #[arg(long, value_name = "DAYS")]
        days: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// Keep the index live: watch the source stores and index as they change.
    Watch {
        /// Seconds between stats of the source stores.
        #[arg(long, default_value_t = 5)]
        interval: u64,
        /// Seconds a file must be untouched before it is read, so a
        /// transcript is never indexed while an agent is still writing it.
        #[arg(long, default_value_t = inventory_core::watch::DEFAULT_GRACE_SECS)]
        grace: i64,
    },
    /// Per-source status, including anything frozen.
    Sources,
    /// Print a whole conversation.
    Show { id: i64 },
    /// Save a thought, and see what you already worked out about it.
    Capture { text: Vec<String> },
    /// Recent captures.
    Notes {
        #[arg(long, short, default_value_t = 20)]
        limit: usize,
    },
    /// Clipboard scratchpad. Off by default.
    Scratch {
        #[command(subcommand)]
        action: ScratchAction,
    },
    /// Reopen a conversation in the tool that created it.
    Resume {
        id: i64,
        /// Actually launch it, rather than printing the command.
        #[arg(long)]
        run: bool,
    },
    /// Condense a conversation into an opening message for any other tool.
    Primer { id: i64 },
    /// How much history to keep.
    Retention {
        /// One of 7, 30, 90, 365, all. Omit to list every option with its cost.
        window: Option<String>,
    },
    /// Index size, coverage and configuration.
    Stats,
    /// What ⌘K shows: version, license and update state.
    Palette,
    /// Check the index is sound and the key is reachable.
    Doctor,
}

#[derive(Subcommand)]
enum ScratchAction {
    /// Turn the scratchpad on. Nothing is recorded until you do.
    On,
    /// Turn it off. Existing clips are kept until cleared.
    Off,
    /// Show what has been copied.
    List {
        #[arg(long, short, default_value_t = 20)]
        limit: usize,
    },
    /// Record a clip.
    Add {
        text: Vec<String>,
        #[arg(long, value_name = "APP")]
        app: Option<String>,
    },
    /// Print everything as plain text.
    Export,
    /// Delete everything in the scratchpad.
    Clear,
}

fn open(cli: &Cli) -> Result<Inventory> {
    match &cli.index {
        Some(path) => {
            // An explicit path is a development/inspection affordance, so it
            // takes a key from the environment rather than the machine
            // keychain, which is bound to the real index.
            let key =
                std::env::var(inventory_core::keychain::KEY_ENV).unwrap_or_else(|_| "0".repeat(64));
            Inventory::open_at(path, &StaticKey::new(key)).map_err(Into::into)
        }
        None => Inventory::open().map_err(Into::into),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{} {e:#}", paint(WARN, "error:"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Command::Index { full } => cmd_index(&cli, *full),
        Command::Search {
            query,
            source,
            limit,
            no_meaning,
            days,
            json,
        } => cmd_search(&cli, query, source, *limit, !*no_meaning, *days, *json),
        Command::Watch { interval, grace } => cmd_watch(&cli, *interval, *grace),
        Command::Sources => cmd_sources(&cli),
        Command::Show { id } => cmd_show(&cli, *id),
        Command::Capture { text } => cmd_capture(&cli, text),
        Command::Notes { limit } => cmd_notes(&cli, *limit),
        Command::Scratch { action } => cmd_scratch(&cli, action),
        Command::Resume { id, run } => cmd_resume(&cli, *id, *run),
        Command::Primer { id } => cmd_primer(&cli, *id),
        Command::Retention { window } => cmd_retention(&cli, window.as_deref()),
        Command::Stats => cmd_stats(&cli),
        Command::Palette => cmd_palette(&cli),
        Command::Doctor => cmd_doctor(&cli),
    }
}

fn cmd_index(cli: &Cli, full: bool) -> Result<()> {
    let mut inv = open(cli)?;
    let report = inv.index(full)?;

    for entry in &report.per_source {
        let Some(source) = entry.source else { continue };
        match entry.state {
            Some(inventory_core::SourceState::Absent) => {
                println!(
                    "  {} {:<14} {}",
                    paint(DIM, "○"),
                    source.display_name(),
                    paint(DIM, "not installed")
                );
            }
            Some(inventory_core::SourceState::Frozen) => {
                println!(
                    "  {} {:<14} {}",
                    paint(WARN, "●"),
                    source.display_name(),
                    paint(WARN, "frozen — existing history kept, will retry next run")
                );
                if let Some(e) = &entry.error {
                    println!("      {}", paint(DIM, e));
                }
            }
            _ => {
                println!(
                    "  {} {:<14} {}",
                    paint(ACCENT, "●"),
                    source.display_name(),
                    paint(
                        DIM,
                        &format!(
                            "{} new · {} updated · {} messages",
                            entry.conversations_added,
                            entry.conversations_updated,
                            entry.messages_indexed
                        )
                    )
                );
            }
        }
    }

    println!();
    if report.retrained {
        println!(
            "{}",
            paint(
                DIM,
                "Retrained the on-device semantic model from your own conversations."
            )
        );
    }
    if report.pruned > 0 {
        println!(
            "{}",
            paint(
                DIM,
                &format!(
                    "{} conversations dropped by the retention window.",
                    report.pruned
                )
            )
        );
    }
    println!(
        "{}",
        paint(
            DIM,
            &format!(
                "{} new, {} updated, {} embedded in {}ms.",
                report.total_added(),
                report.total_updated(),
                report.embeddings_written,
                report.elapsed_ms
            )
        )
    );
    Ok(())
}

fn parse_sources(values: &[String]) -> Result<Vec<SourceId>> {
    values
        .iter()
        .map(|v| v.parse::<SourceId>().map_err(Into::into))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn cmd_search(
    cli: &Cli,
    query: &[String],
    sources: &[String],
    limit: usize,
    meaning: bool,
    days: Option<i64>,
    json: bool,
) -> Result<()> {
    let text = query.join(" ");
    if text.trim().is_empty() {
        anyhow::bail!("give me something to search for");
    }
    let inv = open(cli)?;

    let mut q = SearchQuery::new(&text);
    q.sources = parse_sources(sources)?;
    q.limit = limit;
    q.meaning = meaning;
    q.since = days.map(|d| inventory_core::model::now_unix() - d * 86_400);

    let response = inv.search(&q)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        render::search_results(&response, &text);
    }
    Ok(())
}

fn cmd_watch(cli: &Cli, interval: u64, grace: i64) -> Result<()> {
    use inventory_core::Watcher;
    use std::time::Duration;

    let mut inv = open(cli)?;
    let interval = Duration::from_secs(interval.max(1));

    // Index the backlog once, then prime the watcher against the state that
    // index just consumed — so the first tick reports what changed *since*,
    // not the whole disk over again.
    let first = inv.index(false)?;
    inv.checkpoint()?;
    println!(
        "{}",
        paint(
            DIM,
            &format!(
                "Indexed {} new, {} updated in {}ms.",
                first.total_added(),
                first.total_updated(),
                first.elapsed_ms
            )
        )
    );

    let mut watcher = Watcher::new(grace);
    watcher.prime();
    println!(
        "{}",
        paint(
            DIM,
            &format!(
                "Watching {} files across the installed tools, every {}s. Ctrl-C to stop.",
                watcher.tracked_count(),
                interval.as_secs()
            )
        )
    );

    loop {
        std::thread::sleep(interval);
        let tick = watcher.poll();
        if !tick.needs_index() {
            continue;
        }

        let names: Vec<&str> = tick
            .changed_sources
            .iter()
            .map(|s| s.display_name())
            .collect();
        let report = inv.index(false)?;
        inv.checkpoint()?;
        println!(
            "{} {}",
            paint(
                DIM,
                &format!(
                    "{:>10}",
                    format::relative(inventory_core::model::now_unix())
                )
            ),
            paint(
                DIM,
                &format!(
                    "{} · {} new, {} updated ({}ms)",
                    names.join(", "),
                    report.total_added(),
                    report.total_updated(),
                    report.elapsed_ms
                )
            )
        );
        for entry in report.frozen() {
            if let Some(source) = entry.source {
                println!(
                    "           {}",
                    paint(
                        WARN,
                        &format!("{} froze — existing history kept", source.display_name())
                    )
                );
            }
        }
    }
}

fn cmd_sources(cli: &Cli) -> Result<()> {
    let inv = open(cli)?;
    render::sources(&inv.source_status()?);
    Ok(())
}

fn cmd_show(cli: &Cli, id: i64) -> Result<()> {
    let inv = open(cli)?;
    let (conversation, messages) = inv.conversation(id)?;

    println!("{}", paint(BOLD, &conversation.title));
    let mut meta = vec![
        conversation.source.display_name().to_string(),
        format::timestamp(conversation.updated_at),
    ];
    if let Some(p) = &conversation.project_path {
        meta.push(p.clone());
    }
    if let Some(b) = &conversation.git_branch {
        meta.push(b.clone());
    }
    println!("{}", paint(DIM, &meta.join(" · ")));
    println!();

    for m in messages {
        println!("{}", paint(ACCENT, &format!("── {}", m.role.as_str())));
        println!("{}\n", m.text);
    }
    Ok(())
}

fn cmd_capture(cli: &Cli, text: &[String]) -> Result<()> {
    let text = text.join(" ");
    let inv = open(cli)?;
    let result = inv.capture(&text)?;

    println!("{}", paint(DIM, "Captured."));
    if result.related.hits.is_empty() {
        println!("{}", paint(DIM, "Nothing related in your history yet."));
        return Ok(());
    }
    println!();
    println!("{}", paint(BOLD, "You may have already worked this out:"));
    println!();
    render::search_results(&result.related, &text);
    Ok(())
}

fn cmd_notes(cli: &Cli, limit: usize) -> Result<()> {
    let inv = open(cli)?;
    let notes = inv.notes(limit)?;
    if notes.is_empty() {
        println!("{}", paint(DIM, "No captures yet."));
        return Ok(());
    }
    for n in notes {
        println!(
            "{} {}",
            paint(DIM, &format!("{:>10}", format::relative(n.created_at))),
            n.text
        );
    }
    Ok(())
}

fn cmd_scratch(cli: &Cli, action: &ScratchAction) -> Result<()> {
    let inv = open(cli)?;
    match action {
        ScratchAction::On => {
            inv.set_scratchpad_enabled(true)?;
            println!("Clipboard scratchpad is {}.", paint(ACCENT, "on"));
            // Say plainly what it stores, rather than burying it.
            println!(
                "{}",
                paint(
                    DIM,
                    "Everything you copy is stored in the index, tagged with the app it came from,\n\
                     until you clear it. It is encrypted at rest with everything else."
                )
            );
        }
        ScratchAction::Off => {
            inv.set_scratchpad_enabled(false)?;
            println!("Clipboard scratchpad is {}.", paint(DIM, "off"));
            println!(
                "{}",
                paint(
                    DIM,
                    "Existing clips are kept — `inv scratch clear` deletes them."
                )
            );
        }
        ScratchAction::Add { text, app } => {
            let stored = inv.remember_clip(&text.join(" "), app.as_deref())?;
            if !stored {
                println!(
                    "{}",
                    paint(
                        DIM,
                        "Not stored — the scratchpad is off, or that was a duplicate."
                    )
                );
            }
        }
        ScratchAction::List { limit } => {
            if !inv.scratchpad_enabled()? {
                println!(
                    "{}",
                    paint(DIM, "Scratchpad is off. `inv scratch on` to enable.")
                );
            }
            let clips = inv.clips(*limit)?;
            if clips.is_empty() {
                println!("{}", paint(DIM, "Nothing copied yet."));
            }
            for c in clips {
                println!(
                    "{} {}",
                    paint(
                        DIM,
                        &format!(
                            "{:>10} {:<12}",
                            format::relative(c.created_at),
                            c.app.unwrap_or_else(|| "—".into())
                        )
                    ),
                    c.text.replace('\n', " ⏎ ")
                );
            }
            if let Some(prompt) = inv.scratchpad_prompt()? {
                println!();
                println!(
                    "{}",
                    paint(
                        WARN,
                        &format!(
                            "{} ({} clips). `inv scratch export > clips.txt` then `inv scratch clear`.",
                            prompt.reason, prompt.clips
                        )
                    )
                );
            }
        }
        ScratchAction::Export => print!("{}", inv.export_clips()?),
        ScratchAction::Clear => {
            let n = inv.clear_clips()?;
            println!("{}", paint(DIM, &format!("Cleared {n} clips.")));
        }
    }
    Ok(())
}

fn cmd_resume(cli: &Cli, id: i64, run: bool) -> Result<()> {
    let inv = open(cli)?;
    let cmd = inv.resume(id)?;

    if cmd.project_moved {
        println!(
            "{}",
            paint(
                WARN,
                "The original project folder is gone — running from your home directory instead."
            )
        );
    }
    println!("{}", paint(DIM, &format!("cd {}", cmd.cwd.display())));
    println!("{}", paint(BOLD, &cmd.display()));

    if !run {
        println!();
        println!("{}", paint(DIM, "Pass --run to launch it."));
        return Ok(());
    }

    let status = std::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .current_dir(&cmd.cwd)
        .status()
        .with_context(|| format!("could not launch `{}`", cmd.program))?;
    if !status.success() {
        anyhow::bail!("{} exited with {status}", cmd.program);
    }
    Ok(())
}

fn cmd_primer(cli: &Cli, id: i64) -> Result<()> {
    let inv = open(cli)?;
    print!("{}", inv.primer(id)?);
    Ok(())
}

fn cmd_retention(cli: &Cli, window: Option<&str>) -> Result<()> {
    let inv = open(cli)?;

    if let Some(w) = window {
        let retention: Retention = w.parse()?;
        let pruned = inv.set_retention(retention)?;
        println!("Keeping {}.", paint(BOLD, retention.label()));
        if pruned > 0 {
            println!(
                "{}",
                paint(
                    DIM,
                    &format!("{pruned} conversations dropped from the index.")
                )
            );
        }
        return Ok(());
    }

    // Show the on-disk cost of each choice before the trade is made.
    println!("{}", paint(BOLD, "How much history to keep"));
    println!();
    for option in inv.retention_options()? {
        let marker = if option.selected {
            paint(ACCENT, "●")
        } else {
            paint(DIM, "○")
        };
        println!(
            "  {marker} {:<12} {:<22} {}",
            option.retention.label(),
            paint(DIM, &format!("{} conversations", option.conversations)),
            paint(DIM, &format::bytes(option.bytes))
        );
    }
    println!();
    println!("{}", paint(DIM, "inv retention 90"));
    Ok(())
}

fn cmd_stats(cli: &Cli) -> Result<()> {
    let inv = open(cli)?;
    render::stats(&inv.stats()?);
    Ok(())
}

fn cmd_palette(cli: &Cli) -> Result<()> {
    let inv = open(cli)?;
    let stats = inv.stats()?;

    println!("{}", paint(BOLD, "Inventory"));
    println!();
    render::row("Version", inventory_core::VERSION);
    render::row("License", "one-time purchase · verified offline");
    render::row(
        "Update checks",
        if inv.update_checks_enabled()? {
            "on"
        } else {
            "off"
        },
    );
    render::row("Index", &inv.path().display().to_string());
    render::row("Retention", stats.retention.label());
    render::row("Conversations", &stats.conversations.to_string());
    println!();
    println!("{}", paint(DIM, "Shortcuts in the desktop app"));
    for (action, key) in [
        ("Search", "⌘⇧Space"),
        ("Quick capture", "⌘⇧N"),
        ("Clipboard scratchpad", "⌘⇧V"),
        ("Command palette", "⌘K"),
        ("Toggle meaning search", "⌘M"),
        ("Close", "Esc"),
    ] {
        render::row(action, key);
    }
    Ok(())
}

fn cmd_doctor(cli: &Cli) -> Result<()> {
    let inv = open(cli)?;
    let stats = inv.stats()?;

    println!("{}", paint(BOLD, "Checks"));
    println!();

    let ok = |b: bool| {
        if b {
            paint(ACCENT, "pass")
        } else {
            paint(WARN, "check")
        }
    };

    println!("  {} index opens with the machine key", ok(true));
    println!(
        "  {} encrypted at rest ({:.4} bits/byte — 8.0000 is the maximum)",
        ok(stats.encrypted),
        stats.entropy_bits_per_byte
    );
    println!(
        "  {} every conversation is embedded ({} of {})",
        ok(stats.embedded_conversations >= stats.conversations),
        stats.embedded_conversations,
        stats.conversations
    );

    let statuses = inv.source_status()?;
    let frozen: Vec<_> = statuses
        .iter()
        .filter(|s| s.state == inventory_core::SourceState::Frozen)
        .collect();
    println!(
        "  {} no source is frozen{}",
        ok(frozen.is_empty()),
        if frozen.is_empty() {
            String::new()
        } else {
            format!(
                " ({} — history retained, retried each index)",
                frozen
                    .iter()
                    .map(|s| s.source.display_name())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );

    let installed = statuses
        .iter()
        .filter(|s| s.state != inventory_core::SourceState::Absent)
        .count();
    println!("  {} {installed} of 6 sources present", ok(installed > 0));

    println!();
    println!(
        "{}",
        paint(
            DIM,
            "Encryption protects the index once it is away from this unlocked machine — a copied\n\
             backup, another account, the drive read elsewhere. It does not protect against a\n\
             process already running as you while the keychain is unlocked."
        )
    );
    Ok(())
}
