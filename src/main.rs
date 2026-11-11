use std::process::ExitCode;

use clap::{Parser, Subcommand};

use aur_prefetch::commands;
use aur_prefetch::error::Error;

#[derive(Parser)]
#[command(
    name = "aur-prefetch",
    version,
    about = "Split Arch updates: background fetch/build, foreground install",
    after_help = "Exit codes: 0 healthy or done, 1 action needed or gated, 2 execution error, 130 Ctrl-C in the interactive TUI."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Set up timer, sudoers, checks
    Init {
        /// Write the artifacts and enable the timer. Without this, print them.
        #[arg(long)]
        apply: bool,
    },
    /// Pending updates, disk, lock, caches, timer status
    Check {
        /// Print one JSON object instead of the text report
        #[arg(long)]
        json: bool,
    },
    /// Free-space gate, pacman -Syuw only
    Fetch,
    /// News gate, lock check, then yay -Syuu
    Install {
        /// Continue without a TTY. Required when stdin is not a terminal.
        #[arg(long)]
        yes: bool,
        /// Proceed even when the trust file is empty.
        #[arg(long)]
        force: bool,
    },
    /// Prune download cache, yay builds, logs, orphans
    Clean {
        /// Delete. Without this flag the command only reports.
        #[arg(long)]
        apply: bool,
    },
    /// Verify timers, limits, repo, journald
    Doctor {
        /// Print one JSON object instead of the text report
        #[arg(long)]
        json: bool,
    },
    /// Interactive dependency tree
    Tui {
        /// Render one frame to stdout and exit. Works without a tty.
        #[arg(long)]
        render_once: bool,
        /// Use ASCII glyphs only
        #[arg(long)]
        ascii: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Init { apply } => commands::init::run(apply),
        Commands::Check { json } => commands::check::run(json),
        Commands::Fetch => commands::fetch::run(),
        Commands::Install { yes, force } => commands::install::run(yes, force),
        Commands::Clean { apply } => commands::clean::run(apply),
        Commands::Doctor { json } => commands::doctor::run(json),
        Commands::Tui { render_once, ascii } => aur_prefetch::tui::run(render_once, ascii),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Interrupted) => ExitCode::from(130),
        Err(error) => {
            eprintln!("aur-prefetch: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}
