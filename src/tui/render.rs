//! The T1 tree frame. The data comes from the report layer; this module only
//! lays it out and never shells out.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use std::collections::{BTreeSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::install::{INSTALL_COMMAND, Preflight};
use crate::gate::{Gate, GateState};
use crate::post_install::Receipt;
use crate::report::{ArtifactState, PrefetchReport, Snapshot, Update};
use crate::runner::{StreamKind, StreamLine};
use crate::util::human_bytes;

use super::live::{Activity, LiveRun, LiveState, LiveView, Progress, Window, WindowSlot};
use super::sanitize::{cell_text, display_width};
use super::tree::{RowId, Tree, TreeError, TreeRow, TreeRowKind};

/// The ANSI-16 semantic palette. Color is an accent on top of a glyph and a
/// word; it is never the only signal.
const PLANNED: Color = Color::Blue;
const ACTIVE: Color = Color::Yellow;
const DONE: Color = Color::Green;
const DIM: Color = Color::DarkGray;
const FAILURE: Color = Color::Red;

fn accent(color: bool, value: Color) -> Style {
    if color {
        Style::default().fg(value)
    } else {
        Style::default()
    }
}

fn accent_bold(color: bool, value: Color) -> Style {
    if color {
        Style::default().fg(value).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    }
}

fn dim(color: bool) -> Style {
    if color {
        Style::default().fg(DIM)
    } else {
        Style::default()
    }
}

fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// The optional action line above the key hint.
///
/// The static frame passes [`Footer::default`], so its bytes never change.
/// The live fetch passes a real status and the keys that are legal while the
/// fetch runs or its result is on screen.
#[derive(Debug, Clone, Copy, Default)]
pub struct Footer<'a> {
    pub status: Option<&'a str>,
    pub keys: Option<&'a str>,
    pub failure: bool,
    /// When present, the repair card replaces the action status line. It
    /// carries only observed facts: the failed row, the command evidence and
    /// the blocked count.
    pub repair: Option<&'a RepairCard>,
}

/// A failure summary built from the real exit and the frozen frontier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairCard {
    /// The failed row's label, when one node carries the evidence.
    pub node: Option<String>,
    /// `exit N` plus the first stderr line, exactly as the failed row shows it.
    pub evidence: String,
    /// A suggestion keyed to the evidence, never a claim that it was tried.
    pub repair: String,
    /// Nodes the frozen frontier blocked downstream.
    pub blocked: usize,
    /// An observation error that sits beside the exit; never hidden by the
    /// card.
    pub note: Option<String>,
}

/// The smallest honest repair suggestion for a fetch failure.
///
/// Keyed to the observed stderr text. It never claims a fix ran and never
/// hides the original evidence.
pub fn repair_hint(evidence: &str) -> &'static str {
    let evidence = evidence.to_ascii_lowercase();
    if evidence.contains("no space left") || evidence.contains("disk full") {
        "free disk space, then fetch again"
    } else if evidence.contains("unable to lock database") {
        "wait for the other pacman to finish; never delete the lock file"
    } else if evidence.contains("invalid or corrupted package") || evidence.contains("checksum") {
        "remove the bad .part from the cache, then fetch again"
    } else if evidence.contains("failed retrieving")
        || evidence.contains("failed to synchronise")
        || evidence.contains("could not resolve")
        || evidence.contains("network is unreachable")
    {
        "check the network and the mirror, then fetch again"
    } else {
        "fix the reported error, then fetch again"
    }
}

pub fn render_frame(
    snapshot: &Snapshot,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
    help: bool,
) -> String {
    render_with(
        snapshot,
        None,
        Footer::default(),
        width,
        height,
        ascii,
        color,
        help,
        false,
        None,
    )
}

/// Render a frame that reads the live frontier. The static frame is unchanged;
/// this is the one the fetch loop will use when T2c wires the byte source.
pub fn render_live_frame(
    snapshot: &Snapshot,
    run: &LiveRun,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
    help: bool,
) -> String {
    render_with(
        snapshot,
        Some(run),
        Footer::default(),
        width,
        height,
        ascii,
        color,
        help,
        false,
        None,
    )
}

/// Render one live frame with an action status line. Used by the fetch loop
/// and by the live-fetch goldens. `ledger` flattens the forest rows.
#[allow(clippy::too_many_arguments)] // the goldens read the flat frame inputs
pub fn render_live_frame_status(
    snapshot: &Snapshot,
    run: &LiveRun,
    footer: Footer<'_>,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
    help: bool,
    ledger: bool,
    gates: Option<&[Gate]>,
) -> String {
    render_with(
        snapshot,
        Some(run),
        footer,
        width,
        height,
        ascii,
        color,
        help,
        ledger,
        gates,
    )
}

/// Render the static frame with a status line, for a refused action that has
/// no live run to show.
#[allow(clippy::too_many_arguments)] // the ledger goldens read the flat frame inputs
pub fn render_frame_status(
    snapshot: &Snapshot,
    footer: Footer<'_>,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
    help: bool,
    ledger: bool,
    gates: Option<&[Gate]>,
) -> String {
    render_with(
        snapshot, None, footer, width, height, ascii, color, help, ledger, gates,
    )
}

#[allow(clippy::too_many_arguments)] // one call site per renderer entry point
fn render_with(
    snapshot: &Snapshot,
    run: Option<&LiveRun>,
    footer: Footer<'_>,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
    help: bool,
    ledger: bool,
    gates: Option<&[Gate]>,
) -> String {
    if width == 0 || height == 0 {
        return String::new();
    }

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| {
            draw_with(
                frame, snapshot, run, footer, ascii, color, help, ledger, gates,
            )
        })
        .expect("draw frame");

    buffer_text(terminal.backend().buffer(), width)
}

fn buffer_text(buffer: &Buffer, width: u16) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for (index, cell) in buffer.content().iter().enumerate() {
        line.push_str(cell.symbol());
        if (index + 1) % width as usize == 0 {
            out.push_str(line.trim_end());
            out.push('\n');
            line.clear();
        }
    }
    out
}

/// Draw the captured fetch log over the frame. The newest lines that fit are
/// shown; every line crosses the same untrusted-cell gate as the tree.
pub fn draw_log(frame: &mut Frame, lines: &VecDeque<StreamLine>, ascii: bool, color: bool) {
    let area = frame.area();
    if area.width < 24 || area.height < 6 {
        return;
    }
    let width = area.width.min(100);
    let height = area.height.min(20);
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let block = if ascii {
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::PLAIN)
            .title("fetch log")
    } else {
        Block::default().borders(Borders::ALL).title("fetch log")
    };
    frame.render_widget(block, popup);
    let inner = Rect::new(
        popup.x.saturating_add(1),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    let text = log_lines(
        lines,
        inner.width as usize,
        inner.height as usize,
        ascii,
        color,
    );
    frame.render_widget(Paragraph::new(text), inner);
}

fn log_lines(
    lines: &VecDeque<StreamLine>,
    width: usize,
    height: usize,
    ascii: bool,
    color: bool,
) -> Vec<Line<'static>> {
    if lines.is_empty() {
        return vec![Line::styled(
            cell_text("(no output yet)", width, ascii),
            dim(color),
        )];
    }
    let start = lines.len().saturating_sub(height);
    lines
        .iter()
        .skip(start)
        .map(|line| {
            let style = match line.kind {
                StreamKind::Stdout => Style::default(),
                StreamKind::Stderr => accent(color, FAILURE),
            };
            Line::styled(cell_text(&line.line, width, ascii), style)
        })
        .collect()
}

/// Draw the read-only install gate overlay.
///
/// It shows the checks that actually ran, the exact command the CLI would run,
/// and the session-local news attestation. The action key arrives with the
/// attached-install slice; this overlay never runs `yay`.
pub fn draw_install_gate(
    frame: &mut Frame,
    preflight: &Preflight,
    attested: bool,
    ascii: bool,
    color: bool,
) {
    let area = frame.area();
    if area.width < 40 || area.height < 12 {
        return;
    }
    // The gate is a screen, not a small modal: it covers the frame so no stale
    // footer advertises keys that the modal does not accept.
    frame.render_widget(Clear, area);
    let separator = if ascii { "|" } else { "·" };
    let title = format!(
        " Install {separator} {} ",
        package_count(preflight.official + preflight.aur)
    );
    let block = if ascii {
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::PLAIN)
            .title(title)
    } else {
        Block::default().borders(Borders::ALL).title(title)
    };
    frame.render_widget(block, area);
    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    let text = install_gate_lines(preflight, attested, inner.width as usize, ascii, color);
    frame.render_widget(Paragraph::new(text), inner);
}

fn install_gate_lines(
    preflight: &Preflight,
    attested: bool,
    width: usize,
    ascii: bool,
    color: bool,
) -> Vec<Line<'static>> {
    let separator = if ascii { "|" } else { "·" };
    let mut lines = vec![Line::styled(cell_text("PRE-FLIGHT", width, ascii), bold())];
    for gate in &preflight.gates {
        if gate.name == "news" && attested {
            lines.push(gate_line(
                &Gate::pass("news", "attested for this session"),
                width,
                ascii,
                color,
            ));
        } else {
            lines.push(gate_line(gate, width, ascii, color));
        }
    }
    if !attested {
        lines.push(Line::styled(
            cell_text(
                "  press space to attest after reading the news",
                width,
                ascii,
            ),
            dim(color),
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        cell_text(&format!("COMMAND   {INSTALL_COMMAND}"), width, ascii),
        bold(),
    ));
    let total = preflight.official + preflight.aur;
    if total > 0 {
        let summary = if preflight.aur == 0 {
            format!("{} official update(s).", preflight.official)
        } else {
            format!(
                "{} official + {} AUR. If an AUR package builds now, aurto is stale: read the PKGBUILD.",
                preflight.official, preflight.aur
            )
        };
        lines.push(Line::styled(
            cell_text(&summary, width, ascii),
            Style::default(),
        ));
        lines.push(Line::styled(
            cell_text(
                "Reboot after if the kernel or libraries change.",
                width,
                ascii,
            ),
            Style::default(),
        ));
    }
    lines.push(Line::from(""));
    let keys = if !attested {
        format!("space attest {separator} esc cancel")
    } else if preflight.blocked() {
        format!("a gate blocks install {separator} esc cancel")
    } else {
        format!("enter install {separator} esc cancel")
    };
    lines.push(Line::styled(cell_text(&keys, width, ascii), dim(color)));
    lines
}

/// The install gate in a test backend, for goldens.
pub fn render_install_gate(
    preflight: &Preflight,
    attested: bool,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
) -> String {
    if width == 0 || height == 0 {
        return String::new();
    }
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| draw_install_gate(frame, preflight, attested, ascii, color))
        .expect("draw install gate");
    buffer_text(terminal.backend().buffer(), width)
}

/// Draw the post-install receipt. Read-only: it reports the real exit, the
/// kernel on disk, the `.pacnew` count and the reboot advice, and never edits
/// `/etc`.
pub fn draw_post_install(frame: &mut Frame, receipt: &Receipt, ascii: bool, color: bool) {
    let area = frame.area();
    if area.width < 40 || area.height < 12 {
        return;
    }
    frame.render_widget(Clear, area);
    let separator = if ascii { "|" } else { "·" };
    let mode = if receipt.failed { "failure" } else { "success" };
    let title = format!(" Post-install {separator} {mode} ");
    let block = if ascii {
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::PLAIN)
            .title(title)
    } else {
        Block::default().borders(Borders::ALL).title(title)
    };
    frame.render_widget(block, area);
    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    let text = post_install_lines(receipt, inner.width as usize, ascii, color);
    frame.render_widget(Paragraph::new(text), inner);
}

fn post_install_lines(
    receipt: &Receipt,
    width: usize,
    ascii: bool,
    color: bool,
) -> Vec<Line<'static>> {
    let separator = if ascii { "|" } else { "·" };
    let ok = if ascii { "+" } else { "✓" };
    let fail = if ascii { "x" } else { "✖" };
    let warn = if ascii { "!" } else { "▲" };
    let none = if ascii { "-" } else { "–" };

    let (exit_glyph, exit_style) = if receipt.failed {
        (fail, accent_bold(color, FAILURE))
    } else {
        (ok, accent_bold(color, DONE))
    };
    let exit_text = if receipt.ran {
        format!("yay -Syuu exited {}", receipt.data.exit_code)
    } else {
        match &receipt.reason {
            Some(reason) => format!("yay -Syuu not run: {reason}"),
            None => "yay -Syuu not run".to_string(),
        }
    };
    let mut lines = vec![
        Line::styled(cell_text("POST-INSTALL", width, ascii), bold()),
        Line::styled(
            cell_text(&format!("{exit_glyph}  {exit_text}"), width, ascii),
            exit_style,
        ),
    ];

    let running = if receipt.data.kernel_running.is_empty() {
        "unknown".to_string()
    } else {
        receipt.data.kernel_running.clone()
    };
    let disk = if receipt.data.kernel_disk.is_empty() {
        "unknown".to_string()
    } else {
        receipt.data.kernel_disk.join(", ")
    };
    let kernel = format!("{none}  kernel    running {running} {separator} on disk {disk}");
    lines.push(Line::styled(cell_text(&kernel, width, ascii), dim(color)));

    let pacdiff = if receipt.failed {
        format!("{none}  pacdiff   not checked because the upgrade failed")
    } else if receipt.data.pacdiff_count == 0 {
        format!("{none}  pacdiff   no .pacnew files")
    } else {
        format!(
            "{warn}  pacdiff   {} .pacnew file(s); run sudo pacdiff after closing",
            receipt.data.pacdiff_count
        )
    };
    let pacdiff_style = if !receipt.failed && receipt.data.pacdiff_count > 0 {
        accent_bold(color, PLANNED)
    } else {
        dim(color)
    };
    lines.push(Line::styled(
        cell_text(&pacdiff, width, ascii),
        pacdiff_style,
    ));

    let reboot = if receipt.data.reboot_required {
        format!("{warn}  reboot    recommended: the running kernel is not on disk")
    } else {
        format!("{none}  reboot    not required")
    };
    let reboot_style = if receipt.data.reboot_required {
        accent_bold(color, PLANNED)
    } else {
        dim(color)
    };
    lines.push(Line::styled(cell_text(&reboot, width, ascii), reboot_style));

    if let Some(advice) = crate::post_install::repair_advice(receipt) {
        lines.push(Line::styled(
            cell_text(&format!("repair: {advice}"), width, ascii),
            accent_bold(color, FAILURE),
        ));
    }

    lines.push(Line::from(""));
    lines.push(Line::styled(
        cell_text("enter / esc close", width, ascii),
        dim(color),
    ));
    lines
}

/// The receipt in a test backend, for goldens.
pub fn render_post_install(
    receipt: &Receipt,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
) -> String {
    if width == 0 || height == 0 {
        return String::new();
    }
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| draw_post_install(frame, receipt, ascii, color))
        .expect("draw post-install");
    buffer_text(terminal.backend().buffer(), width)
}

/// The log overlay in a test backend, for goldens and sanitizer tests.
pub fn render_log_frame(
    lines: &VecDeque<StreamLine>,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
) -> String {
    if width == 0 || height == 0 {
        return String::new();
    }
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| draw_log(frame, lines, ascii, color))
        .expect("draw log");
    buffer_text(terminal.backend().buffer(), width)
}

pub fn draw(frame: &mut Frame, snapshot: &Snapshot, ascii: bool, color: bool, help: bool) {
    draw_with(
        frame,
        snapshot,
        None,
        Footer::default(),
        ascii,
        color,
        help,
        false,
        None,
    );
}

pub fn draw_live(
    frame: &mut Frame,
    snapshot: &Snapshot,
    run: &LiveRun,
    ascii: bool,
    color: bool,
    help: bool,
) {
    draw_with(
        frame,
        snapshot,
        Some(run),
        Footer::default(),
        ascii,
        color,
        help,
        false,
        None,
    );
}

#[allow(clippy::too_many_arguments)] // the frame carries the full view state
pub fn draw_live_status(
    frame: &mut Frame,
    snapshot: &Snapshot,
    run: &LiveRun,
    footer: Footer<'_>,
    ascii: bool,
    color: bool,
    help: bool,
    ledger: bool,
    gates: Option<&[Gate]>,
) {
    draw_with(
        frame,
        snapshot,
        Some(run),
        footer,
        ascii,
        color,
        help,
        ledger,
        gates,
    );
}

#[allow(clippy::too_many_arguments)] // the frame carries the full view state
pub fn draw_status(
    frame: &mut Frame,
    snapshot: &Snapshot,
    footer: Footer<'_>,
    ascii: bool,
    color: bool,
    help: bool,
    ledger: bool,
    gates: Option<&[Gate]>,
) {
    draw_with(
        frame, snapshot, None, footer, ascii, color, help, ledger, gates,
    );
}

#[allow(clippy::too_many_arguments)] // the frame carries the full view state
fn draw_with(
    frame: &mut Frame,
    snapshot: &Snapshot,
    run: Option<&LiveRun>,
    footer: Footer<'_>,
    ascii: bool,
    color: bool,
    help: bool,
    ledger: bool,
    gates: Option<&[Gate]>,
) {
    let area = frame.area();
    if area.width < 40 || area.height < 12 {
        let message = if ascii {
            "terminal too small (40x12 minimum)"
        } else {
            "terminal too small (40×12 minimum)"
        };
        frame.render_widget(Paragraph::new(message).alignment(Alignment::Center), area);
        return;
    }

    let footer = footer_lines(footer, area.width as usize, ascii, color);
    // Static and running footers keep the original two rows; the repair card
    // takes only the rows it needs.
    let footer_height = (footer.len() as u16).max(2);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(footer_height),
        ])
        .split(area);

    let tree = snapshot
        .prefetch
        .data()
        .map(|report| Tree::build(&report.packages));
    let live = match (run, tree.as_ref()) {
        (Some(run), Some(Ok(tree))) => Some(LiveView::build(tree, run)),
        _ => None,
    };
    // The body is shared by three blocks: the gates, the tree and the summary.
    // `window` now returns at most the budget it is given, markers included,
    // so the tallies here are exact rather than estimates.
    let gate_rows = gates.map_or(0, |gates| {
        if gates.is_empty() {
            0
        } else {
            gates.len() as u16 + 1
        }
    });
    let body_height = chunks[1].height;
    // Summary first: a frame without it cannot be read. Then the gates, capped
    // so they never take more than half of what is left. The tree gets the
    // remainder, and the bottom-up fill keeps the roots and the goal.
    let after_summary = body_height.saturating_sub(1);
    let gate_cap = if gates.is_some_and(|gates| !gates.is_empty()) {
        (after_summary / 2).min(gate_rows)
    } else {
        0
    };
    let tree_rows = after_summary.saturating_sub(gate_cap).max(1);
    let gate_rows = gate_rows.min(gate_cap);
    let window = match (live.as_ref(), tree.as_ref()) {
        (Some(live), Some(Ok(tree))) => Some(live.window(tree, usize::from(tree_rows))),
        _ => None,
    };

    let header = header_lines(
        snapshot,
        tree.as_ref(),
        live.as_ref(),
        window.as_ref(),
        chunks[0].width as usize,
        ascii,
        color,
        ledger,
    );
    frame.render_widget(Paragraph::new(header), chunks[0]);

    let body = body_lines(
        snapshot,
        tree.as_ref(),
        live.as_ref(),
        window.as_ref(),
        ascii,
        chunks[1].width as usize,
        color,
        ledger,
        gates,
        usize::from(gate_rows),
        usize::from(tree_rows),
    );
    // Last line of defence: the body block may never exceed its chunk. Only
    // clipped markers are dropped here; a marker says "something is hidden"
    // and a summary says how the whole run is going, so the summary wins.
    let mut body = body;
    let budget = usize::from(chunks[1].height);
    // Only a marker that belongs to a run still represented by another marker
    // may go. Dropping the last `⋮` would make the frame claim it shows
    // everything, which is the one thing a clipped frame must not do.
    while body.len() > budget {
        let markers: Vec<usize> = body
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                let text = line.to_string();
                let trimmed = text.trim_start();
                trimmed.starts_with('⋮') || trimmed.starts_with(':')
            })
            .map(|(index, _)| index)
            .collect();
        if markers.len() < 2 {
            break;
        }
        body.remove(markers[0]);
    }
    body.truncate(budget);
    frame.render_widget(Paragraph::new(body), chunks[1]);

    frame.render_widget(Paragraph::new(footer), chunks[2]);

    if help {
        draw_help(frame, area, ascii);
    }
}

fn footer_lines(footer: Footer<'_>, width: usize, ascii: bool, color: bool) -> Vec<Line<'static>> {
    let default_keys = if ascii {
        "q quit | ? help | r refresh"
    } else {
        "q quit · ? help · r refresh"
    };
    let keys = footer.keys.unwrap_or(default_keys);
    let mut lines = Vec::new();
    if let Some(card) = footer.repair {
        lines.extend(repair_lines(card, width, ascii, color));
    } else if let Some(status) = footer.status {
        let style = if footer.failure {
            accent_bold(color, FAILURE)
        } else {
            bold()
        };
        lines.push(Line::styled(cell_text(status, width, ascii), style));
    }
    lines.push(Line::styled(cell_text(keys, width, ascii), dim(color)));
    lines
}

/// Three observed lines above the keys. The hint is a suggestion, not proof
/// that anything was repaired; the evidence stays verbatim.
fn repair_lines(card: &RepairCard, width: usize, ascii: bool, color: bool) -> Vec<Line<'static>> {
    let warning = if ascii { "!" } else { "⚠" };
    let separator = if ascii { "|" } else { "·" };
    let head = match &card.node {
        Some(node) => format!("{warning} {node} {separator} {}", card.evidence),
        None => format!("{warning} {}", card.evidence),
    };
    let blocked = match card.blocked {
        0 => "no downstream blocked".to_string(),
        1 => "1 blocked downstream".to_string(),
        count => format!("{count} blocked downstream"),
    };
    let mut lines = vec![
        Line::styled(cell_text(&head, width, ascii), accent_bold(color, FAILURE)),
        Line::styled(
            cell_text(&format!("repair: {}", card.repair), width, ascii),
            bold(),
        ),
        Line::styled(
            cell_text(
                &format!("{blocked} {separator} no state written"),
                width,
                ascii,
            ),
            dim(color),
        ),
    ];
    if let Some(note) = &card.note {
        lines.push(Line::styled(
            cell_text(&format!("cache read: {note}"), width, ascii),
            dim(color),
        ));
    }
    lines
}

#[allow(clippy::too_many_arguments)] // the frame carries the full view state
fn header_lines(
    snapshot: &Snapshot,
    tree: Option<&Result<Tree, TreeError>>,
    live: Option<&LiveView>,
    window: Option<&Window>,
    width: usize,
    ascii: bool,
    color: bool,
    ledger: bool,
) -> Vec<Line<'static>> {
    let separator = if ascii { "|" } else { "·" };
    let Some(report) = snapshot.prefetch.data() else {
        let pending = pending_count(snapshot);
        let header = format!(
            "aur-prefetch {separator} {pending} pending {separator} {} action(s)",
            snapshot.actionable
        );
        return vec![Line::styled(cell_text(&header, width, ascii), bold())];
    };

    let counts = &report.counts;
    let packages = match tree {
        Some(Ok(tree)) => tree.pending_count(),
        _ => counts.total(),
    };

    let header = match live {
        Some(live) => format!(
            "aur-prefetch {separator} Fetch {separator} {} {separator} live",
            package_count(live.summary.total)
        ),
        None => format!(
            "aur-prefetch {separator} Progress {separator} {} pending {separator} {} prefetched {separator} {} built",
            counts.total(),
            counts.ready(),
            counts.built
        ),
    };

    let (shown_roots, total_roots) = match window {
        Some(window) => (window.shown_roots, window.total_roots),
        None => {
            let roots = match tree {
                Some(Ok(tree)) => tree.roots.len(),
                _ => 0,
            };
            (roots, roots)
        }
    };
    let left = if ledger {
        "Ledger:".to_string()
    } else {
        graph_title(shown_roots, total_roots)
    };
    let right = match live {
        Some(_) => format!("install {} {separator} live", package_count(packages)),
        None => format!(
            "install {} {separator} static prefetch {separator} ghost",
            package_count(packages)
        ),
    };
    // Two halves on one line. When they do not both fit, the left half wins:
    // it names the screen and the root count, while the right half repeats the
    // install total that the header line already carries. Joining them with no
    // gap produced `Dependency Graph showing 2 of 5 roots:i…` at 40 columns.
    let gap = 2;
    let title = if display_width(&left) + gap + display_width(&right) <= width {
        let padding = width - display_width(&left) - display_width(&right);
        format!("{left}{}{right}", " ".repeat(padding))
    } else {
        left
    };

    vec![
        Line::styled(cell_text(&header, width, ascii), bold()),
        Line::styled(cell_text(&title, width, ascii), dim(color)),
    ]
}

/// Nom's graph header. The live viewport reports how many roots it kept; the
/// static frame always shows every root.
fn graph_title(shown: usize, total: usize) -> String {
    if shown >= total {
        "Dependency Graph:".to_string()
    } else {
        format!("Dependency Graph showing {shown} of {total} roots:")
    }
}

fn pending_count(snapshot: &Snapshot) -> usize {
    snapshot
        .official
        .data()
        .map_or(0, Vec::len)
        .saturating_add(snapshot.aur.data().map_or(0, Vec::len))
}

fn updates(snapshot: &Snapshot) -> Vec<&Update> {
    let mut all = Vec::new();
    if let Some(official) = snapshot.official.data() {
        all.extend(official);
    }
    if let Some(aur) = snapshot.aur.data() {
        all.extend(aur);
    }
    all
}

#[derive(Debug, Clone, Copy)]
struct FrameStyle {
    ascii: bool,
    width: usize,
    color: bool,
}

#[allow(clippy::too_many_arguments)] // the frame carries the full view state
fn body_lines(
    snapshot: &Snapshot,
    tree: Option<&Result<Tree, TreeError>>,
    live: Option<&LiveView>,
    window: Option<&Window>,
    ascii: bool,
    width: usize,
    color: bool,
    ledger: bool,
    gates: Option<&[Gate]>,
    gate_room: usize,
    body_budget: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if let Some(error) = &snapshot.fatal {
        lines.push(Line::from(cell_text(
            &format!("error: {error}"),
            width,
            ascii,
        )));
        return lines;
    }

    if let Some(gates) = gates {
        lines.extend(gate_lines(gates, width, ascii, color, gate_room));
    }

    if let Some(error) = snapshot.prefetch.error() {
        lines.push(Line::from(cell_text(
            &format!("prefetch: {error}"),
            width,
            ascii,
        )));
    }

    if let Some(report) = snapshot.prefetch.data() {
        let tree = tree.expect("the prefetch tree is built with the report");
        // The summary and the notes come after the rows and must survive the
        // clip, so they are paid for before the rows are given their budget.
        let tail = 1 + report.notes.len();
        lines.extend(prefetch_lines(
            report,
            tree,
            live,
            window,
            snapshot.prefetch.at(),
            FrameStyle {
                ascii,
                width,
                color,
            },
            ledger,
            body_budget.saturating_sub(tail),
        ));
        return lines;
    }

    if let Some(error) = snapshot.aur.error() {
        lines.push(Line::from(cell_text(
            &format!("aur updates: {error}"),
            width,
            ascii,
        )));
    }

    let all = updates(snapshot);
    if all.is_empty() {
        lines.push(Line::from("system current"));
        return lines;
    }

    lines.push(Line::from(format!("install {}", package_count(all.len()))));
    let branch = if ascii { "|-- " } else { "├── " };
    let last = if ascii { "`-- " } else { "└── " };
    for (index, update) in all.iter().enumerate() {
        let prefix = if index + 1 == all.len() { last } else { branch };
        lines.push(Line::from(cell_text(
            &format!("  {prefix}{}", update.raw),
            width,
            ascii,
        )));
    }
    lines
}

/// The fetch gates, in the order they ran. A later gate after a failure is
/// absent because it never ran; no status is invented for it.
/// The gate block, bounded by `room` rows including its title.
///
/// Gates are pre-flight context and the tree is the screen, so when the body
/// is short the block keeps whole gates and one row saying how many it left
/// out. The overflow row is inside the budget, not extra: a block that overruns
/// by one row is what pushed the summary off the bottom.
fn gate_lines(
    gates: &[Gate],
    width: usize,
    ascii: bool,
    color: bool,
    room: usize,
) -> Vec<Line<'static>> {
    if gates.is_empty() || room == 0 {
        return Vec::new();
    }
    let mut lines = vec![Line::styled(cell_text("PRE-FLIGHT", width, ascii), bold())];
    if room == 1 {
        return lines;
    }
    let fits = (room - 1).min(gates.len());
    let whole = if fits == gates.len() {
        fits
    } else {
        fits.saturating_sub(1)
    };
    lines.extend(
        gates
            .iter()
            .take(whole)
            .map(|gate| gate_line(gate, width, ascii, color)),
    );
    if whole < gates.len() {
        let hidden = gates.len() - whole;
        let more = format!("  {} {hidden} more gate(s)", if ascii { ":" } else { "⋮" });
        lines.push(Line::styled(cell_text(&more, width, ascii), dim(color)));
    }
    lines
}

fn gate_line(gate: &Gate, width: usize, ascii: bool, color: bool) -> Line<'static> {
    let (glyph, style) = match gate.state {
        GateState::Pass => (if ascii { "+" } else { "✓" }, accent_bold(color, DONE)),
        GateState::Warn => (if ascii { "!" } else { "▲" }, accent_bold(color, PLANNED)),
        GateState::Fail => (if ascii { "x" } else { "✗" }, accent_bold(color, FAILURE)),
    };
    let text = format!("{glyph}  {:<8} {}", gate.name, gate.detail);
    Line::styled(cell_text(&text, width, ascii), style)
}

#[allow(clippy::too_many_arguments)] // the frame carries the full view state
fn prefetch_lines(
    report: &PrefetchReport,
    tree: &Result<Tree, TreeError>,
    live: Option<&LiveView>,
    window: Option<&Window>,
    at: Option<SystemTime>,
    style: FrameStyle,
    ledger: bool,
    body_budget: usize,
) -> Vec<Line<'static>> {
    let FrameStyle {
        ascii,
        width,
        color,
    } = style;
    let mut lines = Vec::new();
    let now = at.unwrap_or(UNIX_EPOCH);
    let separator = if ascii { "|" } else { "·" };
    let spinner = live.map_or(0, |live| live.spinner);

    if report.packages.is_empty() {
        lines.push(Line::from("system current"));
    } else {
        match tree {
            Ok(tree) => {
                let display = tree.display_rows();
                // The live path already windowed itself and carries its own
                // clipped markers. The static path has no window, so it keeps
                // the roots and prints how many rows it dropped.
                let static_window = (live.is_none() && window.is_none())
                    .then(|| static_window(&display, body_budget))
                    .flatten();
                let slots: Vec<WindowSlot> = match (window, &static_window) {
                    (Some(window), _) => window.slots.clone(),
                    (None, Some((_, rows))) => rows.iter().copied().map(WindowSlot::Row).collect(),
                    (None, None) => display.iter().map(|row| WindowSlot::Row(row.id)).collect(),
                };
                let geometry = progress_geometry(tree, live, &slots, ascii, width);
                // Live rows draw a 3-cell marker, static rows a 1-cell glyph.
                let lead = solve_lead(
                    tree,
                    &slots,
                    ledger,
                    ascii,
                    if live.is_some() { 3 } else { 1 },
                    width,
                );

                if let Some((hidden, _)) = &static_window {
                    lines.push(hidden_line(*hidden, width, ascii, color));
                }

                for slot in slots {
                    match slot {
                        WindowSlot::Clipped => {
                            lines.push(clipped_line(width, ascii, color));
                        }
                        WindowSlot::Row(id) => {
                            let row = &tree.rows[id];
                            let prefix = if ledger {
                                String::new()
                            } else {
                                branch_prefix(tree, row, ascii)
                            };
                            if row.kind == TreeRowKind::Package {
                                if let Some(live) = live {
                                    if let Some(live_row) = live.row(id) {
                                        match &geometry {
                                            Some(geometry) if geometry.rows.contains(&id) => {
                                                lines.push(progress_row_line(
                                                    row, live_row, &prefix, ascii, width, color,
                                                    geometry, spinner,
                                                ));
                                            }
                                            _ => {
                                                lines.push(live_row_line(
                                                    row, live_row, &prefix, ascii, width, color,
                                                    lead, spinner,
                                                ));
                                            }
                                        }
                                        continue;
                                    }
                                }
                            }
                            lines.push(static_row_line(
                                row, &prefix, now, ascii, width, color, lead,
                            ));
                        }
                    }
                }
            }
            Err(error) => lines.push(Line::from(cell_text(
                &format!("error: {error}"),
                width,
                ascii,
            ))),
        }
    }

    match live {
        Some(live) => lines.extend(live_summary_lines(report, live, ascii, width, color)),
        None => lines.extend(static_summary_lines(report, separator, ascii, width, color)),
    }

    for note in &report.notes {
        lines.push(Line::from(cell_text(
            &format!("note: {note}"),
            width,
            ascii,
        )));
    }
    lines
}

fn static_summary_lines(
    report: &PrefetchReport,
    separator: &str,
    ascii: bool,
    width: usize,
    color: bool,
) -> Vec<Line<'static>> {
    let counts = &report.counts;
    let host = report.host.as_deref().unwrap_or("unknown");
    let text = format!(
        "{} packages {separator} {} ready {separator} {} missing {separator} {} stale {separator} {host}",
        counts.total(),
        counts.ready(),
        counts.missing,
        counts.stale
    );
    vec![Line::styled(cell_text(&text, width, ascii), dim(color))]
}

/// What the prefetch already saved, from the package list alone.
///
/// A cached or built artifact is a package this run will not download. Bytes
/// are summed only over sizes that were actually observed; when some ready
/// package has no known size the total is a floor and says so, rather than
/// reporting a partial sum as a whole.
fn prefetch_saving(report: &PrefetchReport) -> Option<String> {
    let mut ready = 0usize;
    let mut known = 0u64;
    let mut unknown = 0usize;
    for package in &report.packages {
        if !matches!(
            package.artifact,
            ArtifactState::Cached | ArtifactState::Built
        ) {
            continue;
        }
        ready += 1;
        match package.download_bytes {
            Some(bytes) => known = known.saturating_add(bytes),
            None => unknown += 1,
        }
    }
    if ready == 0 {
        return None;
    }
    if known == 0 {
        return Some(format!("{ready} prefetched"));
    }
    let floor = if unknown > 0 { "\u{2265}" } else { "" };
    Some(format!("{ready} prefetched {floor}{}", human_bytes(known)))
}

fn live_summary_lines(
    report: &PrefetchReport,
    live: &LiveView,
    ascii: bool,
    width: usize,
    color: bool,
) -> Vec<Line<'static>> {
    let separator = if ascii { "|" } else { "·" };
    let running = live.summary.builds.running + live.summary.downloads.running;
    let done = live.summary.builds.done + live.summary.downloads.done;
    let waiting = live.summary.builds.planned + live.summary.downloads.planned;
    let host = report.host.as_deref().unwrap_or("unknown");
    let mut text = format!(
        "{} done {separator} {running} running {separator} {waiting} waiting {separator} {} failed",
        done, live.summary.failed
    );
    // Frozen is state, not decoration, so it is placed before anything that
    // may be dropped, and the summary is assembled only from segments that fit
    // whole. A truncated `froze…` would be worse than no savings at all.
    if live.summary.frozen {
        text.push_str(&format!(" {separator} frozen"));
    }
    let mut optional: Vec<String> = Vec::new();
    if let Some(saving) = prefetch_saving(report) {
        optional.push(saving);
    }
    if let Some(timer) = super::live::timer_label(live.summary.elapsed, ascii) {
        optional.push(timer);
    }
    optional.push(host.to_string());
    for segment in optional {
        let candidate = format!("{text} {separator} {segment}");
        if display_width(&candidate) <= width {
            text = candidate;
        }
    }
    let style = if live.summary.failed > 0 || live.summary.frozen {
        accent_bold(color, FAILURE)
    } else {
        dim(color)
    };
    vec![Line::styled(cell_text(&text, width, ascii), style)]
}

/// Nom's bottom-up branch grammar. The last child is the top row and gets the
/// `┌─` corner; earlier children get `├─`; an ancestor that is not its parent's
/// last child keeps a `│` lane. ASCII uses the existing `` `-- `` fallback for
/// the last child and `|--` for the rest. A root has no branch prefix.
fn branch_prefix(tree: &Tree, row: &TreeRow, ascii: bool) -> String {
    if row.depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    let mut ancestors = Vec::new();
    let mut current = row.parent;
    while let Some(id) = current {
        let ancestor = &tree.rows[id];
        if ancestor.depth == 0 {
            break;
        }
        ancestors.push(id);
        current = ancestor.parent;
    }
    ancestors.reverse();
    for id in ancestors {
        let last = tree.rows[id]
            .parent
            .is_some_and(|parent| tree.rows[parent].children.last() == Some(&id));
        prefix.push_str(if last {
            "   "
        } else if ascii {
            "|  "
        } else {
            "│  "
        });
    }

    let Some(parent) = row.parent else {
        return prefix;
    };
    let siblings = &tree.rows[parent].children;
    let position = siblings.iter().position(|id| *id == row.id).unwrap_or(0);
    let last = position + 1 == siblings.len();
    let connector = if ascii {
        if last { "`-- " } else { "|-- " }
    } else if last {
        "┌─ "
    } else {
        "├─ "
    };
    prefix.push_str(connector);
    prefix
}

fn package_count(count: usize) -> String {
    if count == 1 {
        "1 package".to_string()
    } else {
        format!("{count} packages")
    }
}

struct RowPaint<'a> {
    prefix: &'a str,
    prefix_style: Style,
    glyph: &'a str,
    glyph_style: Style,
    name_style: Style,
    suffix: Vec<(String, Style)>,
    size_style: Style,
    /// The widest `prefix + marker` in this frame. Every row pads to it, so the
    /// state and size columns land on the same screen column at every depth.
    lead: usize,
}

/// Solve the shared column widths once per frame.
///
/// Measuring the lead per row was the bug: a depth-2 row has a longer branch
/// prefix, so its name column narrowed and its state and size moved right.
/// bootvine solves its table once and every row shares the answer; this is the
/// same rule with our own cells.
///
/// `marker_width` is what the frame draws between the branch and the label: 3
/// for the live `[ ]` markers, 1 for the static artifact glyphs.
fn solve_lead(
    tree: &Tree,
    slots: &[WindowSlot],
    ledger: bool,
    ascii: bool,
    marker_width: usize,
    width: usize,
) -> usize {
    let mut widest = 0;
    for slot in slots {
        let WindowSlot::Row(id) = *slot else {
            continue;
        };
        let prefix = if ledger {
            String::new()
        } else {
            branch_prefix(tree, &tree.rows[id], ascii)
        };
        widest = widest.max(display_width(&prefix));
    }
    let lead = widest + marker_width + 1;
    lead + (width.saturating_sub(lead) / 3).clamp(12, 36)
}

/// Lay out one tree row against the frame's shared label cell. Every column is
/// measured in terminal cells, and the optional size column is right-aligned
/// exactly as the static frame did.
fn row_line(row: &TreeRow, paint: RowPaint<'_>, ascii: bool, width: usize) -> Line<'static> {
    let own_lead = display_width(paint.prefix) + display_width(paint.glyph) + 1;
    // The label cell is the same width on every row, so the state column and
    // the size column land on one screen column whatever the row's depth. A
    // deeper row has less room for the name itself, and only the name.
    let label_cell = paint.lead.max(own_lead);
    let name_width = label_cell.saturating_sub(own_lead);
    let size_width = if width >= 60 { 8 } else { 0 };
    let state_width =
        width.saturating_sub(label_cell + 1 + if size_width > 0 { size_width + 1 } else { 0 });

    let mut remaining = width;
    let mut spans = Vec::new();
    push_span(
        &mut spans,
        paint.prefix,
        paint.prefix_style,
        &mut remaining,
        ascii,
    );
    push_span(
        &mut spans,
        paint.glyph,
        paint.glyph_style,
        &mut remaining,
        ascii,
    );
    push_span(&mut spans, " ", Style::default(), &mut remaining, ascii);
    // `fit` pads to `name_width`, so this cell always ends on the same column.
    push_span(
        &mut spans,
        &fit(&row.label, name_width, ascii),
        paint.name_style,
        &mut remaining,
        ascii,
    );
    push_span(&mut spans, " ", Style::default(), &mut remaining, ascii);

    let mut suffix_remaining = state_width.min(remaining);
    for (text, style) in paint.suffix {
        push_span(&mut spans, &text, style, &mut suffix_remaining, ascii);
    }
    if suffix_remaining > 0 {
        push_span(
            &mut spans,
            &" ".repeat(suffix_remaining),
            Style::default(),
            &mut remaining,
            ascii,
        );
    }

    if size_width > 0 {
        push_span(&mut spans, " ", Style::default(), &mut remaining, ascii);
        let size = row.size.map_or_else(String::new, human_bytes);
        push_span(
            &mut spans,
            &fit_right(&size, size_width, ascii),
            paint.size_style,
            &mut remaining,
            ascii,
        );
    }

    Line::from(spans)
}

fn static_row_line(
    row: &TreeRow,
    prefix: &str,
    now: SystemTime,
    ascii: bool,
    width: usize,
    color: bool,
    lead: usize,
) -> Line<'static> {
    let separator = if ascii { "|" } else { "·" };
    let artifact = row.artifact.unwrap_or(ArtifactState::Unknown);
    let (glyph, label) = artifact_glyph(artifact, ascii);
    let hue = artifact_color(artifact);
    let mut suffix = vec![(label.to_string(), accent(color, hue))];
    if let Some(detail) = row.detail.as_deref() {
        if !detail.is_empty() && detail != label {
            suffix.push((format!(" {separator} {detail}"), dim(color)));
        }
    }
    if matches!(artifact, ArtifactState::Cached | ArtifactState::Built) {
        if let Some(age) = age_label(row.artifact_at, now) {
            suffix.push((format!(" {separator} {age}"), dim(color)));
        }
    }

    row_line(
        row,
        RowPaint {
            prefix,
            prefix_style: accent(color, PLANNED),
            glyph,
            glyph_style: accent(color, hue),
            name_style: Style::default(),
            suffix,
            size_style: dim(color),
            lead,
        },
        ascii,
        width,
    )
}

#[allow(clippy::too_many_arguments)] // the frame carries the full view state
fn live_row_line(
    row: &TreeRow,
    live: &super::live::LiveRow,
    prefix: &str,
    ascii: bool,
    width: usize,
    color: bool,
    lead: usize,
    spinner: usize,
) -> Line<'static> {
    let separator = if ascii { "|" } else { "·" };
    let glyph = live_glyph_at(live, ascii, spinner);
    let state_label = live_state_label(live);
    let hue = live_color(live.state);
    let name_style = if matches!(live.state, LiveState::Running | LiveState::Failed) {
        accent_bold(color, hue)
    } else {
        accent(color, hue)
    };

    let mut suffix = vec![(state_label.to_string(), accent(color, hue))];
    let annotation = match live.state {
        LiveState::Planned => live.waiting.label(ascii),
        LiveState::Running | LiveState::Done => live.timer(ascii),
        LiveState::Failed => match (live.evidence.as_deref(), live.timer(ascii)) {
            (Some(evidence), Some(timer)) => Some(format!("{evidence} {separator} {timer}")),
            (Some(evidence), None) => Some(evidence.to_string()),
            (None, Some(timer)) => Some(timer),
            (None, None) => None,
        },
        LiveState::Skipped { satisfied: true } => match row.artifact {
            Some(ArtifactState::Cached) => Some("cached".to_string()),
            Some(ArtifactState::Built) => Some("built".to_string()),
            _ => None,
        },
        LiveState::Blocked | LiveState::Skipped { satisfied: false } => None,
    };
    if let Some(annotation) = annotation {
        suffix.push((format!(" {separator} {annotation}"), dim(color)));
    }

    row_line(
        row,
        RowPaint {
            prefix,
            prefix_style: accent(color, PLANNED),
            glyph,
            glyph_style: accent(color, hue),
            name_style,
            suffix,
            size_style: dim(color),
            lead,
        },
        ascii,
        width,
    )
}

/// One live row's marker. A running row spins; everything else is still, so a
/// frame with nothing running never changes on the clock alone.
fn live_glyph_at(live: &super::live::LiveRow, ascii: bool, spinner: usize) -> &'static str {
    match live.state {
        LiveState::Running => match spinner % 4 {
            0 => "[|]",
            1 => "[/]",
            2 => "[-]",
            _ => "[\\]",
        },
        _ => live_glyph(live, ascii),
    }
}

fn live_glyph(live: &super::live::LiveRow, ascii: bool) -> &'static str {
    match live.state {
        LiveState::Planned => "[ ]",
        LiveState::Running => "[|]",
        LiveState::Done => {
            if ascii {
                "[v]"
            } else {
                "[√]"
            }
        }
        LiveState::Failed | LiveState::Blocked | LiveState::Skipped { satisfied: false } => {
            if ascii { "[x]" } else { "[×]" }
        }
        LiveState::Skipped { satisfied: true } => {
            if ascii {
                "[v]"
            } else {
                "[√]"
            }
        }
    }
}

fn live_state_label(live: &super::live::LiveRow) -> &'static str {
    match live.state {
        LiveState::Planned => "planned",
        LiveState::Running => match live.activity {
            Some(Activity::Build) => "building",
            Some(Activity::Download) => "downloading",
            None => "running",
        },
        LiveState::Done => "done",
        LiveState::Failed => "failed",
        LiveState::Blocked => "blocked",
        LiveState::Skipped { .. } => "skipped",
    }
}

fn live_color(state: LiveState) -> Color {
    match state {
        LiveState::Planned => PLANNED,
        LiveState::Running => ACTIVE,
        LiveState::Done => DONE,
        LiveState::Failed => FAILURE,
        LiveState::Blocked | LiveState::Skipped { .. } => DIM,
    }
}

/// The shared bar column for the visible transfer rows.
struct ProgressGeometry {
    left_width: usize,
    bar_width: usize,
    rows: BTreeSet<RowId>,
}

/// Nom's bar geometry across the rows actually on screen:
/// `left_width = max(60, 1 + widest row with progress)` and
/// `bar_width = width - 2 - left_width - 6`. A terminal too narrow for one bar
/// cell draws no bar at all instead of a partial or invented one.
fn progress_geometry(
    tree: &Tree,
    live: Option<&LiveView>,
    slots: &[WindowSlot],
    ascii: bool,
    width: usize,
) -> Option<ProgressGeometry> {
    let live = live?;
    let mut rows = BTreeSet::new();
    let mut widest = 0usize;

    for slot in slots {
        let WindowSlot::Row(id) = *slot else {
            continue;
        };
        let row = &tree.rows[id];
        if row.kind != TreeRowKind::Package {
            continue;
        }
        let Some(live_row) = live.row(id) else {
            continue;
        };
        if !has_visible_bar(live_row) {
            continue;
        }
        let prefix = branch_prefix(tree, row, ascii);
        let text = progress_left_text(row, live_row, &prefix, ascii);
        widest = widest.max(display_width(&text));
        rows.insert(id);
    }

    if rows.is_empty() {
        return None;
    }
    let left_width = 60.max(widest.saturating_add(1));
    let bar_width = width.saturating_sub(2 + left_width + 6);
    if bar_width == 0 {
        return None;
    }
    Some(ProgressGeometry {
        left_width,
        bar_width,
        rows,
    })
}

/// A bar exists only for an observed running download. A build has no byte
/// stream, and a planned or finished node has nothing to animate.
fn has_visible_bar(live: &super::live::LiveRow) -> bool {
    live.state == LiveState::Running
        && live.activity == Some(Activity::Download)
        && live.progress.is_some()
}

fn progress_bytes(progress: Progress) -> String {
    format!(
        "{}/{}",
        human_bytes(progress.done()),
        human_bytes(progress.total())
    )
}

/// The left column of a progress row. This is the same text the styled row
/// draws, used once to measure the shared bar column.
fn progress_left_text(
    row: &TreeRow,
    live: &super::live::LiveRow,
    prefix: &str,
    ascii: bool,
) -> String {
    let separator = if ascii { " | " } else { " · " };
    let progress = live.progress.expect("progress rows have bytes");
    let mut text = format!("{prefix}{} {}", live_glyph(live, ascii), row.label);
    text.push(' ');
    text.push_str(live_state_label(live));
    if let Some(timer) = live.timer(ascii) {
        text.push_str(separator);
        text.push_str(&timer);
    }
    text.push(' ');
    text.push_str(&progress_bytes(progress));
    text
}

/// Nom's transfer row: the readable tree row and bytes on the left, then a
/// fixed right edge with the bar and `%5.1f%%`. The percent is bold; color mode
/// adds the accent. The text is identical with `NO_COLOR`.
#[allow(clippy::too_many_arguments)] // the frame carries the full view state
fn progress_row_line(
    row: &TreeRow,
    live: &super::live::LiveRow,
    prefix: &str,
    ascii: bool,
    width: usize,
    color: bool,
    geometry: &ProgressGeometry,
    spinner: usize,
) -> Line<'static> {
    let separator = if ascii { " | " } else { " · " };
    let glyph = live_glyph_at(live, ascii, spinner);
    let hue = live_color(live.state);
    let name_style = accent_bold(color, hue);
    let progress = live.progress.expect("progress rows have bytes");
    let bytes = progress_bytes(progress);

    let mut spans = Vec::new();
    let mut left_remaining = geometry.left_width;
    push_span(
        &mut spans,
        prefix,
        accent(color, PLANNED),
        &mut left_remaining,
        ascii,
    );
    push_span(
        &mut spans,
        glyph,
        accent_bold(color, hue),
        &mut left_remaining,
        ascii,
    );
    push_span(
        &mut spans,
        " ",
        Style::default(),
        &mut left_remaining,
        ascii,
    );
    push_span(
        &mut spans,
        &row.label,
        name_style,
        &mut left_remaining,
        ascii,
    );
    push_span(
        &mut spans,
        " ",
        Style::default(),
        &mut left_remaining,
        ascii,
    );
    push_span(
        &mut spans,
        live_state_label(live),
        accent(color, hue),
        &mut left_remaining,
        ascii,
    );
    if let Some(timer) = live.timer(ascii) {
        push_span(
            &mut spans,
            separator,
            dim(color),
            &mut left_remaining,
            ascii,
        );
        push_span(&mut spans, &timer, dim(color), &mut left_remaining, ascii);
    }
    push_span(
        &mut spans,
        " ",
        Style::default(),
        &mut left_remaining,
        ascii,
    );
    push_span(
        &mut spans,
        &bytes,
        Style::default(),
        &mut left_remaining,
        ascii,
    );
    if left_remaining > 0 {
        spans.push(Span::raw(" ".repeat(left_remaining)));
    }

    let mut remaining = width.saturating_sub(geometry.left_width);
    push_span(&mut spans, " ", Style::default(), &mut remaining, ascii);

    let (full_glyph, half_glyph, empty_glyph) = if ascii {
        ("#", "-", " ")
    } else {
        ("■", "◧", "□")
    };
    let (full, half, empty) = progress_fill(progress, geometry.bar_width);
    if full > 0 {
        push_span(
            &mut spans,
            &full_glyph.repeat(full),
            accent(color, ACTIVE),
            &mut remaining,
            ascii,
        );
    }
    if half > 0 {
        push_span(
            &mut spans,
            &half_glyph.repeat(half),
            accent(color, ACTIVE),
            &mut remaining,
            ascii,
        );
    }
    if empty > 0 {
        push_span(
            &mut spans,
            &empty_glyph.repeat(empty),
            dim(color),
            &mut remaining,
            ascii,
        );
    }
    push_span(&mut spans, " ", Style::default(), &mut remaining, ascii);
    let percent = format!("{:5.1}%", progress.percent());
    let percent_style = accent_bold(color, ACTIVE);
    push_span(&mut spans, &percent, percent_style, &mut remaining, ascii);

    Line::from(spans)
}

/// Half-cell precision, floored: `done == total` is all full, `done == 0` is
/// all empty, and `done` was already clamped by [`Progress::new`].
fn progress_fill(progress: Progress, bar_width: usize) -> (usize, usize, usize) {
    let units = (2 * bar_width as u128 * progress.done() as u128) / progress.total() as u128;
    let full = (units / 2) as usize;
    let half = (units % 2) as usize;
    (full, half, bar_width.saturating_sub(full + half))
}

fn clipped_line(width: usize, ascii: bool, color: bool) -> Line<'static> {
    let marker = if ascii { "  :" } else { "  ⋮" };
    Line::styled(cell_text(marker, width, ascii), dim(color))
}

/// The rows the static frame must keep when the forest is taller than the body.
///
/// The display order is bottom-up, so the roots and the goal sit at the end.
/// Keeping the tail preserves them and drops the deep children first; the
/// count of what was dropped is printed, so a clipped frame never looks whole.
fn static_window(display: &[&TreeRow], budget: usize) -> Option<(usize, Vec<RowId>)> {
    if display.len() <= budget {
        return None;
    }
    // One row pays for the count marker itself.
    let keep = budget.saturating_sub(1);
    let hidden = display.len() - keep;
    let rows = display[display.len() - keep..]
        .iter()
        .map(|row| row.id)
        .collect();
    Some((hidden, rows))
}

fn hidden_line(hidden: usize, width: usize, ascii: bool, color: bool) -> Line<'static> {
    let marker = if ascii { "  :" } else { "  ⋮" };
    let text = format!("{marker} {hidden} more");
    Line::styled(cell_text(&text, width, ascii), dim(color))
}

fn artifact_color(state: ArtifactState) -> Color {
    match state {
        ArtifactState::Cached | ArtifactState::Built => DONE,
        ArtifactState::Stale | ArtifactState::Partial => ACTIVE,
        ArtifactState::Failed => FAILURE,
        ArtifactState::Missing | ArtifactState::BuildPending => PLANNED,
        ArtifactState::Unknown => DIM,
    }
}

/// Append one sanitized span without ever exceeding `remaining` cells.
fn push_span(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    style: Style,
    remaining: &mut usize,
    ascii: bool,
) {
    if *remaining == 0 || text.is_empty() {
        return;
    }
    let text = cell_text(text, *remaining, ascii);
    let used = display_width(&text);
    if used == 0 {
        return;
    }
    *remaining = remaining.saturating_sub(used);
    spans.push(Span::styled(text, style));
}

fn fit(text: &str, width: usize, ascii: bool) -> String {
    let text = cell_text(text, width, ascii);
    let used = display_width(&text);
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}

fn fit_right(text: &str, width: usize, ascii: bool) -> String {
    let text = cell_text(text, width, ascii);
    let used = display_width(&text);
    format!("{}{text}", " ".repeat(width.saturating_sub(used)))
}

fn artifact_glyph(state: ArtifactState, ascii: bool) -> (&'static str, &'static str) {
    let (unicode, fallback, label) = match state {
        ArtifactState::Cached => ("✓", "+", "cached"),
        ArtifactState::Built => ("◆", "#", "built"),
        ArtifactState::Stale => ("▲", "!", "stale"),
        ArtifactState::Partial => ("◔", "~", "partial"),
        ArtifactState::Missing => ("○", "o", "missing"),
        ArtifactState::BuildPending => ("⏸", "=", "build-pending"),
        ArtifactState::Failed => ("✖", "x", "failed"),
        ArtifactState::Unknown => ("?", "?", "unknown"),
    };
    (if ascii { fallback } else { unicode }, label)
}

fn age_label(at: Option<SystemTime>, now: SystemTime) -> Option<String> {
    let at = at?;
    let seconds = now.duration_since(at).ok()?.as_secs();
    Some(match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    })
}

fn draw_help(frame: &mut Frame, area: Rect, ascii: bool) {
    let width = area.width.min(60);
    let height = area.height.min(12);
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );

    frame.render_widget(Clear, popup);
    let block = if ascii {
        Block::default()
            .borders(Borders::ALL)
            .border_set(border::PLAIN)
            .title("help")
    } else {
        Block::default().borders(Borders::ALL).title("help")
    };
    frame.render_widget(block, popup);

    let inner = Rect::new(
        popup.x.saturating_add(2),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );
    let text = vec![
        Line::from("q       quit"),
        Line::from("? / esc close help"),
        Line::from("F       fetch downloads"),
        Line::from("g       tree/ledger"),
        Line::from("i       install gate"),
        Line::from("l       fetch log"),
        Line::from("r       refresh"),
        Line::from("ctrl-c  quit with 130"),
        Line::from("ctrl-z  suspend"),
    ];
    frame.render_widget(Paragraph::new(text), inner);
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};

    use super::{
        Footer, RepairCard, display_width, render_frame, render_install_gate, render_live_frame,
        render_live_frame_status, render_log_frame, render_post_install, repair_hint,
    };
    use crate::commands::install::Preflight;
    use crate::frontier::Observed;
    use crate::gate::Gate;
    use crate::post_install::Receipt;
    use crate::report::PostInstall;
    use crate::report::{
        ArtifactState, Cached, PackageView, PrefetchReport, Repo, Section, Snapshot, Update,
    };
    use crate::runner::{StreamKind, StreamLine};
    use crate::tui::live::{Activity, LiveRun};
    use crate::tui::tree::Tree;
    use std::collections::VecDeque;

    fn at() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_760_000_000)
    }

    fn update(raw: &str) -> Update {
        let (left, available) = raw.split_once(" -> ").unwrap_or((raw, ""));
        let (name, installed) = left.split_once(' ').unwrap_or((left, ""));
        Update {
            name: name.to_string(),
            repo: Repo::Unknown,
            installed: installed.to_string(),
            available: available.to_string(),
            download_bytes: None,
            cached: Cached::Unknown,
            raw: raw.to_string(),
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            official: Section::ready(
                vec![
                    update("bash 5.2.037-1 -> 5.2.037-2"),
                    update("linux 6.11.5.arch1-1 -> 6.11.6.arch1-1"),
                ],
                at(),
            ),
            aur: Section::ready(vec![update("yay-bin 12.4.0-1 -> 12.4.1-1")], at()),
            actionable: 2,
            ..Snapshot::default()
        }
    }

    fn prefetch_package(
        name: &str,
        available: &str,
        artifact: ArtifactState,
        detail: &str,
        artifact_at: SystemTime,
        depends: &[&str],
    ) -> PackageView {
        PackageView {
            name: name.to_string(),
            repo: Repo::Unknown,
            installed: "1.0.0-1".to_string(),
            available: available.to_string(),
            download_bytes: Some(1_200_000),
            artifact,
            artifact_at: Some(artifact_at),
            detail: Some(detail.to_string()),
            depends: depends.iter().map(|dep| (*dep).to_string()).collect(),
        }
    }

    fn prefetch_snapshot_with(packages: Vec<PackageView>) -> Snapshot {
        let now = at();
        let report =
            PrefetchReport::new(packages, Some("mirror.example.org".to_string()), Vec::new());
        Snapshot {
            prefetch: Section::ready(report, now),
            actionable: 1,
            ..Snapshot::default()
        }
    }

    fn prefetch_snapshot() -> Snapshot {
        let now = at();
        prefetch_snapshot_with(vec![
            prefetch_package(
                "bash",
                "5.2.037-2",
                ArtifactState::Cached,
                "cached",
                now - Duration::from_secs(240),
                &["glibc"],
            ),
            prefetch_package(
                "linux",
                "6.17.2.arch1-1",
                ArtifactState::Stale,
                "older cache file 6.16.1-1",
                now - Duration::from_secs(2 * 86_400),
                &["bash", "yay-bin"],
            ),
            prefetch_package(
                "yay-bin",
                "12.4.1-1",
                ArtifactState::BuildPending,
                "build pending",
                now,
                &[],
            ),
            prefetch_package(
                "simapp",
                "2.0.0-1",
                ArtifactState::Missing,
                "not in cache",
                now,
                &[],
            ),
        ])
    }

    fn live_at() -> SystemTime {
        at()
    }

    fn live_package(
        name: &str,
        repo: Repo,
        artifact: ArtifactState,
        depends: &[&str],
    ) -> PackageView {
        PackageView {
            name: name.to_string(),
            repo,
            installed: "1.0.0-1".to_string(),
            available: "2.0.0-1".to_string(),
            download_bytes: Some(1_200_000),
            artifact,
            artifact_at: Some(live_at()),
            detail: None,
            depends: depends.iter().map(|dep| (*dep).to_string()).collect(),
        }
    }

    fn tree_of(snapshot: &Snapshot) -> Tree {
        Tree::build(&snapshot.prefetch.data().expect("prefetch section").packages)
            .expect("forest builds")
    }

    /// A forest taller than the body must keep its summary and say how many
    /// rows it dropped. Before this, 20 packages silently clipped the summary
    /// line and printed no overflow marker at all.
    fn wide_snapshot(count: usize) -> Snapshot {
        let now = at();
        prefetch_snapshot_with(
            (0..count)
                .map(|index| {
                    let depends: Vec<String> = if index % 5 == 0 {
                        Vec::new()
                    } else {
                        vec![format!("pkg-{:04}", index - 1)]
                    };
                    PackageView {
                        name: format!("pkg-{index:04}"),
                        repo: Repo::Extra,
                        installed: "1.0.0-1".to_string(),
                        available: "2.0.0-1".to_string(),
                        download_bytes: Some(1_100_000),
                        artifact: ArtifactState::Missing,
                        artifact_at: Some(now),
                        detail: None,
                        depends,
                    }
                })
                .collect(),
        )
    }

    #[test]
    fn a_tall_forest_keeps_its_summary_and_counts_what_it_hides() {
        let short = render_frame(&wide_snapshot(18), 80, 24, false, false, false);
        assert!(
            short.contains("mirror.example.org"),
            "a short forest keeps the summary"
        );
        assert!(
            !short.lines().any(|line| line.contains(" more")),
            "a forest that fits needs no overflow row:\n{short}"
        );

        for (count, width, height) in [(20, 80, 24), (500, 80, 24), (2_000, 120, 40)] {
            let frame = render_frame(&wide_snapshot(count), width, height, false, false, false);
            assert!(
                frame.contains("mirror.example.org"),
                "{count} packages at {width}x{height} lost the summary:\n{frame}"
            );
            let overflow = frame
                .lines()
                .find(|line| line.contains(" more"))
                .unwrap_or_else(|| panic!("{count} packages hid rows without saying so:\n{frame}"));
            let rows = frame.lines().filter(|line| line.contains("pkg-")).count();
            let hidden: usize = overflow
                .split_whitespace()
                .find_map(|word| word.parse().ok())
                .expect("the overflow row carries a count");
            assert_eq!(
                rows + hidden,
                count,
                "every package is either printed or counted:\n{frame}"
            );
        }

        // The tail of the bottom-up order is the roots and the goal.
        let frame = render_frame(&wide_snapshot(500), 80, 24, false, false, false);
        assert!(
            frame.contains("pkg-0000") || frame.contains("pkg-0004"),
            "a root survives the clip:\n{frame}"
        );

        let ascii = render_frame(&wide_snapshot(500), 80, 24, true, false, false);
        assert!(
            ascii.lines().all(|line| line.is_ascii()),
            "ASCII stays ASCII"
        );
        assert!(ascii.contains(" more"), "ASCII keeps the meaning:\n{ascii}");
    }

    fn diamond_snapshot() -> Snapshot {
        prefetch_snapshot_with(vec![
            live_package("base", Repo::Extra, ArtifactState::Missing, &[]),
            live_package("left", Repo::Aur, ArtifactState::Missing, &["base"]),
            live_package("right", Repo::Extra, ArtifactState::Missing, &["base"]),
            live_package(
                "join",
                Repo::Aur,
                ArtifactState::Missing,
                &["left", "right"],
            ),
        ])
    }

    fn diamond_run() -> LiveRun {
        let tree = tree_of(&diamond_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe(
            "base",
            Observed::Finished,
            live_at() + Duration::from_millis(800),
            None,
            "exit 0",
        )
        .expect("finishes");
        run.observe(
            "left",
            Observed::Started,
            live_at() + Duration::from_millis(800),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        // Advance the observation clock without moving a node.
        run.observe(
            "right",
            Observed::EdgeResolved,
            live_at() + Duration::from_millis(2_800),
            None,
            "edge recheck",
        )
        .expect("rechecks");
        run
    }

    fn subsecond_run() -> LiveRun {
        let tree = tree_of(&diamond_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe(
            "base",
            Observed::EdgeResolved,
            live_at() + Duration::from_millis(500),
            None,
            "poll",
        )
        .expect("rechecks");
        run
    }

    fn roots_snapshot() -> Snapshot {
        prefetch_snapshot_with(vec![
            live_package("root-a", Repo::Extra, ArtifactState::Missing, &[]),
            live_package("child-a", Repo::Extra, ArtifactState::Missing, &["root-a"]),
            live_package("root-b", Repo::Aur, ArtifactState::Missing, &[]),
            live_package("child-b", Repo::Aur, ArtifactState::Missing, &["root-b"]),
        ])
    }

    fn roots_run() -> LiveRun {
        let tree = tree_of(&roots_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        run.observe(
            "root-a",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe(
            "root-b",
            Observed::Started,
            live_at(),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        run.observe(
            "child-a",
            Observed::EdgeResolved,
            live_at() + Duration::from_secs(2),
            None,
            "edge recheck",
        )
        .expect("rechecks");
        run
    }

    fn failure_snapshot() -> Snapshot {
        prefetch_snapshot_with(vec![
            live_package("base", Repo::Aur, ArtifactState::Missing, &[]),
            live_package("left", Repo::Extra, ArtifactState::Missing, &["base"]),
            live_package("right", Repo::Extra, ArtifactState::Missing, &["base"]),
            live_package(
                "join",
                Repo::Extra,
                ArtifactState::Missing,
                &["left", "right"],
            ),
            live_package("solo", Repo::Extra, ArtifactState::Missing, &[]),
        ])
    }

    fn failure_run() -> LiveRun {
        let tree = tree_of(&failure_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        run.observe(
            "base",
            Observed::Started,
            live_at(),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        run.observe(
            "solo",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe(
            "base",
            Observed::Failed,
            live_at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("fails");
        run
    }

    fn skipped_snapshot() -> Snapshot {
        prefetch_snapshot_with(vec![live_package(
            "cached-pkg",
            Repo::Extra,
            ArtifactState::Cached,
            &[],
        )])
    }

    fn skipped_run() -> LiveRun {
        let tree = tree_of(&skipped_snapshot());
        LiveRun::plan(&tree, live_at()).expect("plan seeds")
    }

    fn progress_snapshot() -> Snapshot {
        let mut package = live_package("linux", Repo::Extra, ArtifactState::Missing, &[]);
        package.download_bytes = Some(59_800_000);
        prefetch_snapshot_with(vec![package])
    }

    fn progress_run() -> LiveRun {
        let tree = tree_of(&progress_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        run.observe(
            "linux",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe_progress(
            "linux",
            49_000_000,
            59_800_000,
            live_at() + Duration::from_secs(6),
        )
        .expect("progress records");
        run
    }

    fn unknown_progress_run() -> LiveRun {
        let tree = tree_of(&progress_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        run.observe(
            "linux",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe_progress("linux", 49_000_000, 0, live_at() + Duration::from_secs(6))
            .expect("an unknown total is still an observation");
        run
    }

    fn fetch_run() -> LiveRun {
        let tree = tree_of(&progress_snapshot());
        let mut run = LiveRun::plan_concurrent(&tree, live_at()).expect("plan seeds");
        run.observe(
            "linux",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "partial linux-2.0.0-1-x86_64.pkg.tar.zst.part",
        )
        .expect("starts");
        run.observe_progress(
            "linux",
            49_000_000,
            59_800_000,
            live_at() + Duration::from_secs(6),
        )
        .expect("progress records");
        run
    }

    fn fetch_footer() -> Footer<'static> {
        Footer {
            status: Some("fetch: running sudo -n /usr/bin/pacman -Syuw --noconfirm"),
            keys: Some("esc detach · g ledger · ? help"),
            failure: false,
            repair: None,
        }
    }

    fn repair_card() -> RepairCard {
        RepairCard {
            node: Some("base 2.0.0-1".to_string()),
            evidence: "exit 2: error: target not found: go".to_string(),
            repair: repair_hint("exit 2: error: target not found: go").to_string(),
            blocked: 3,
            note: None,
        }
    }

    fn repair_footer(card: &RepairCard) -> Footer<'_> {
        Footer {
            status: Some("fetch failed: exit 2: error: target not found: go"),
            keys: Some("esc dismiss · F fetch again · g ledger · q quit · ? help"),
            failure: true,
            repair: Some(card),
        }
    }

    fn ledger_footer() -> Footer<'static> {
        Footer {
            status: None,
            keys: Some("q quit · g tree · ? help · r refresh"),
            failure: false,
            repair: None,
        }
    }

    fn fetch_gates() -> Vec<Gate> {
        vec![
            Gate::pass("lock", "no lock file, no pacman process"),
            Gate::pass("lists", "1 pending, 825 B to download"),
            Gate::pass("cache", "/var/cache/pacman/pkg"),
            Gate::pass(
                "space",
                "/var/cache/pacman/pkg: need 5.0 GiB, have 14.2 GiB",
            ),
            Gate::pass("space", "/: need 5.0 GiB, have 40.1 GiB"),
        ]
    }

    fn log_fixture() -> VecDeque<StreamLine> {
        let mut lines = VecDeque::new();
        for (kind, line) in [
            (StreamKind::Stdout, ":: Synchronizing package databases..."),
            (StreamKind::Stdout, ":: Starting full system upgrade..."),
            (StreamKind::Stderr, "error: failed retrieving file 'x'"),
            (
                StreamKind::Stdout,
                "\x1b[31mdownloading\x1b[0m\tsimapp-2.0.0-1...",
            ),
        ] {
            lines.push_back(StreamLine {
                kind,
                line: line.to_string(),
            });
        }
        lines
    }

    fn success_receipt() -> Receipt {
        Receipt {
            data: PostInstall {
                exit_code: 0,
                kernel_running: "6.2".to_string(),
                kernel_disk: vec!["6.1".to_string(), "6.2".to_string()],
                pacdiff_count: 2,
                reboot_required: false,
                news_url: "https://archlinux.org/news/".to_string(),
            },
            failed: false,
            disk_full: false,
            ran: true,
            reason: None,
        }
    }

    fn failure_receipt() -> Receipt {
        Receipt {
            data: PostInstall {
                exit_code: 1,
                kernel_running: "6.1".to_string(),
                kernel_disk: vec!["6.2".to_string()],
                pacdiff_count: 0,
                reboot_required: true,
                news_url: "https://archlinux.org/news/".to_string(),
            },
            failed: true,
            disk_full: true,
            ran: true,
            reason: None,
        }
    }

    fn install_preflight() -> Preflight {
        Preflight {
            gates: vec![
                Gate::pass("root", "running as the user, not root"),
                Gate::pass("lock", "no lock file, no pacman process"),
                Gate::pass("lists", "1 official, 0 AUR update(s) pending"),
                Gate::pass("trust", "allowlist · 2 maintainer(s)"),
                Gate::warn(
                    "news",
                    "cannot be verified; read https://archlinux.org/news/ since 2026-11-01T00:00:00+0000",
                ),
            ],
            official: 1,
            aur: 0,
            news_since: "2026-11-01T00:00:00+0000".to_string(),
            euid: Some(1000),
        }
    }

    fn long_log_fixture() -> VecDeque<StreamLine> {
        let mut lines = log_fixture();
        for index in 0..100 {
            lines.push_back(StreamLine {
                kind: StreamKind::Stdout,
                line: format!("line {index}"),
            });
        }
        lines
    }

    /// Exactly 200 packages: 195 independent roots plus a done/running chain
    /// and a failed/blocked chain. The window cannot show them all.
    fn window_snapshot() -> Snapshot {
        let mut packages = Vec::new();
        for index in 0..195 {
            packages.push(live_package(
                &format!("pkg-{index:03}"),
                Repo::Extra,
                ArtifactState::Missing,
                &[],
            ));
        }
        packages.push(live_package(
            "a-base",
            Repo::Extra,
            ArtifactState::Missing,
            &[],
        ));
        packages.push(live_package(
            "a-top",
            Repo::Extra,
            ArtifactState::Missing,
            &["a-base"],
        ));
        packages.push(live_package(
            "z-base",
            Repo::Extra,
            ArtifactState::Missing,
            &[],
        ));
        packages.push(live_package(
            "z-mid",
            Repo::Extra,
            ArtifactState::Missing,
            &["z-base"],
        ));
        packages.push(live_package(
            "z-top",
            Repo::Extra,
            ArtifactState::Missing,
            &["z-mid"],
        ));
        assert_eq!(packages.len(), 200);
        prefetch_snapshot_with(packages)
    }

    fn window_run() -> LiveRun {
        let tree = tree_of(&window_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        for (node, kind, activity, evidence) in [
            (
                "z-base",
                Observed::Started,
                Some(Activity::Download),
                "started",
            ),
            ("z-base", Observed::Finished, None, "exit 0"),
            (
                "z-mid",
                Observed::Started,
                Some(Activity::Download),
                "started",
            ),
            (
                "a-base",
                Observed::Started,
                Some(Activity::Download),
                "started",
            ),
        ] {
            run.observe(node, kind, live_at(), activity, evidence)
                .expect("observation applies");
        }
        run.observe(
            "a-base",
            Observed::Failed,
            live_at() + Duration::from_secs(2),
            None,
            "exit 2",
        )
        .expect("the failure freezes the run");
        run
    }

    /// One frame carrying every feature the live screen has.
    ///
    /// The topology is chosen so each state is reachable, not just present in
    /// the snapshot: the cached roots seed `NoWork`, which makes their
    /// dependents Ready, which is the only way a start and a failure apply.
    /// `nvidia` is Ready (its only prerequisite is cached), so it can fail;
    /// `simapp` depends on it, so the freeze blocks it.
    fn showcase_snapshot() -> Snapshot {
        let mut packages = vec![
            live_package("glibc", Repo::Extra, ArtifactState::Cached, &[]),
            live_package("lib32-glibc", Repo::Multilib, ArtifactState::Cached, &[]),
            live_package("linux", Repo::Extra, ArtifactState::Missing, &["glibc"]),
            live_package("yay-bin", Repo::Aur, ArtifactState::BuildPending, &[]),
            live_package("paru", Repo::Aur, ArtifactState::Missing, &["yay-bin"]),
            live_package("ollama", Repo::Aur, ArtifactState::Missing, &["yay-bin"]),
            live_package("nvidia", Repo::Extra, ArtifactState::Missing, &["glibc"]),
            live_package("simapp", Repo::Local, ArtifactState::Stale, &["nvidia"]),
        ];
        // The download size the bar divides against, from the progress fixture.
        for package in &mut packages {
            if package.name == "linux" {
                package.download_bytes = Some(59_800_000);
            }
        }
        prefetch_snapshot_with(packages)
    }

    fn showcase_run() -> LiveRun {
        let tree = tree_of(&showcase_snapshot());
        let mut run = LiveRun::plan(&tree, live_at()).expect("plan seeds");
        run.observe(
            "linux",
            Observed::Started,
            live_at(),
            Some(Activity::Download),
            "started",
        )
        .expect("starts");
        run.observe(
            "yay-bin",
            Observed::Started,
            live_at() + Duration::from_secs(1),
            Some(Activity::Build),
            "started",
        )
        .expect("starts");
        // The bytes are observed while the run can still accept them. A freeze
        // stops byte polling, so this must come before the failure.
        run.observe_progress(
            "linux",
            49_000_000,
            59_800_000,
            live_at() + Duration::from_secs(6),
        )
        .expect("real bytes");
        let sequence: [(&str, Observed, Option<Activity>, &str); 3] = [
            (
                "nvidia",
                Observed::Started,
                Some(Activity::Build),
                "started",
            ),
            (
                "nvidia",
                Observed::Failed,
                None,
                "exit 2: error: target not found: go",
            ),
            ("lib32-glibc", Observed::Finished, None, "exit 0"),
        ];
        for (index, (node, kind, activity, evidence)) in sequence.into_iter().enumerate() {
            let at = live_at() + Duration::from_secs(6 + index as u64);
            run.observe(node, kind, at, activity, evidence)
                .expect("observation applies");
        }
        run
    }

    fn showcase_footer(card: &RepairCard, ascii: bool) -> Footer<'_> {
        let keys = if ascii {
            "esc dismiss | F fetch again | g ledger | l log | ? help | q quit"
        } else {
            "esc dismiss · F fetch again · g ledger · l log · ? help · q quit"
        };
        Footer {
            status: Some("fetch: exit 2 after 6s"),
            keys: Some(keys),
            failure: true,
            repair: Some(card),
        }
    }

    fn showcase_card() -> RepairCard {
        RepairCard {
            node: Some("nvidia 2.0.0-1".to_string()),
            evidence: "exit 2: error: target not found: go".to_string(),
            repair: repair_hint("exit 2: error: target not found: go").to_string(),
            blocked: 1,
            note: None,
        }
    }

    /// The frame-dump harness: every screen at every harness size, as plain
    /// text, so layout can be judged from files instead of from images.
    ///
    /// `cargo test dump_frames -- --ignored` writes `target/frames/`. Those
    /// files are a diagnostic; the goldens are the contract.
    #[test]
    #[ignore = "writes target/frames for visual review"]
    fn dump_frames() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/frames");
        let card = showcase_card();
        let gates = fetch_gates();
        let cases: [(&str, Snapshot, LiveRun, bool); 4] = [
            ("showcase", showcase_snapshot(), showcase_run(), false),
            ("diamond", diamond_snapshot(), diamond_run(), false),
            ("failure", failure_snapshot(), failure_run(), true),
            ("progress", progress_snapshot(), progress_run(), false),
        ];
        for (name, snapshot, run, failing) in &cases {
            let case = dir.join(name);
            std::fs::create_dir_all(&case).expect("case dir");
            for (width, height) in [(80u16, 24u16), (120, 40), (40, 15)] {
                for ascii in [false, true] {
                    let footer = if *failing {
                        showcase_footer(&card, ascii)
                    } else {
                        Footer::default()
                    };
                    let frame = render_live_frame_status(
                        snapshot,
                        run,
                        footer,
                        width,
                        height,
                        ascii,
                        false,
                        false,
                        false,
                        Some(&gates),
                    );
                    let tint = if ascii { "ascii" } else { "color" };
                    std::fs::write(case.join(format!("{width}x{height}-{tint}.txt")), &frame)
                        .expect("frame");
                }
            }
        }
        println!("wrote {}", dir.display());
    }

    #[test]
    fn every_feature_is_visible_at_once() {
        let snapshot = showcase_snapshot();
        let run = showcase_run();
        let card = showcase_card();
        let gates = fetch_gates();
        let frame = render_live_frame_status(
            &snapshot,
            &run,
            showcase_footer(&card, false),
            120,
            40,
            false,
            false,
            false,
            false,
            Some(&gates),
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-showcase-120x40.txt"),
            "the full-feature frame is stable"
        );

        // The same fixture at 80x24. This is the size that decides what a
        // narrow frame may drop: the markers, the state words and the repair
        // card must survive, and nothing may be truncated silently.
        let narrow = render_live_frame_status(
            &snapshot,
            &run,
            showcase_footer(&card, false),
            80,
            24,
            false,
            false,
            false,
            false,
            Some(&gates),
        );
        assert_eq!(
            narrow,
            include_str!("../../tests/golden/tui-showcase-80x24.txt"),
            "the narrow full-feature frame is stable"
        );
        for needle in ["[|]", "[√]", "[×]", "repair: ", "no state written"] {
            assert!(
                narrow.contains(needle),
                "{needle:?} must survive at 80x24:\n{narrow}"
            );
        }
        // The viewport contract is explicit: running, failed and blocked rows
        // are always kept. Everything else is kept while it fits. At this size
        // all eight packages fit, so the frame is honest with no marker at
        // all; a marker is only owed when a run is actually hidden.
        let hidden_runs = narrow.matches('⋮').count();
        let shown_packages = narrow
            .lines()
            .filter(|line| line.starts_with("[") || line.contains("─ ["))
            .count();
        assert_eq!(
            hidden_runs, 0,
            "all eight packages fit, so nothing may claim to be hidden:\n{narrow}"
        );
        assert_eq!(shown_packages, 8, "every package is on screen:\n{narrow}");
        // With every package on screen there is nothing to report, so the
        // plain title is correct; the clipped form is asserted where the
        // window really clips (`a_200_package_window_...`).
        assert!(
            narrow.contains("Dependency Graph:"),
            "the header names the graph:\n{narrow}"
        );
        assert!(
            !narrow.contains('\u{fffd}'),
            "no replacement character at 80x24:\n{narrow}"
        );

        // Every state the view can draw, in one frame.
        for needle in [
            "[ ]",         // planned
            "[|]",         // running
            "[√]",         // done
            "[×]",         // failed
            "downloading", // activity kind
            "building",    // activity kind
            "waiting for", // prerequisite counts
            "⏱",           // a timer above the floor
            "MiB/",        // both observed sizes
            "81.9%",       // the only percent either size can produce
            "PRE-FLIGHT",  // the gate checklist
            "cached",      // a satisfied artifact
            "frozen",      // the frontier stopped
        ] {
            assert!(frame.contains(needle), "{needle:?} missing:\n{frame}");
        }
        // The blocker is dim and says so; the dependents are blocked, not failed.
        assert!(
            frame
                .lines()
                .any(|line| line.contains("[×]") && line.contains("blocked")),
            "a blocked dependent is drawn:\n{frame}"
        );
        // The card names the failure, the hint and what it blocked.
        assert!(
            frame.contains("nvidia"),
            "the card names the node:\n{frame}"
        );
        assert!(frame.contains("repair: "), "the hint is shown:\n{frame}");
        assert!(
            frame.contains("1 blocked downstream"),
            "the blocked count is real:\n{frame}"
        );
        assert!(frame.contains("no state written"), "the write is stated");
        assert!(frame.contains("✓"), "the gates passed with a mark");

        // ASCII carries the same meaning with a different glyph set.
        let ascii = render_live_frame_status(
            &snapshot,
            &run,
            showcase_footer(&card, true),
            120,
            40,
            true,
            false,
            false,
            false,
            Some(&gates),
        );
        assert!(
            ascii.lines().all(|line| line.is_ascii()),
            "ASCII stays ASCII"
        );
        // ASCII swaps the whole marker set, not just the bars.
        for needle in ["[v]", "[|]", "[x]", "[ ]", "81.9%", "repair: ", "+  lock"] {
            assert!(
                ascii.contains(needle),
                "{needle:?} missing from ASCII:\n{ascii}"
            );
        }
        assert!(
            !ascii.contains("[√]") && !ascii.contains("✓"),
            "ASCII does not borrow the Unicode marks:\n{ascii}"
        );
        assert!(
            !ascii.contains("█") && !ascii.contains("■"),
            "ASCII bars use ASCII cells:\n{ascii}"
        );
    }

    fn buffer_of(
        snapshot: &Snapshot,
        run: Option<&LiveRun>,
        width: u16,
        height: u16,
        ascii: bool,
        color: bool,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| match run {
                Some(run) => super::draw_live(frame, snapshot, run, ascii, color, false),
                None => super::draw(frame, snapshot, ascii, color, false),
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &ratatui::buffer::Buffer, width: u16, row: usize) -> String {
        let start = row * width as usize;
        buffer.content()[start..start + width as usize]
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn find_row(buffer: &ratatui::buffer::Buffer, width: u16, needle: &str) -> usize {
        (0..buffer.area.height as usize)
            .find(|row| row_text(buffer, width, *row).contains(needle))
            .unwrap_or_else(|| panic!("{needle} is on screen"))
    }

    fn cell_at(
        buffer: &ratatui::buffer::Buffer,
        width: u16,
        row: usize,
        column: usize,
    ) -> ratatui::buffer::Cell {
        buffer.content()[row * width as usize + column].clone()
    }

    /// The cell column of `needle`, not its byte offset. Every symbol in this
    /// frame is exactly one cell, so counting characters is exact.
    fn text_column(
        buffer: &ratatui::buffer::Buffer,
        width: u16,
        row: usize,
        needle: &str,
    ) -> Option<usize> {
        let text = row_text(buffer, width, row);
        let byte = text.find(needle)?;
        Some(text[..byte].chars().count())
    }

    #[test]
    fn live_diamond_frame_is_stable_at_120x40() {
        let snapshot = diamond_snapshot();
        let run = diamond_run();
        assert_eq!(
            render_live_frame(&snapshot, &run, 120, 40, false, false, false),
            include_str!("../../tests/golden/tui-live-diamond-120x40.txt")
        );
    }

    #[test]
    fn live_diamond_frame_is_stable_at_80x24() {
        let snapshot = diamond_snapshot();
        let run = diamond_run();
        assert_eq!(
            render_live_frame(&snapshot, &run, 80, 24, false, false, false),
            include_str!("../../tests/golden/tui-live-diamond-80x24.txt")
        );
    }

    #[test]
    fn live_diamond_ascii_frame_is_stable() {
        let snapshot = diamond_snapshot();
        let run = diamond_run();
        let frame = render_live_frame(&snapshot, &run, 80, 24, true, false, false);
        assert!(
            frame.is_ascii(),
            "ascii live frame had non-ASCII bytes\n{frame}"
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-live-diamond-ascii-80x24.txt")
        );
    }

    #[test]
    fn live_timer_stays_hidden_below_one_second() {
        let snapshot = diamond_snapshot();
        let run = subsecond_run();
        let frame = render_live_frame(&snapshot, &run, 80, 24, false, false, false);
        assert!(!frame.contains("⏱"), "no timer under one second\n{frame}");
        assert!(!frame.contains("0s"), "no zero-second timer\n{frame}");
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-live-subsecond-80x24.txt")
        );
    }

    #[test]
    fn live_two_roots_frame_is_stable_at_120x40() {
        let snapshot = roots_snapshot();
        let run = roots_run();
        let frame = render_live_frame(&snapshot, &run, 120, 40, false, false, false);
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-live-roots-120x40.txt")
        );
    }

    #[test]
    fn live_two_roots_frame_is_stable_at_80x24() {
        let snapshot = roots_snapshot();
        let run = roots_run();
        let frame = render_live_frame(&snapshot, &run, 80, 24, false, false, false);
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-live-roots-80x24.txt")
        );
    }

    #[test]
    fn live_failure_frames_are_stable() {
        let snapshot = failure_snapshot();
        let run = failure_run();
        let wide = render_live_frame(&snapshot, &run, 120, 40, false, false, false);
        assert_eq!(
            wide,
            include_str!("../../tests/golden/tui-live-failure-120x40.txt")
        );
        let compact = render_live_frame(&snapshot, &run, 80, 24, false, false, false);
        assert_eq!(
            compact,
            include_str!("../../tests/golden/tui-live-failure-80x24.txt")
        );
    }

    #[test]
    fn live_fetch_frame_is_stable_at_120x40() {
        let snapshot = progress_snapshot();
        let run = fetch_run();
        let frame = render_live_frame_status(
            &snapshot,
            &run,
            fetch_footer(),
            120,
            40,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(
            frame.contains("fetch: running sudo -n /usr/bin/pacman -Syuw --noconfirm"),
            "the real argv is on screen\n{frame}"
        );
        assert!(frame.contains("downloading"), "the row is live\n{frame}");
        assert!(
            frame.contains('%'),
            "a known total draws a percentage\n{frame}"
        );
        assert!(
            frame.contains("esc detach"),
            "the live keys are explicit\n{frame}"
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-live-fetch-120x40.txt")
        );
    }

    #[test]
    fn live_fetch_frame_is_stable_at_80x24() {
        let snapshot = progress_snapshot();
        let run = fetch_run();
        let frame = render_live_frame_status(
            &snapshot,
            &run,
            fetch_footer(),
            80,
            24,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(frame.contains("fetch: running"));
        assert!(frame.contains('%'));
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-live-fetch-80x24.txt")
        );
    }

    #[test]
    fn a_refused_action_keeps_the_static_frame_and_names_the_reason() {
        let frame = super::render_frame_status(
            &prefetch_snapshot(),
            Footer {
                status: Some("fetch blocked: pacman is running"),
                keys: Some("r refresh · q quit · ? help"),
                failure: true,
                repair: None,
            },
            120,
            40,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(frame.contains("fetch blocked: pacman is running"));
        assert!(frame.contains("r refresh"));
        assert!(
            frame.contains("simapp"),
            "the static tree is still readable\n{frame}"
        );
    }

    #[test]
    fn live_frames_read_as_the_frontier_without_invented_state() {
        let snapshot = diamond_snapshot();
        let run = diamond_run();
        let frame = render_live_frame(&snapshot, &run, 120, 40, false, false, false);

        for needle in [
            "[|] left",
            "[√] base",
            "[ ] right",
            "planned",
            "building",
            "done",
            "waiting for 1 ⏵ 1 ⏸",
            "⏱ 2s",
            "1 done",
            "1 running",
            "2 waiting",
            "mirror.example.org",
            "0 failed",
        ] {
            assert!(frame.contains(needle), "missing {needle:?} in\n{frame}");
        }
        assert!(!frame.contains("∅"), "no estimate without history\n{frame}");
        assert!(
            !frame.contains("static prefetch"),
            "a live frame is not static"
        );
        assert!(frame.contains("live"));
    }

    #[test]
    fn a_failure_blocks_downstream_without_blanking_the_screen() {
        let snapshot = failure_snapshot();
        let run = failure_run();
        let frame = render_live_frame(&snapshot, &run, 120, 40, false, false, false);

        assert!(
            frame.contains("[×] base"),
            "the failed node stays visible\n{frame}"
        );
        assert!(frame.contains("exit 2"), "the evidence is shown\n{frame}");
        for name in ["left", "right", "join"] {
            assert!(
                frame.contains(&format!("[×] {name}")),
                "{name} is blocked\n{frame}"
            );
        }
        assert!(
            frame.contains("[|] solo"),
            "an independent root keeps running\n{frame}"
        );
        assert!(frame.contains("1 failed"));
        assert!(frame.contains("frozen"));
        assert_eq!(frame.matches("blocked").count(), 3);
    }

    #[test]
    fn repair_hints_name_the_observed_cause() {
        for (evidence, expected) in [
            (
                "exit 1: No space left on device",
                "free disk space, then fetch again",
            ),
            (
                "exit 1: error: failed retrieving file 'x'",
                "check the network and the mirror, then fetch again",
            ),
            (
                "exit 1: error: unable to lock database",
                "wait for the other pacman to finish; never delete the lock file",
            ),
            (
                "exit 1: error: invalid or corrupted package",
                "remove the bad .part from the cache, then fetch again",
            ),
            (
                "exit 1: something else",
                "fix the reported error, then fetch again",
            ),
        ] {
            assert_eq!(repair_hint(evidence), expected, "{evidence}");
        }
    }

    #[test]
    fn the_repair_card_names_the_failed_node_evidence_and_blocked_count() {
        let card = repair_card();
        let frame = render_live_frame_status(
            &failure_snapshot(),
            &failure_run(),
            repair_footer(&card),
            120,
            40,
            false,
            false,
            false,
            false,
            None,
        );

        assert!(
            frame.contains("⚠ base 2.0.0-1 · exit 2: error: target not found: go"),
            "the real evidence stays verbatim\n{frame}"
        );
        assert!(
            frame.contains("repair: fix the reported error, then fetch again"),
            "the correction for this error\n{frame}"
        );
        assert!(
            frame.contains("3 blocked downstream · no state written"),
            "the frozen frontier reports its blocked dependents\n{frame}"
        );
        assert!(frame.contains("esc dismiss · F fetch again · g ledger"));
        assert!(
            !frame.contains("fetch failed:"),
            "the card replaces the duplicated status line\n{frame}"
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-repair-120x40.txt")
        );
    }

    #[test]
    fn the_repair_card_is_stable_at_80x24_and_in_ascii() {
        let card = repair_card();
        let frame = render_live_frame_status(
            &failure_snapshot(),
            &failure_run(),
            repair_footer(&card),
            80,
            24,
            false,
            false,
            false,
            false,
            None,
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-repair-80x24.txt")
        );

        let ascii = render_live_frame_status(
            &failure_snapshot(),
            &failure_run(),
            repair_footer(&card),
            120,
            40,
            true,
            false,
            false,
            false,
            None,
        );
        assert!(ascii.contains("! base 2.0.0-1 | exit 2"), "{ascii}");
        assert!(ascii.contains("repair: "), "{ascii}");
        assert!(
            !ascii.contains('⚠'),
            "ASCII has no unicode card glyph\n{ascii}"
        );
    }

    #[test]
    fn a_failure_without_a_blamed_node_keeps_the_evidence_and_the_note() {
        let card = RepairCard {
            node: None,
            evidence: "exit 1: error: failed to synchronise any database".to_string(),
            repair: repair_hint("failed to synchronise").to_string(),
            blocked: 0,
            note: Some("cannot read cache dir".to_string()),
        };
        let frame = render_live_frame_status(
            &failure_snapshot(),
            &failure_run(),
            repair_footer(&card),
            120,
            40,
            false,
            false,
            false,
            false,
            None,
        );

        assert!(
            frame.contains("⚠ exit 1: error: failed to synchronise any database"),
            "{frame}"
        );
        assert!(frame.contains("no downstream blocked"), "{frame}");
        assert!(
            frame.contains("cache read: cannot read cache dir"),
            "{frame}"
        );
    }

    #[test]
    fn the_ledger_lists_the_same_rows_without_branch_connectors() {
        let frame = render_live_frame_status(
            &diamond_snapshot(),
            &diamond_run(),
            ledger_footer(),
            120,
            40,
            false,
            false,
            false,
            true,
            None,
        );
        assert!(frame.contains("Ledger:"), "{frame}");
        assert!(!frame.contains("├─ "), "no tree connectors\n{frame}");
        assert!(!frame.contains("┌─ "), "no tree corners\n{frame}");
        for name in ["base", "left", "right", "join"] {
            assert!(frame.contains(name), "{name} is in the ledger\n{frame}");
        }
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-ledger-120x40.txt")
        );
    }

    #[test]
    fn the_ledger_is_stable_at_80x24() {
        let frame = render_live_frame_status(
            &diamond_snapshot(),
            &diamond_run(),
            ledger_footer(),
            80,
            24,
            false,
            false,
            false,
            true,
            None,
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-ledger-80x24.txt")
        );
    }

    #[test]
    fn the_static_prefetch_frame_is_unchanged_by_the_ledger() {
        let tree = render_frame(&prefetch_snapshot(), 120, 40, false, false, false);
        let ledger = super::render_frame_status(
            &prefetch_snapshot(),
            ledger_footer(),
            120,
            40,
            false,
            false,
            false,
            true,
            None,
        );
        assert_eq!(
            tree,
            include_str!("../../tests/golden/tui-prefetch-120x40.txt")
        );
        assert!(ledger.contains("Ledger:"));
    }

    #[test]
    fn the_fetch_frame_shows_every_gate_that_ran() {
        let gates = fetch_gates();
        let frame = render_live_frame_status(
            &progress_snapshot(),
            &fetch_run(),
            fetch_footer(),
            120,
            40,
            false,
            false,
            false,
            false,
            Some(&gates),
        );

        assert!(frame.contains("PRE-FLIGHT"), "{frame}");
        for needle in ["✓  lock", "✓  lists", "✓  cache", "✓  space"] {
            assert!(frame.contains(needle), "missing {needle:?}\n{frame}");
        }
        assert!(
            frame.contains("1 pending, 825 B to download"),
            "the lists gate carries the real plan\n{frame}"
        );
        assert!(
            frame.contains("need 5.0 GiB, have 14.2 GiB"),
            "the space gate carries the real numbers\n{frame}"
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-gates-120x40.txt")
        );
    }

    #[test]
    fn the_gate_frame_is_stable_at_80x24() {
        let gates = fetch_gates();
        let frame = render_live_frame_status(
            &progress_snapshot(),
            &fetch_run(),
            fetch_footer(),
            80,
            24,
            false,
            false,
            false,
            false,
            Some(&gates),
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-gates-80x24.txt")
        );
    }

    #[test]
    fn a_refused_gate_shows_only_the_gates_that_ran() {
        let gates = vec![
            Gate::pass("lock", "no lock file, no pacman process"),
            Gate::pass("lists", "1 pending, 825 B to download"),
            Gate::pass("cache", "/var/cache/pacman/pkg"),
            Gate::fail(
                "space",
                "not enough free space on /: need 5.0 GiB, have 1000 B",
            ),
        ];
        let frame = super::render_frame_status(
            &prefetch_snapshot(),
            Footer {
                status: Some(
                    "fetch blocked: not enough free space on /: need 5.0 GiB, have 1000 B",
                ),
                keys: Some("F retry · g ledger · r refresh · q quit · ? help"),
                failure: true,
                repair: None,
            },
            120,
            40,
            false,
            false,
            false,
            false,
            Some(&gates),
        );

        assert!(frame.contains("PRE-FLIGHT"), "{frame}");
        assert!(
            frame.contains("✗  space"),
            "the failed gate is named\n{frame}"
        );
        assert!(
            frame.contains("not enough free space"),
            "the exact reason is kept\n{frame}"
        );
        assert!(frame.contains("fetch blocked:"), "{frame}");
    }

    #[test]
    fn a_warning_gate_is_not_a_pass() {
        let gates = vec![
            Gate::pass("lock", "no lock file, no pacman process"),
            Gate::warn(
                "lists",
                "pacman -Qui exited 1; assuming the minimum free-space floor",
            ),
            Gate::pass("cache", "/var/cache/pacman/pkg"),
            Gate::pass("space", "/: need 1.0 MiB, have 40.1 GiB"),
        ];
        let frame = render_live_frame_status(
            &progress_snapshot(),
            &fetch_run(),
            fetch_footer(),
            120,
            40,
            false,
            false,
            false,
            true,
            Some(&gates),
        );

        assert!(frame.contains("▲  lists"), "{frame}");
        assert!(
            frame.contains("assuming the minimum free-space floor"),
            "{frame}"
        );
        assert!(
            !frame.contains("✓  lists"),
            "a warning is never a green check\n{frame}"
        );
    }

    #[test]
    fn the_log_shows_the_newest_lines_through_the_untrusted_gate() {
        let frame = render_log_frame(&long_log_fixture(), 120, 40, false, false);

        assert!(frame.contains("fetch log"), "{frame}");
        assert!(
            frame.contains("line 99"),
            "the newest line is visible\n{frame}"
        );
        assert!(
            !frame.contains("Synchronizing"),
            "the tail is shown, not the head\n{frame}"
        );
        assert!(!frame.contains('\x1b'), "ESC is dropped\n{frame}");
        assert!(!frame.contains('\t'), "tabs are expanded\n{frame}");
        assert_eq!(frame, include_str!("../../tests/golden/tui-log-120x40.txt"));
    }

    #[test]
    fn the_log_sanitizes_child_output_and_names_an_empty_log() {
        let frame = render_log_frame(&log_fixture(), 120, 40, false, false);
        assert!(frame.contains("downloading"), "{frame}");
        assert!(
            frame.contains("downloading simapp-2.0.0-1"),
            "the tab became a space\n{frame}"
        );
        assert!(!frame.contains('\x1b'), "ESC is dropped\n{frame}");

        let empty = render_log_frame(&VecDeque::new(), 80, 24, false, false);
        assert!(empty.contains("(no output yet)"), "{empty}");
    }

    #[test]
    fn the_install_gate_shows_the_checks_and_the_exact_command() {
        let frame = render_install_gate(&install_preflight(), false, 120, 40, false, false);

        for needle in [
            "Install · 1 package",
            "PRE-FLIGHT",
            "✓  root",
            "✓  lock",
            "✓  lists",
            "✓  trust",
            "▲  news",
            "archlinux.org/news",
            "COMMAND   yay -Syuu",
            "space attest",
            "esc cancel",
        ] {
            assert!(frame.contains(needle), "missing {needle:?}\n{frame}");
        }
        assert!(
            !frame.contains("enter"),
            "the action key is not advertised before it exists\n{frame}"
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-install-gate-120x40.txt")
        );
    }

    #[test]
    fn attestation_marks_the_news_gate_for_this_session() {
        let frame = render_install_gate(&install_preflight(), true, 120, 40, false, false);

        assert!(frame.contains("✓  news"), "{frame}");
        assert!(frame.contains("attested for this session"), "{frame}");
        assert!(!frame.contains("press space"), "{frame}");
        assert!(!frame.contains("space attest"), "{frame}");
        assert!(
            frame.contains("enter install"),
            "the action is armed\n{frame}"
        );
        assert!(!frame.contains("a gate blocks install"), "{frame}");
    }

    #[test]
    fn the_success_receipt_reports_the_exit_kernel_pacnew_and_reboot() {
        let frame = render_post_install(&success_receipt(), 120, 40, false, false);

        for needle in [
            "Post-install · success",
            "✓  yay -Syuu exited 0",
            "running 6.2",
            "on disk 6.1, 6.2",
            "2 .pacnew file(s)",
            "run sudo pacdiff after closing",
            "reboot    not required",
            "enter / esc close",
        ] {
            assert!(frame.contains(needle), "missing {needle:?}\n{frame}");
        }
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-post-install-120x40.txt")
        );
    }

    #[test]
    fn the_failure_receipt_names_the_exit_and_the_repair() {
        let frame = render_post_install(&failure_receipt(), 120, 40, false, false);

        assert!(frame.contains("Post-install · failure"), "{frame}");
        assert!(frame.contains("✖  yay -Syuu exited 1"), "{frame}");
        assert!(
            frame.contains("not checked because the upgrade failed"),
            "{frame}"
        );
        assert!(frame.contains("repair: disk full"), "{frame}");
        assert!(
            frame.contains("reboot    recommended"),
            "the kernel on disk differs\n{frame}"
        );
        assert!(!frame.contains("run sudo pacdiff"), "{frame}");
    }

    #[test]
    fn a_not_run_receipt_never_claims_an_exit() {
        let mut receipt = success_receipt();
        receipt.ran = false;
        receipt.failed = true;
        receipt.reason = Some("stale lock db.lck".to_string());
        let frame = render_post_install(&receipt, 120, 40, false, false);

        assert!(
            frame.contains("yay -Syuu not run: stale lock db.lck"),
            "{frame}"
        );
        assert!(!frame.contains("exited"), "no invented exit\n{frame}");
        assert!(frame.contains("the install did not run"), "{frame}");
    }

    #[test]
    fn the_receipt_is_ascii_safe() {
        let frame = render_post_install(&success_receipt(), 120, 40, true, false);

        assert!(frame.contains("Post-install | success"), "{frame}");
        assert!(frame.contains("+  yay -Syuu exited 0"), "{frame}");
        assert!(frame.contains("-  reboot"), "{frame}");
        assert!(!frame.contains('·'), "{frame}");
    }

    #[test]
    fn a_blocked_gate_never_arms_the_install() {
        let mut preflight = install_preflight();
        preflight.gates.push(Gate::fail(
            "trust",
            "trusted-users is empty; aurto would trust no one; fix the file first",
        ));
        let frame = render_install_gate(&preflight, true, 120, 40, false, false);

        assert!(frame.contains("a gate blocks install"), "{frame}");
        assert!(!frame.contains("enter install"), "{frame}");
    }

    #[test]
    fn the_install_gate_is_ascii_safe() {
        let frame = render_install_gate(&install_preflight(), false, 120, 40, true, false);

        assert!(frame.contains("Install | 1 package"), "{frame}");
        assert!(frame.contains("!  news"), "{frame}");
        assert!(frame.contains("+  root"), "{frame}");
        assert!(!frame.contains('·'), "no unicode separator\n{frame}");
    }

    #[test]
    fn a_skipped_node_is_dim_and_never_a_success() {
        let snapshot = skipped_snapshot();
        let run = skipped_run();
        let frame = render_live_frame(&snapshot, &run, 80, 24, false, false, false);
        let row = frame
            .lines()
            .find(|line| line.contains("cached-pkg"))
            .expect("the skipped row is on screen");
        assert!(row.contains("[√]"), "skipped glyph\n{frame}");
        assert!(row.contains("skipped"), "the live state is named\n{frame}");
        assert!(
            row.contains("cached"),
            "the cache evidence is kept\n{frame}"
        );
        assert!(
            !row.contains("✔"),
            "a skip is not a completed build\n{frame}"
        );
    }

    #[test]
    fn a_known_transfer_draws_noms_bar_geometry() {
        let snapshot = progress_snapshot();
        let run = progress_run();

        let wide = render_live_frame(&snapshot, &run, 120, 40, false, false, false);
        let row = wide
            .lines()
            .find(|line| line.contains("downloading"))
            .expect("the transfer row is on screen");
        assert_eq!(row.chars().count(), 120, "the row fills the frame");
        assert!(row.contains("46.7 MiB/57.0 MiB"), "bytes before the bar");
        assert!(row.ends_with(" 81.9%"), "the percent is the right edge");
        assert_eq!(
            row.chars().position(|ch| ch == '■'),
            Some(61),
            "left_width 60 plus one separator"
        );
        assert_eq!(row.matches('■').count(), 42, "full cells");
        assert_eq!(row.matches('◧').count(), 1, "one half cell");
        assert_eq!(row.matches('□').count(), 9, "empty cells");
        assert_eq!(
            row.matches('■').count() + row.matches('◧').count() + row.matches('□').count(),
            52
        );
        assert_eq!(
            wide,
            include_str!("../../tests/golden/tui-live-progress-120x40.txt")
        );

        let compact = render_live_frame(&snapshot, &run, 80, 24, false, false, false);
        let row = compact
            .lines()
            .find(|line| line.contains("downloading"))
            .expect("the compact transfer row is on screen");
        assert_eq!(row.chars().count(), 80);
        assert!(row.ends_with(" 81.9%"));
        assert_eq!(row.matches('■').count(), 9);
        assert_eq!(row.matches('◧').count(), 1);
        assert_eq!(row.matches('□').count(), 2);
        assert_eq!(
            compact,
            include_str!("../../tests/golden/tui-live-progress-80x24.txt")
        );
    }

    #[test]
    fn an_unknown_total_draws_no_bar_and_no_percent() {
        let snapshot = progress_snapshot();
        let run = unknown_progress_run();

        for (width, height) in [(120, 40), (80, 24)] {
            let frame = render_live_frame(&snapshot, &run, width, height, false, false, false);
            let row = frame
                .lines()
                .find(|line| line.contains("downloading"))
                .expect("the running row is still on screen");
            assert!(row.contains("downloading"));
            assert!(!row.contains('%'), "no percent without a total\n{frame}");
            for glyph in ['■', '◧', '□'] {
                assert!(
                    !row.contains(glyph),
                    "no bar cell {glyph} without a total\n{frame}"
                );
            }
        }

        assert_eq!(
            render_live_frame(&snapshot, &run, 120, 40, false, false, false),
            include_str!("../../tests/golden/tui-live-progress-unknown-120x40.txt")
        );
        assert_eq!(
            render_live_frame(&snapshot, &run, 80, 24, false, false, false),
            include_str!("../../tests/golden/tui-live-progress-unknown-80x24.txt")
        );
    }

    #[test]
    fn ascii_bars_keep_the_geometry_without_unicode() {
        let snapshot = progress_snapshot();
        let run = progress_run();

        let wide = render_live_frame(&snapshot, &run, 120, 40, true, false, false);
        assert!(wide.is_ascii(), "ascii frame had non-ASCII bytes\n{wide}");
        let wide_row = wide
            .lines()
            .find(|line| line.contains("downloading"))
            .expect("the wide ascii transfer row is on screen");
        assert_eq!(wide_row.chars().count(), 120);
        assert_eq!(wide_row.matches('#').count(), 42, "full cells");
        assert!(
            wide_row.contains(&format!("{}-", "#".repeat(42))),
            "42 full cells then the half cell"
        );
        assert_eq!(
            wide,
            include_str!("../../tests/golden/tui-live-progress-ascii-120x40.txt")
        );

        let frame = render_live_frame(&snapshot, &run, 80, 24, true, false, false);
        assert!(frame.is_ascii(), "ascii frame had non-ASCII bytes\n{frame}");

        let row = frame
            .lines()
            .find(|line| line.contains("downloading"))
            .expect("the ascii transfer row is on screen");
        assert!(row.contains("[|] linux"), "the download glyph is ASCII");
        assert_eq!(row.chars().count(), 80);
        assert!(row.ends_with(" 81.9%"));
        assert_eq!(row.matches('#').count(), 9, "full cells");
        assert!(
            row.contains("#########-"),
            "nine full cells then one half cell"
        );
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-live-progress-ascii-80x24.txt")
        );
    }

    #[test]
    fn a_progress_row_at_40_columns_does_not_overflow() {
        let snapshot = progress_snapshot();
        let run = progress_run();
        let frame = render_live_frame(&snapshot, &run, 40, 24, false, false, false);

        for line in frame.lines() {
            assert!(
                display_width(line) <= 40,
                "row wider than 40 cells: {line:?}"
            );
        }
        let row = frame
            .lines()
            .find(|line| line.contains("downloading"))
            .expect("the running row is still readable");
        assert!(
            !row.contains('%'),
            "no percent when the bar cannot fit\n{frame}"
        );
        for glyph in ['■', '◧', '□'] {
            assert!(!row.contains(glyph), "no bar at 40 columns\n{frame}");
        }
    }

    #[test]
    fn progress_no_color_keeps_the_text_and_drops_the_palette() {
        let snapshot = progress_snapshot();
        let run = progress_run();
        let colored = render_live_frame(&snapshot, &run, 120, 40, false, true, false);
        let plain = render_live_frame(&snapshot, &run, 120, 40, false, false, false);
        assert_eq!(colored, plain, "color never changes the text");

        let buffer = buffer_of(&snapshot, Some(&run), 120, 40, false, true);
        let row = find_row(&buffer, 120, "81.9%");
        let percent = text_column(&buffer, 120, row, "81.9%").expect("percent");
        assert_eq!(cell_at(&buffer, 120, row, percent).fg, Color::Yellow);
        assert!(
            cell_at(&buffer, 120, row, percent)
                .modifier
                .contains(Modifier::BOLD),
            "the percent is bold in color mode"
        );
        let full = text_column(&buffer, 120, row, "■").expect("full cell");
        assert_eq!(cell_at(&buffer, 120, row, full).fg, Color::Yellow);
        let empty = text_column(&buffer, 120, row, "□").expect("empty cell");
        assert_eq!(cell_at(&buffer, 120, row, empty).fg, Color::DarkGray);

        let plain_buffer = buffer_of(&snapshot, Some(&run), 120, 40, false, false);
        assert!(
            plain_buffer
                .content()
                .iter()
                .all(|cell| cell.fg == Color::Reset),
            "NO_COLOR leaves no foreground color"
        );
        let plain_row = find_row(&plain_buffer, 120, "81.9%");
        let plain_percent = text_column(&plain_buffer, 120, plain_row, "81.9%").expect("percent");
        assert!(
            cell_at(&plain_buffer, 120, plain_row, plain_percent)
                .modifier
                .contains(Modifier::BOLD),
            "bold survives NO_COLOR; only the color is dropped"
        );
    }

    #[test]
    fn a_200_package_window_clips_but_never_hides_live_rows() {
        let snapshot = window_snapshot();
        let run = window_run();

        let wide = render_live_frame(&snapshot, &run, 120, 40, false, false, false);
        assert!(
            wide.contains("Dependency Graph showing 31 of 197 roots:"),
            "the header names the clipped root count\n{wide}"
        );
        assert!(wide.contains("⋮"), "the clipped marker is shown\n{wide}");
        for name in ["z-mid", "a-base", "a-top"] {
            assert!(wide.contains(name), "{name} is never hidden\n{wide}");
        }
        assert!(
            !wide.contains("pkg-100"),
            "a settled row outside the window is clipped\n{wide}"
        );
        for name in ["z-mid", "a-base", "a-top", "pkg-009"] {
            assert_eq!(
                wide.matches(name).count(),
                1,
                "{name} appears exactly once\n{wide}"
            );
        }
        assert_eq!(
            wide,
            include_str!("../../tests/golden/tui-live-window-120x40.txt")
        );

        let compact = render_live_frame(&snapshot, &run, 80, 24, false, false, false);
        assert!(
            compact.contains("Dependency Graph showing 15 of 197 roots:"),
            "the compact header names the clipped root count\n{compact}"
        );
        assert!(
            compact.contains("⋮"),
            "the compact window is marked\n{compact}"
        );
        for name in ["z-mid", "a-base", "a-top"] {
            assert!(compact.contains(name), "{name} is never hidden\n{compact}");
        }
        assert!(
            !compact.contains("pkg-100"),
            "a settled row outside the compact window is clipped\n{compact}"
        );
        assert_eq!(
            compact,
            include_str!("../../tests/golden/tui-live-window-80x24.txt")
        );
    }

    #[test]
    fn no_color_keeps_the_exact_text_and_drops_every_color() {
        let snapshot = diamond_snapshot();
        let run = diamond_run();
        let colored = render_live_frame(&snapshot, &run, 120, 40, false, true, false);
        let plain = render_live_frame(&snapshot, &run, 120, 40, false, false, false);
        assert_eq!(colored, plain, "color never changes the text");

        let buffer = buffer_of(&snapshot, Some(&run), 120, 40, false, false);
        assert!(
            buffer.content().iter().all(|cell| cell.fg == Color::Reset),
            "NO_COLOR leaves no foreground color"
        );
    }

    #[test]
    fn the_palette_speaks_through_glyphs_and_words_not_color_alone() {
        let snapshot = diamond_snapshot();
        let run = diamond_run();
        let buffer = buffer_of(&snapshot, Some(&run), 120, 40, false, true);

        let planned = find_row(&buffer, 120, "planned");
        let planned_glyph =
            text_column(&buffer, 120, planned, "[ ]").expect("the planned glyph is on the row");
        assert_eq!(
            cell_at(&buffer, 120, planned, planned_glyph).fg,
            Color::Blue,
            "planned is blue"
        );

        let running = find_row(&buffer, 120, "building");
        let glyph =
            text_column(&buffer, 120, running, "[|]").expect("the running glyph is on the row");
        assert_eq!(cell_at(&buffer, 120, running, glyph).fg, Color::Yellow);
        let name = text_column(&buffer, 120, running, "left").expect("the running name");
        assert!(
            cell_at(&buffer, 120, running, name)
                .modifier
                .contains(Modifier::BOLD)
        );

        let done = find_row(&buffer, 120, "[√]");
        let done_glyph = text_column(&buffer, 120, done, "[√]").expect("done glyph");
        assert_eq!(cell_at(&buffer, 120, done, done_glyph).fg, Color::Green);

        let host = find_row(&buffer, 120, "mirror.example.org");
        assert!(
            text_column(&buffer, 120, host, "mirror").is_some(),
            "the host value sits on the summary row"
        );
        let labels = find_row(&buffer, 120, "running");
        assert_eq!(
            cell_at(&buffer, 120, labels, 0).fg,
            Color::DarkGray,
            "the footer is dim"
        );

        let waiting = find_row(&buffer, 120, "waiting for");
        let waiting_column = text_column(&buffer, 120, waiting, "waiting").expect("waiting word");
        assert_eq!(
            cell_at(&buffer, 120, waiting, waiting_column).fg,
            Color::DarkGray,
            "waiting for is dim"
        );

        assert!(
            cell_at(&buffer, 120, 0, 0)
                .modifier
                .contains(Modifier::BOLD)
        );

        let roots = buffer_of(&roots_snapshot(), Some(&roots_run()), 120, 40, false, true);
        let download = find_row(&roots, 120, "downloading");
        let down_glyph =
            text_column(&roots, 120, download, "[|]").expect("the running marker is on the row");
        assert_eq!(
            cell_at(&roots, 120, download, down_glyph).fg,
            Color::Yellow,
            "a running download is yellow"
        );

        let failure = buffer_of(
            &failure_snapshot(),
            Some(&failure_run()),
            120,
            40,
            false,
            true,
        );
        let failed = find_row(&failure, 120, "failed");
        let warning = text_column(&failure, 120, failed, "[×]").expect("the failure glyph");
        assert_eq!(
            cell_at(&failure, 120, failed, warning).fg,
            Color::Red,
            "a failure is red"
        );
        let blocked = find_row(&failure, 120, "blocked");
        let square = text_column(&failure, 120, blocked, "[×]").expect("the blocked glyph");
        assert_eq!(
            cell_at(&failure, 120, blocked, square).fg,
            Color::DarkGray,
            "a blocked node is dim"
        );

        let skipped = buffer_of(
            &skipped_snapshot(),
            Some(&skipped_run()),
            80,
            24,
            false,
            true,
        );
        let skipped_row = find_row(&skipped, 80, "cached-pkg");
        let skipped_glyph =
            text_column(&skipped, 80, skipped_row, "[√]").expect("the skipped glyph");
        assert_eq!(
            cell_at(&skipped, 80, skipped_row, skipped_glyph).fg,
            Color::DarkGray,
            "a skipped node is dim"
        );

        for word in [
            "planned", "building", "done", "failed", "blocked", "skipped",
        ] {
            assert!(
                render_live_frame(
                    &failure_snapshot(),
                    &failure_run(),
                    120,
                    40,
                    false,
                    true,
                    false
                )
                .contains(word)
                    || render_live_frame(&snapshot, &run, 120, 40, false, true, false)
                        .contains(word)
                    || render_live_frame(
                        &skipped_snapshot(),
                        &skipped_run(),
                        120,
                        40,
                        false,
                        true,
                        false
                    )
                    .contains(word),
                "state word {word} is present somewhere"
            );
        }
    }

    #[test]
    fn the_tree_frame_is_stable_at_80x24() {
        let frame = render_frame(&snapshot(), 80, 24, false, false, false);
        assert_eq!(frame, include_str!("../../tests/golden/tui-80x24.txt"));
    }

    #[test]
    fn the_tree_frame_is_stable_at_120x40() {
        let frame = render_frame(&snapshot(), 120, 40, false, false, false);
        assert_eq!(frame, include_str!("../../tests/golden/tui-120x40.txt"));
    }

    #[test]
    fn ascii_mode_has_no_non_ascii_bytes() {
        let frame = render_frame(&snapshot(), 80, 24, true, false, false);
        assert!(
            frame.is_ascii(),
            "ascii frame contained a non-ASCII character"
        );
    }

    #[test]
    fn prefetch_badges_name_each_package_once() {
        let frame = render_frame(&prefetch_snapshot(), 80, 24, false, false, false);
        for label in ["cached", "stale", "build-pending", "missing"] {
            assert!(frame.contains(label), "missing {label} in\n{frame}");
        }
        assert_eq!(frame.matches("simapp").count(), 1);
        assert!(frame.contains("1 ready · 1 missing · 1 stale"));
        assert!(frame.contains("mirror.example.org"));
        assert!(frame.contains("4 packages"));
    }

    #[test]
    fn prefetch_ascii_has_no_non_ascii_bytes() {
        let frame = render_frame(&prefetch_snapshot(), 80, 24, true, false, false);
        assert!(
            frame.is_ascii(),
            "ascii prefetch frame contained a non-ASCII character"
        );
    }

    #[test]
    fn the_prefetch_forest_is_bottom_up_with_nom_branches() {
        let frame = render_frame(&prefetch_snapshot(), 120, 40, false, false, false);
        let lines: Vec<&str> = frame.lines().collect();

        assert!(lines[1].contains("Dependency Graph:"));
        assert!(lines[1].contains("install 4 packages"));
        assert!(
            lines
                .iter()
                .skip(2)
                .all(|line| !line.trim_start().starts_with("install")),
            "no tree row may be an install node\n{frame}"
        );
        assert!(
            lines[2].contains("simapp"),
            "the first tree row is a real root\n{frame}"
        );

        for (child, parent) in [("yay-bin", "linux"), ("glibc", "bash"), ("bash", "linux")] {
            let child_line = lines
                .iter()
                .position(|line| line.contains(child))
                .unwrap_or_else(|| panic!("{child} missing in\n{frame}"));
            let parent_line = lines
                .iter()
                .position(|line| line.contains(parent))
                .unwrap_or_else(|| panic!("{parent} missing in\n{frame}"));
            assert!(
                child_line < parent_line,
                "{child} must render above {parent} in\n{frame}"
            );
        }
        assert!(
            frame.contains("┌─"),
            "a last child uses the corner\n{frame}"
        );
        assert!(
            frame.contains("├─"),
            "an earlier child uses a branch\n{frame}"
        );
        assert!(
            frame.contains("Dependency Graph:"),
            "the nom box top\n{frame}"
        );
        assert!(
            frame.contains("┌─"),
            "the corner connector is back\n{frame}"
        );
        assert!(
            !frame.contains("┏━") && !frame.contains("┃"),
            "the heavy nom box is gone\n{frame}"
        );
        assert_eq!(frame.matches("? glibc").count(), 1);
        assert_eq!(frame.matches("simapp").count(), 1);
        assert!(frame.contains("unknown"));
    }

    #[test]
    fn the_prefetch_forest_ascii_uses_ascii_branches() {
        let frame = render_frame(&prefetch_snapshot(), 80, 24, true, false, false);
        assert!(frame.is_ascii(), "ascii frame had non-ASCII bytes\n{frame}");
        assert!(
            frame.contains("`--") || frame.contains("|--"),
            "no ascii branch in\n{frame}"
        );
        assert!(frame.contains("install 4 packages"));
        assert!(frame.contains("Dependency Graph:"));
    }

    #[test]
    fn the_prefetch_forest_truncates_safely_at_40_columns() {
        let frame = render_frame(&prefetch_snapshot(), 40, 24, false, false, false);
        for line in frame.lines() {
            assert!(
                line.chars().count() <= 40,
                "row wider than 40 cells: {line:?}"
            );
        }
        assert!(frame.contains("Dependency Graph:"));
    }

    #[test]
    fn two_independent_roots_render_as_two_top_level_rows() {
        let now = at();
        let snapshot = prefetch_snapshot_with(vec![
            prefetch_package(
                "root-a",
                "2.0.0-1",
                ArtifactState::Cached,
                "cached",
                now,
                &[],
            ),
            prefetch_package(
                "root-b",
                "2.0.0-1",
                ArtifactState::Missing,
                "not in cache",
                now,
                &[],
            ),
        ]);
        let frame = render_frame(&snapshot, 80, 24, false, false, false);
        let lines: Vec<&str> = frame.lines().collect();

        assert!(lines[1].contains("Dependency Graph:"));
        assert!(lines[1].contains("install 2 packages"));
        assert!(
            lines[2].contains("root-b"),
            "last root is the top row\n{frame}"
        );
        assert!(
            lines[3].contains("root-a"),
            "first root is the bottom row\n{frame}"
        );
        assert!(
            lines.iter().skip(2).all(|line| !line.contains("install")),
            "the forest contains no install row\n{frame}"
        );
    }

    #[test]
    fn the_graph_header_uses_nom_forms() {
        assert_eq!(super::graph_title(3, 3), "Dependency Graph:");
        assert_eq!(
            super::graph_title(2, 5),
            "Dependency Graph showing 2 of 5 roots:"
        );
    }

    #[test]
    fn an_unrooted_cycle_renders_a_deterministic_error() {
        let now = at();
        let snapshot = prefetch_snapshot_with(vec![
            prefetch_package(
                "a",
                "2.0.0-1",
                ArtifactState::Missing,
                "not in cache",
                now,
                &["b"],
            ),
            prefetch_package(
                "b",
                "2.0.0-1",
                ArtifactState::Missing,
                "not in cache",
                now,
                &["a"],
            ),
        ]);
        let frame = render_frame(&snapshot, 80, 24, false, false, false);

        assert!(frame.contains("error: dependency cycle has no root: a, b"));
        assert!(
            !frame.contains("┌─"),
            "no tree is drawn for a cycle\n{frame}"
        );
    }

    #[test]
    fn prefetch_frames_are_stable_at_80x24() {
        let frame = render_frame(&prefetch_snapshot(), 80, 24, false, false, false);
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-prefetch-80x24.txt")
        );
    }

    #[test]
    fn prefetch_frames_are_stable_at_120x40() {
        let frame = render_frame(&prefetch_snapshot(), 120, 40, false, false, false);
        assert_eq!(
            frame,
            include_str!("../../tests/golden/tui-prefetch-120x40.txt")
        );
    }

    #[test]
    fn help_opens_over_the_tree() {
        let frame = render_frame(&snapshot(), 80, 24, false, false, true);
        assert!(frame.contains("help"));
        assert!(frame.contains("ctrl-z  suspend"));
    }

    #[test]
    fn a_tiny_terminal_gets_one_centered_line() {
        let frame = render_frame(&snapshot(), 20, 5, false, false, false);
        assert!(frame.contains("terminal too small"));
    }

    #[test]
    fn zero_size_does_not_panic() {
        assert_eq!(render_frame(&snapshot(), 0, 0, false, false, false), "");
        assert_eq!(render_frame(&snapshot(), 80, 0, false, false, false), "");
    }

    #[test]
    #[ignore = "regenerates the golden frames"]
    fn regenerate_golden_frames() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
        std::fs::write(
            dir.join("tui-80x24.txt"),
            render_frame(&snapshot(), 80, 24, false, false, false),
        )
        .expect("write 80x24");
        std::fs::write(
            dir.join("tui-120x40.txt"),
            render_frame(&snapshot(), 120, 40, false, false, false),
        )
        .expect("write 120x40");
        std::fs::write(
            dir.join("tui-prefetch-80x24.txt"),
            render_frame(&prefetch_snapshot(), 80, 24, false, false, false),
        )
        .expect("write prefetch 80x24");
        std::fs::write(
            dir.join("tui-prefetch-120x40.txt"),
            render_frame(&prefetch_snapshot(), 120, 40, false, false, false),
        )
        .expect("write prefetch 120x40");

        let diamond = diamond_snapshot();
        let diamond_run = diamond_run();
        std::fs::write(
            dir.join("tui-live-diamond-120x40.txt"),
            render_live_frame(&diamond, &diamond_run, 120, 40, false, false, false),
        )
        .expect("write live diamond 120x40");
        std::fs::write(
            dir.join("tui-live-diamond-80x24.txt"),
            render_live_frame(&diamond, &diamond_run, 80, 24, false, false, false),
        )
        .expect("write live diamond 80x24");
        std::fs::write(
            dir.join("tui-live-diamond-ascii-80x24.txt"),
            render_live_frame(&diamond, &diamond_run, 80, 24, true, false, false),
        )
        .expect("write live diamond ascii");
        std::fs::write(
            dir.join("tui-live-subsecond-80x24.txt"),
            render_live_frame(&diamond, &subsecond_run(), 80, 24, false, false, false),
        )
        .expect("write live subsecond");
        std::fs::write(
            dir.join("tui-live-roots-120x40.txt"),
            render_live_frame(
                &roots_snapshot(),
                &roots_run(),
                120,
                40,
                false,
                false,
                false,
            ),
        )
        .expect("write live roots 120x40");
        std::fs::write(
            dir.join("tui-live-roots-80x24.txt"),
            render_live_frame(&roots_snapshot(), &roots_run(), 80, 24, false, false, false),
        )
        .expect("write live roots 80x24");
        std::fs::write(
            dir.join("tui-live-failure-120x40.txt"),
            render_live_frame(
                &failure_snapshot(),
                &failure_run(),
                120,
                40,
                false,
                false,
                false,
            ),
        )
        .expect("write live failure 120x40");
        std::fs::write(
            dir.join("tui-live-failure-80x24.txt"),
            render_live_frame(
                &failure_snapshot(),
                &failure_run(),
                80,
                24,
                false,
                false,
                false,
            ),
        )
        .expect("write live failure 80x24");

        let progress = progress_snapshot();
        let progress_run = progress_run();
        std::fs::write(
            dir.join("tui-live-progress-120x40.txt"),
            render_live_frame(&progress, &progress_run, 120, 40, false, false, false),
        )
        .expect("write live progress 120x40");
        std::fs::write(
            dir.join("tui-live-progress-80x24.txt"),
            render_live_frame(&progress, &progress_run, 80, 24, false, false, false),
        )
        .expect("write live progress 80x24");
        std::fs::write(
            dir.join("tui-live-progress-ascii-80x24.txt"),
            render_live_frame(&progress, &progress_run, 80, 24, true, false, false),
        )
        .expect("write live progress ascii 80x24");
        std::fs::write(
            dir.join("tui-live-progress-ascii-120x40.txt"),
            render_live_frame(&progress, &progress_run, 120, 40, true, false, false),
        )
        .expect("write live progress ascii 120x40");

        let unknown = unknown_progress_run();
        std::fs::write(
            dir.join("tui-live-progress-unknown-120x40.txt"),
            render_live_frame(&progress, &unknown, 120, 40, false, false, false),
        )
        .expect("write live unknown progress 120x40");
        std::fs::write(
            dir.join("tui-live-progress-unknown-80x24.txt"),
            render_live_frame(&progress, &unknown, 80, 24, false, false, false),
        )
        .expect("write live unknown progress 80x24");

        let window = window_snapshot();
        let window_run = window_run();
        std::fs::write(
            dir.join("tui-live-window-120x40.txt"),
            render_live_frame(&window, &window_run, 120, 40, false, false, false),
        )
        .expect("write live window 120x40");
        std::fs::write(
            dir.join("tui-live-window-80x24.txt"),
            render_live_frame(&window, &window_run, 80, 24, false, false, false),
        )
        .expect("write live window 80x24");

        let fetch = progress_snapshot();
        let fetch_run = fetch_run();
        std::fs::write(
            dir.join("tui-live-fetch-120x40.txt"),
            render_live_frame_status(
                &fetch,
                &fetch_run,
                fetch_footer(),
                120,
                40,
                false,
                false,
                false,
                false,
                None,
            ),
        )
        .expect("write live fetch 120x40");
        std::fs::write(
            dir.join("tui-live-fetch-80x24.txt"),
            render_live_frame_status(
                &fetch,
                &fetch_run,
                fetch_footer(),
                80,
                24,
                false,
                false,
                false,
                false,
                None,
            ),
        )
        .expect("write live fetch 80x24");

        let card = repair_card();
        std::fs::write(
            dir.join("tui-repair-120x40.txt"),
            render_live_frame_status(
                &failure_snapshot(),
                &failure_run(),
                repair_footer(&card),
                120,
                40,
                false,
                false,
                false,
                false,
                None,
            ),
        )
        .expect("write repair 120x40");
        std::fs::write(
            dir.join("tui-repair-80x24.txt"),
            render_live_frame_status(
                &failure_snapshot(),
                &failure_run(),
                repair_footer(&card),
                80,
                24,
                false,
                false,
                false,
                false,
                None,
            ),
        )
        .expect("write repair 80x24");

        std::fs::write(
            dir.join("tui-ledger-120x40.txt"),
            render_live_frame_status(
                &diamond,
                &diamond_run,
                ledger_footer(),
                120,
                40,
                false,
                false,
                false,
                true,
                None,
            ),
        )
        .expect("write ledger 120x40");
        std::fs::write(
            dir.join("tui-ledger-80x24.txt"),
            render_live_frame_status(
                &diamond,
                &diamond_run,
                ledger_footer(),
                80,
                24,
                false,
                false,
                false,
                true,
                None,
            ),
        )
        .expect("write ledger 80x24");

        let gates = fetch_gates();
        std::fs::write(
            dir.join("tui-gates-120x40.txt"),
            render_live_frame_status(
                &fetch,
                &fetch_run,
                fetch_footer(),
                120,
                40,
                false,
                false,
                false,
                false,
                Some(&gates),
            ),
        )
        .expect("write gates 120x40");
        std::fs::write(
            dir.join("tui-gates-80x24.txt"),
            render_live_frame_status(
                &fetch,
                &fetch_run,
                fetch_footer(),
                80,
                24,
                false,
                false,
                false,
                false,
                Some(&gates),
            ),
        )
        .expect("write gates 80x24");

        std::fs::write(
            dir.join("tui-log-120x40.txt"),
            render_log_frame(&long_log_fixture(), 120, 40, false, false),
        )
        .expect("write log 120x40");

        std::fs::write(
            dir.join("tui-install-gate-120x40.txt"),
            render_install_gate(&install_preflight(), false, 120, 40, false, false),
        )
        .expect("write install gate 120x40");

        std::fs::write(
            dir.join("tui-showcase-120x40.txt"),
            render_live_frame_status(
                &showcase_snapshot(),
                &showcase_run(),
                showcase_footer(&showcase_card(), false),
                120,
                40,
                false,
                false,
                false,
                false,
                Some(&fetch_gates()),
            ),
        )
        .expect("write showcase 120x40");
        std::fs::write(
            dir.join("tui-showcase-80x24.txt"),
            render_live_frame_status(
                &showcase_snapshot(),
                &showcase_run(),
                showcase_footer(&showcase_card(), false),
                80,
                24,
                false,
                false,
                false,
                false,
                Some(&fetch_gates()),
            ),
        )
        .expect("write showcase 80x24");
        std::fs::write(
            dir.join("tui-showcase-ascii-80x24.txt"),
            render_live_frame_status(
                &showcase_snapshot(),
                &showcase_run(),
                showcase_footer(&showcase_card(), true),
                80,
                24,
                true,
                false,
                false,
                false,
                Some(&fetch_gates()),
            ),
        )
        .expect("write showcase ascii 80x24");
        std::fs::write(
            dir.join("tui-showcase-ascii-120x40.txt"),
            render_live_frame_status(
                &showcase_snapshot(),
                &showcase_run(),
                showcase_footer(&showcase_card(), true),
                120,
                40,
                true,
                false,
                false,
                false,
                Some(&fetch_gates()),
            ),
        )
        .expect("write showcase ascii 120x40");

        std::fs::write(
            dir.join("tui-post-install-120x40.txt"),
            render_post_install(&success_receipt(), 120, 40, false, false),
        )
        .expect("write post-install 120x40");
    }
}
