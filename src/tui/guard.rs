//! Terminal ownership: raw mode, the alternate screen, and the Drop guard.
//!
//! Every path out of the TUI must restore the terminal. The guard covers the
//! normal returns and errors; the panic hook covers a panic.

use std::io::{self, stdout};
use std::panic;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

pub struct Guard {
    active: bool,
}

impl Guard {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = stdout();
        if let Err(err) = execute!(out, EnterAlternateScreen, EnableBracketedPaste, Hide) {
            let _ = disable_raw_mode();
            return Err(err);
        }
        Ok(Self { active: true })
    }

    pub fn restore(&mut self) {
        if !self.active {
            return;
        }
        restore_terminal();
        self.active = false;
    }

    /// Leave the terminal, stop this process, and re-enter when resumed.
    ///
    /// `kill -TSTP` is used instead of a libc call so the crate keeps its
    /// no-unsafe rule. The child exits before the parent stops.
    pub fn suspend(&mut self) -> io::Result<()> {
        self.restore();

        let pid = std::process::id().to_string();
        let status = std::process::Command::new("kill")
            .args(["-TSTP", &pid])
            .status()?;
        if !status.success() {
            return Err(io::Error::other("kill -TSTP failed"));
        }

        *self = Self::enter()?;
        Ok(())
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub fn restore_terminal() {
    let mut out = stdout();
    let _ = execute!(out, LeaveAlternateScreen, DisableBracketedPaste, Show);
    let _ = disable_raw_mode();
}

pub fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}
