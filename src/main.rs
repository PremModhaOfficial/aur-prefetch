use std::process::ExitCode;

use clap::{Parser, Subcommand};

use aur_prefetch::commands;

#[derive(Parser)]
#[command(
    name = "aur-prefetch",
    version,
    about = "Split Arch updates: background fetch/build, foreground install"
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
    Check,
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
    Doctor,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Init { apply } => commands::init::run(apply),
        Commands::Check => commands::check::run(),
        Commands::Fetch => commands::fetch::run(),
        Commands::Install { yes, force } => commands::install::run(yes, force),
        Commands::Clean { apply } => commands::clean::run(apply),
        Commands::Doctor => commands::doctor::run(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aur-prefetch: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}
