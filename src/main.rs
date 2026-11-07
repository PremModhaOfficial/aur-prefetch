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
    Init,
    /// Pending updates, disk, lock, caches, timer status
    Check,
    /// Free-space gate, pacman -Syuw only
    Fetch,
    /// News gate, lock check, then yay -Syuu
    Install,
    /// Prune download cache, yay builds, logs, orphans
    Clean,
    /// Verify timers, limits, repo, journald
    Doctor,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Init => commands::init::run(),
        Commands::Check => commands::check::run(),
        Commands::Fetch => commands::fetch::run(),
        Commands::Install => commands::install::run(),
        Commands::Clean => commands::clean::run(),
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
