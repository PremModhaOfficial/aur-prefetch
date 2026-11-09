//! Full-screen TUI entry point.
//!
//! T1 is the read-only tree. It reads the report layer, owns the terminal, and
//! exits. Actions, fetch streaming and the install gate are later phases.

pub mod guard;
pub mod render;
pub mod sanitize;

use std::io::{self, IsTerminal, stdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::flag;

use crate::commands::check;
use crate::config::Config;
use crate::error::Error;
use crate::report::Snapshot;
use crate::runner::SystemRunner;

pub fn run(render_once: bool, ascii: bool) -> Result<(), Error> {
    let config = Config::from_env()?;
    let color = std::env::var_os("NO_COLOR").is_none();

    if render_once {
        // A pipe diagnostic must work without a tty. Fall back to the compact
        // supported size when the terminal cannot report one.
        let (width, height) = terminal::size().unwrap_or((80, 24));
        let snapshot = check::collect(&config, &SystemRunner, SystemTime::now())?;
        print!(
            "{}",
            render::render_frame(&snapshot, width, height, ascii, color, false)
        );
        if let Some(error) = snapshot.fatal.clone() {
            return Err(error);
        }
        return verdict(&snapshot);
    }

    ensure_interactive(
        io::stdin().is_terminal(),
        stdout().is_terminal(),
        std::env::var("TERM").ok().as_deref(),
    )?;
    run_interactive(&config, ascii, color)
}

fn ensure_interactive(stdin_tty: bool, stdout_tty: bool, term: Option<&str>) -> Result<(), Error> {
    if !stdin_tty || !stdout_tty {
        return Err(Error::failed(
            "tui needs a terminal; use --render-once in a pipe",
        ));
    }
    if term == Some("dumb") {
        return Err(Error::failed(
            "TERM=dumb has no cursor addressing; use --render-once",
        ));
    }
    Ok(())
}

fn run_interactive(config: &Config, ascii: bool, color: bool) -> Result<(), Error> {
    guard::install_panic_hook();
    let mut guard = guard::Guard::enter()
        .map_err(|err| Error::failed(format!("cannot enter the terminal: {err}")))?;

    // The sim image is built with `sim-hooks`; a packaged build never enables
    // it. This panics after raw mode and the alternate screen are active, so
    // the pty check proves the panic hook restores the terminal.
    #[cfg(feature = "sim-hooks")]
    if std::env::var_os("AUR_PREFETCH_TUI_TEST_PANIC").is_some() {
        panic!("forced TUI panic for terminal-restore test");
    }

    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
        .map_err(|err| Error::failed(format!("cannot start the terminal: {err}")))?;
    let mut snapshot = check::collect(config, &SystemRunner, SystemTime::now())?;
    let mut help = false;
    let mut dirty = true;

    let interrupted = Arc::new(AtomicBool::new(false));
    for signal in [SIGINT, SIGTERM, SIGHUP] {
        flag::register(signal, Arc::clone(&interrupted))
            .map_err(|err| Error::failed(format!("cannot watch signal {signal}: {err}")))?;
    }

    loop {
        if interrupted.load(Ordering::Relaxed) {
            return Err(Error::Interrupted);
        }

        if dirty {
            terminal
                .draw(|frame| render::draw(frame, &snapshot, ascii, color, help))
                .map_err(|err| Error::failed(format!("cannot draw the terminal: {err}")))?;
            dirty = false;
        }

        if !event::poll(Duration::from_millis(250))
            .map_err(|err| Error::failed(format!("cannot read the terminal: {err}")))?
        {
            continue;
        }

        match event::read()
            .map_err(|err| Error::failed(format!("cannot read the terminal: {err}")))?
        {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(Error::Interrupted);
                }
                KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    drop(terminal);
                    guard
                        .suspend()
                        .map_err(|err| Error::failed(format!("cannot suspend: {err}")))?;
                    terminal =
                        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|err| {
                            Error::failed(format!("cannot restart the terminal: {err}"))
                        })?;
                    terminal.clear().map_err(|err| {
                        Error::failed(format!("cannot clear the terminal: {err}"))
                    })?;
                    dirty = true;
                }
                KeyCode::Char('?') => {
                    help = !help;
                    dirty = true;
                }
                KeyCode::Esc if help => {
                    help = false;
                    dirty = true;
                }
                KeyCode::Char('r') => {
                    snapshot = check::collect(config, &SystemRunner, SystemTime::now())?;
                    dirty = true;
                }
                _ => {}
            },
            Event::Resize(_, _) => dirty = true,
            Event::Paste(_) => {}
            _ => {}
        }
    }

    drop(terminal);
    drop(guard);
    Ok(())
}

fn verdict(snapshot: &Snapshot) -> Result<(), Error> {
    if snapshot.actionable == 0 {
        Ok(())
    } else {
        Err(Error::blocked(format!(
            "{} item(s) need attention",
            snapshot.actionable
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_interactive;

    #[test]
    fn a_pipe_is_not_interactive() {
        assert!(ensure_interactive(false, true, Some("xterm")).is_err());
        assert!(ensure_interactive(true, false, Some("xterm")).is_err());
    }

    #[test]
    fn dumb_terminals_are_rejected() {
        assert!(ensure_interactive(true, true, Some("dumb")).is_err());
        assert!(ensure_interactive(true, true, Some("xterm-256color")).is_ok());
        assert!(ensure_interactive(true, true, None).is_ok());
    }
}
