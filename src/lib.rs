//! Split Arch Linux updates: download in the background, install in the
//! foreground.

pub mod commands;
pub mod config;
pub mod error;
pub mod fetch_source;
pub mod frontier;
pub mod gate;
pub mod lock;
pub mod post_install;
pub mod progress;
pub mod report;
pub mod runner;
pub mod state;
pub mod tui;
pub mod util;
