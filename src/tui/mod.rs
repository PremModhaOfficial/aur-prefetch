//! Full-screen TUI entry point.
//!
//! The static tree is read-only. `F` runs the existing `fetch` command through
//! [`fetch_session`] and draws the real cache until it exits. The install gate
//! and the repair card are later phases.

pub mod fetch_session;
pub mod guard;
pub mod live;
pub mod render;
pub mod sanitize;
pub mod tree;

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
use crate::commands::fetch::Gate;
use crate::config::Config;
use crate::error::Error;
use crate::progress;
use crate::report::Snapshot;
use crate::runner::SystemRunner;
use crate::tui::fetch_session::{FetchSession, StartFailure};
use crate::tui::tree::Tree;

pub fn run(render_once: bool, ascii: bool) -> Result<(), Error> {
    let config = Config::from_env()?;
    let color = use_color(ascii, std::env::var_os("NO_COLOR").is_some());

    if render_once {
        // A pipe diagnostic must work without a tty. Fall back to the compact
        // supported size when the terminal cannot report one.
        let (width, height) = terminal::size().unwrap_or((80, 24));
        let snapshot = collect_snapshot(&config, SystemTime::now())?;
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

/// ASCII mode is a no-color mode: the glyph fallback is the signal, and the
/// layout must read the same with or without a palette.
fn use_color(ascii: bool, no_color: bool) -> bool {
    !ascii && !no_color
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
    let mut snapshot = collect_snapshot(config, SystemTime::now())?;
    let mut session: Option<FetchSession> = None;
    let mut notice: Option<Notice> = None;
    let mut help = false;
    let mut ledger = false;
    let mut log_open = false;
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

        if session.as_ref().is_some_and(FetchSession::running) {
            let tick = session
                .as_mut()
                .expect("a running session is present")
                .tick(SystemTime::now(), log_open);
            if tick.changed {
                dirty = true;
            }
        }

        if dirty {
            terminal
                .draw(|frame| {
                    match (session.as_ref(), notice.as_ref()) {
                        (Some(live), _) => render::draw_live_status(
                            frame,
                            &snapshot,
                            live.live(),
                            live.footer(ascii),
                            ascii,
                            color,
                            help,
                            ledger,
                            Some(live.gates()),
                        ),
                        (None, Some(notice)) => render::draw_status(
                            frame,
                            &snapshot,
                            render::Footer {
                                status: Some(&notice.status),
                                keys: Some(notice_keys(ascii)),
                                failure: notice.failure,
                                repair: None,
                            },
                            ascii,
                            color,
                            help,
                            ledger,
                            Some(&notice.gates),
                        ),
                        (None, None) => {
                            let keys = static_keys(ascii, ledger);
                            render::draw_status(
                                frame,
                                &snapshot,
                                render::Footer {
                                    status: None,
                                    keys: Some(keys),
                                    failure: false,
                                    repair: None,
                                },
                                ascii,
                                color,
                                help,
                                ledger,
                                None,
                            )
                        }
                    }
                    if log_open {
                        if let Some(live) = session.as_ref() {
                            render::draw_log(frame, live.lines(), ascii, color);
                        }
                    }
                })
                .map_err(|err| Error::failed(format!("cannot draw the terminal: {err}")))?;
            dirty = false;
        }

        let wait = if session.as_ref().is_some_and(FetchSession::running) {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(250)
        };
        if !event::poll(wait)
            .map_err(|err| Error::failed(format!("cannot read the terminal: {err}")))?
        {
            continue;
        }

        match event::read()
            .map_err(|err| Error::failed(format!("cannot read the terminal: {err}")))?
        {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') => {
                    // Detach, never kill: the drop below closes our end of the
                    // stream, but the worker keeps the child alive.
                    break;
                }
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
                KeyCode::Esc => {
                    if help {
                        help = false;
                    } else if log_open {
                        log_open = false;
                    } else if session.as_ref().is_some_and(FetchSession::running) {
                        // Detach while the command runs. No result is claimed.
                        break;
                    } else if session.take().is_some() {
                        snapshot = collect_snapshot(config, SystemTime::now())?;
                    } else {
                        notice = None;
                    }
                    dirty = true;
                }
                KeyCode::Char('g') => {
                    // A view toggle only; it never touches the fetch or a file.
                    ledger = !ledger;
                    dirty = true;
                }
                KeyCode::Char('l') => {
                    // The log exists only after a fetch session starts.
                    if session.is_some() {
                        log_open = !log_open;
                        dirty = true;
                    }
                }
                KeyCode::Char('r') => {
                    if session.as_ref().is_some_and(FetchSession::running) {
                        // The live poll owns the refresh while the fetch runs.
                    } else {
                        session = None;
                        notice = None;
                        log_open = false;
                        snapshot = collect_snapshot(config, SystemTime::now())?;
                        dirty = true;
                    }
                }
                KeyCode::Char('F') => {
                    if session.as_ref().is_some_and(FetchSession::running) {
                        // One fetch at a time. No second child.
                    } else {
                        if session.is_some() || notice.is_some() {
                            session = None;
                            snapshot = collect_snapshot(config, SystemTime::now())?;
                        }
                        match start_fetch(config, &snapshot, SystemTime::now()) {
                            Ok(started) => {
                                session = Some(started);
                                notice = None;
                                log_open = false;
                            }
                            Err(failure) => notice = Some(Notice::from_failure(failure)),
                        }
                        dirty = true;
                    }
                }
                _ => {}
            },
            Event::Resize(_, _) => dirty = true,
            Event::Paste(_) => {}
            _ => {}
        }
    }

    let detached = session.as_ref().is_some_and(FetchSession::running);
    drop(terminal);
    drop(guard);
    if detached {
        // The spec allows detaching but forbids claiming its result. The child
        // may still be running; we never killed it.
        eprintln!("aur-prefetch: fetch detached; pacman may still be running");
    }
    Ok(())
}

/// A refused action or an observation error shown over the static tree. It has
/// no live frontier because no command started.
struct Notice {
    status: String,
    failure: bool,
    /// The gates that ran before the refusal. Empty when the action failed
    /// before any gate, so the checklist never invents a row.
    gates: Vec<Gate>,
}

impl Notice {
    fn from_failure(failure: StartFailure) -> Self {
        let prefix = match &failure.error {
            Error::Blocked(_) => "fetch blocked",
            Error::Failed(_) => "fetch failed",
            Error::Interrupted => "fetch interrupted",
        };
        Self {
            status: format!("{prefix}: {}", failure.error),
            failure: true,
            gates: failure.gates,
        }
    }
}

fn notice_keys(ascii: bool) -> &'static str {
    if ascii {
        "F retry | g ledger | r refresh | q quit | ? help"
    } else {
        "F retry · g ledger · r refresh · q quit · ? help"
    }
}

/// The static tree's own key line. `--render-once` uses `render_frame`
/// directly, so its frozen frame is unchanged.
fn static_keys(ascii: bool, ledger: bool) -> &'static str {
    match (ascii, ledger) {
        (false, false) => "q quit · g ledger · ? help · r refresh",
        (false, true) => "q quit · g tree · ? help · r refresh",
        (true, false) => "q quit | g ledger | ? help | r refresh",
        (true, true) => "q quit | g tree | ? help | r refresh",
    }
}

fn start_fetch(
    config: &Config,
    snapshot: &Snapshot,
    now: SystemTime,
) -> Result<FetchSession, StartFailure> {
    let failure = |error: Error| StartFailure {
        error,
        gates: Vec::new(),
    };
    let Some(report) = snapshot.prefetch.data() else {
        let message = snapshot
            .prefetch
            .error()
            .unwrap_or("prefetch data is not available");
        return Err(failure(Error::failed(message.to_string())));
    };
    let tree =
        Tree::build(&report.packages).map_err(|error| failure(Error::failed(error.to_string())))?;
    FetchSession::start(config, SystemRunner, tree, now)
}

fn collect_snapshot(config: &Config, now: SystemTime) -> Result<Snapshot, Error> {
    let mut snapshot = check::collect(config, &SystemRunner, now)?;
    snapshot.prefetch = progress::collect(config, &SystemRunner, &snapshot, now);
    Ok(snapshot)
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
    use super::{ensure_interactive, use_color};

    #[test]
    fn ascii_and_no_color_both_disable_the_palette() {
        assert!(use_color(false, false));
        assert!(!use_color(false, true));
        assert!(!use_color(true, false));
        assert!(!use_color(true, true));
    }

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
