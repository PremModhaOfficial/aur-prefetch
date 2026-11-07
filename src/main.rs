use clap::{Parser, Subcommand};

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

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Check => cmd_check(),
        Commands::Fetch => cmd_fetch(),
        Commands::Install => cmd_install(),
        Commands::Clean => cmd_clean(),
        Commands::Doctor => cmd_doctor(),
    }
}

fn cmd_init() {
    println!("init: not implemented yet");
}

fn cmd_check() {
    println!("check: not implemented yet");
}

fn cmd_fetch() {
    println!("fetch: not implemented yet");
}

fn cmd_install() {
    println!("install: not implemented yet");
}

fn cmd_clean() {
    println!("clean: not implemented yet");
}

fn cmd_doctor() {
    println!("doctor: not implemented yet");
}
