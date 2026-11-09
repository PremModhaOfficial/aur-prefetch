//! The T1 tree frame. The data comes from the report layer; this module only
//! lays it out and never shells out.

use ratatui::backend::TestBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::report::{ArtifactState, PrefetchReport, Snapshot, Update};
use crate::util::human_bytes;

use super::live::{
    Activity, GroupCounts, LiveRun, LiveState, LiveView, Progress, Window, WindowSlot,
    tree_row_budget,
};
use super::sanitize::{cell_text, display_width};
use super::tree::{RowId, Tree, TreeError, TreeRow, TreeRowKind};

/// The ANSI-16 semantic palette. Color is an accent on top of a glyph and a
/// word; it is never the only signal.
const PLANNED: Color = Color::Blue;
const ACTIVE: Color = Color::Yellow;
const DONE: Color = Color::Green;
const HOST: Color = Color::Magenta;
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

pub fn render_frame(
    snapshot: &Snapshot,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
    help: bool,
) -> String {
    render_with(snapshot, None, width, height, ascii, color, help)
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
    render_with(snapshot, Some(run), width, height, ascii, color, help)
}

fn render_with(
    snapshot: &Snapshot,
    run: Option<&LiveRun>,
    width: u16,
    height: u16,
    ascii: bool,
    color: bool,
    help: bool,
) -> String {
    if width == 0 || height == 0 {
        return String::new();
    }

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal
        .draw(|frame| draw_with(frame, snapshot, run, ascii, color, help))
        .expect("draw frame");

    let buffer = terminal.backend().buffer();
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

pub fn draw(frame: &mut Frame, snapshot: &Snapshot, ascii: bool, color: bool, help: bool) {
    draw_with(frame, snapshot, None, ascii, color, help);
}

pub fn draw_live(
    frame: &mut Frame,
    snapshot: &Snapshot,
    run: &LiveRun,
    ascii: bool,
    color: bool,
    help: bool,
) {
    draw_with(frame, snapshot, Some(run), ascii, color, help);
}

fn draw_with(
    frame: &mut Frame,
    snapshot: &Snapshot,
    run: Option<&LiveRun>,
    ascii: bool,
    color: bool,
    help: bool,
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
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
    let window = match (live.as_ref(), tree.as_ref()) {
        (Some(live), Some(Ok(tree))) => Some(live.window(tree, tree_row_budget(area.height))),
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
    );
    frame.render_widget(Paragraph::new(body), chunks[1]);

    let footer = if ascii {
        "q quit | ? help | r refresh"
    } else {
        "q quit · ? help · r refresh"
    };
    frame.render_widget(Paragraph::new(footer).style(dim(color)), chunks[2]);

    if help {
        draw_help(frame, area, ascii);
    }
}

fn header_lines(
    snapshot: &Snapshot,
    tree: Option<&Result<Tree, TreeError>>,
    live: Option<&LiveView>,
    window: Option<&Window>,
    width: usize,
    ascii: bool,
    color: bool,
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
    let left = graph_title(shown_roots, total_roots);
    let right = match live {
        Some(_) => format!("install {} {separator} live", package_count(packages)),
        None => format!(
            "install {} {separator} static prefetch {separator} ghost",
            package_count(packages)
        ),
    };
    let padding = width.saturating_sub(display_width(&left) + display_width(&right));
    let title = format!("{left}{}{right}", " ".repeat(padding));

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

fn body_lines(
    snapshot: &Snapshot,
    tree: Option<&Result<Tree, TreeError>>,
    live: Option<&LiveView>,
    window: Option<&Window>,
    ascii: bool,
    width: usize,
    color: bool,
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

    if let Some(error) = snapshot.prefetch.error() {
        lines.push(Line::from(cell_text(
            &format!("prefetch: {error}"),
            width,
            ascii,
        )));
    }

    if let Some(report) = snapshot.prefetch.data() {
        let tree = tree.expect("the prefetch tree is built with the report");
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

fn prefetch_lines(
    report: &PrefetchReport,
    tree: &Result<Tree, TreeError>,
    live: Option<&LiveView>,
    window: Option<&Window>,
    at: Option<SystemTime>,
    style: FrameStyle,
) -> Vec<Line<'static>> {
    let FrameStyle {
        ascii,
        width,
        color,
    } = style;
    let mut lines = Vec::new();
    let now = at.unwrap_or(UNIX_EPOCH);
    let separator = if ascii { "|" } else { "·" };

    if report.packages.is_empty() {
        lines.push(Line::from("system current"));
    } else {
        match tree {
            Ok(tree) => {
                let display = tree.display_rows();
                let slots: Vec<WindowSlot> = match window {
                    Some(window) => window.slots.clone(),
                    None => display.iter().map(|row| WindowSlot::Row(row.id)).collect(),
                };
                let geometry = progress_geometry(tree, live, &slots, ascii, width);

                for slot in slots {
                    match slot {
                        WindowSlot::Clipped => {
                            lines.push(clipped_line(width, ascii, color));
                        }
                        WindowSlot::Row(id) => {
                            let row = &tree.rows[id];
                            let prefix = branch_prefix(tree, row, ascii);
                            if row.kind == TreeRowKind::Package {
                                if let Some(live) = live {
                                    if let Some(live_row) = live.row(id) {
                                        match &geometry {
                                            Some(geometry) if geometry.rows.contains(&id) => {
                                                lines.push(progress_row_line(
                                                    row, live_row, &prefix, ascii, width, color,
                                                    geometry,
                                                ));
                                            }
                                            _ => {
                                                lines.push(live_row_line(
                                                    row, live_row, &prefix, ascii, width, color,
                                                ));
                                            }
                                        }
                                        continue;
                                    }
                                }
                            }
                            lines.push(static_row_line(row, &prefix, now, ascii, width, color));
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
    let mut lines = Vec::new();

    lines.push(Line::styled(
        cell_text(
            &format!(
                "ready {} = {} cached + {} built {separator} stale {} {separator} partial {}",
                counts.ready(),
                counts.cached,
                counts.built,
                counts.stale,
                counts.partial
            ),
            width,
            ascii,
        ),
        bold(),
    ));
    let held = counts
        .held
        .map_or_else(String::new, |held| format!(" {separator} held {held}"));
    lines.push(Line::styled(
        cell_text(
            &format!(
                "missing {} {separator} build-pending {} {separator} failed {} {separator} unknown {}{held}",
                counts.missing, counts.build_pending, counts.failed, counts.unknown
            ),
            width,
            ascii,
        ),
        bold(),
    ));
    lines.push(labeled_line(
        "Host     ",
        report.host.as_deref().unwrap_or("unknown"),
        bold(),
        accent(color, HOST),
        width,
        ascii,
    ));
    let bytes: u64 = report
        .packages
        .iter()
        .filter_map(|package| package.download_bytes)
        .sum();
    let sigma = if ascii { "S" } else { "Σ" };
    lines.push(labeled_line(
        &format!("{sigma}        "),
        &format!(
            "{} {separator} {} {separator} {} ready {separator} {} failed",
            package_count(report.counts.total()),
            human_bytes(bytes),
            counts.ready(),
            counts.failed
        ),
        bold(),
        Style::default(),
        width,
        ascii,
    ));
    lines
}

fn live_summary_lines(
    report: &PrefetchReport,
    live: &LiveView,
    ascii: bool,
    width: usize,
    color: bool,
) -> Vec<Line<'static>> {
    let separator = if ascii { "|" } else { "·" };
    vec![
        summary_group_line(
            "Builds",
            live.summary.builds,
            Activity::Build,
            ascii,
            width,
            color,
        ),
        summary_group_line(
            "Downloads",
            live.summary.downloads,
            Activity::Download,
            ascii,
            width,
            color,
        ),
        labeled_line(
            "Host      ",
            report.host.as_deref().unwrap_or("unknown"),
            bold(),
            accent(color, HOST),
            width,
            ascii,
        ),
        sigma_line(live, separator, ascii, width, color),
    ]
}

fn summary_group_line(
    label: &str,
    counts: GroupCounts,
    activity: Activity,
    ascii: bool,
    width: usize,
    color: bool,
) -> Line<'static> {
    let (running, done, planned) = match (activity, ascii) {
        (Activity::Build, false) => ("⏵", "✔", "⏸"),
        (Activity::Build, true) => (">", "+", "="),
        (Activity::Download, false) => ("↓", "✔", "⏸"),
        (Activity::Download, true) => ("v", "+", "="),
    };
    let mut remaining = width;
    let mut spans = Vec::new();
    push_span(
        &mut spans,
        &format!("{label:<10}"),
        bold(),
        &mut remaining,
        ascii,
    );
    push_span(
        &mut spans,
        &format!("{running} {}", counts.running),
        accent_bold(color, ACTIVE),
        &mut remaining,
        ascii,
    );
    push_span(&mut spans, "   ", Style::default(), &mut remaining, ascii);
    push_span(
        &mut spans,
        &format!("{done} {}", counts.done),
        accent_bold(color, DONE),
        &mut remaining,
        ascii,
    );
    push_span(&mut spans, "   ", Style::default(), &mut remaining, ascii);
    push_span(
        &mut spans,
        &format!("{planned} {}", counts.planned),
        accent_bold(color, PLANNED),
        &mut remaining,
        ascii,
    );
    Line::from(spans)
}

fn sigma_line(
    live: &LiveView,
    separator: &str,
    ascii: bool,
    width: usize,
    color: bool,
) -> Line<'static> {
    let mut remaining = width;
    let mut spans = Vec::new();
    let sigma = if ascii { "S" } else { "Σ" };
    push_span(
        &mut spans,
        &format!("{sigma:<10}"),
        bold(),
        &mut remaining,
        ascii,
    );
    push_span(
        &mut spans,
        &package_count(live.summary.total),
        bold(),
        &mut remaining,
        ascii,
    );
    if let Some(timer) = super::live::timer_label(live.summary.elapsed, ascii) {
        push_span(
            &mut spans,
            &format!(" {separator} {timer}"),
            dim(color),
            &mut remaining,
            ascii,
        );
    }
    if live.summary.failed > 0 {
        push_span(
            &mut spans,
            &format!(" {separator} {} failed", live.summary.failed),
            accent_bold(color, FAILURE),
            &mut remaining,
            ascii,
        );
    }
    if live.summary.frozen {
        push_span(
            &mut spans,
            &format!(" {separator} frozen"),
            accent_bold(color, FAILURE),
            &mut remaining,
            ascii,
        );
    }
    Line::from(spans)
}

fn labeled_line(
    label: &str,
    value: &str,
    label_style: Style,
    value_style: Style,
    width: usize,
    ascii: bool,
) -> Line<'static> {
    let mut remaining = width;
    let mut spans = Vec::new();
    push_span(&mut spans, label, label_style, &mut remaining, ascii);
    push_span(&mut spans, value, value_style, &mut remaining, ascii);
    Line::from(spans)
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
}

/// Lay out one tree row. Every column is measured in terminal cells, and the
/// optional size column is right-aligned exactly as the static frame did.
fn row_line(row: &TreeRow, paint: RowPaint<'_>, ascii: bool, width: usize) -> Line<'static> {
    let lead = 2 + display_width(paint.prefix) + display_width(paint.glyph) + 1;
    let name_width = ((width.saturating_sub(lead)) / 3).clamp(12, 36);
    let size_width = if width >= 60 { 8 } else { 0 };
    let state_width = width
        .saturating_sub(lead + name_width + 1 + if size_width > 0 { size_width + 1 } else { 0 });

    let mut remaining = width;
    let mut spans = Vec::new();
    push_span(&mut spans, "  ", Style::default(), &mut remaining, ascii);
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
        },
        ascii,
        width,
    )
}

fn live_row_line(
    row: &TreeRow,
    live: &super::live::LiveRow,
    prefix: &str,
    ascii: bool,
    width: usize,
    color: bool,
) -> Line<'static> {
    let separator = if ascii { "|" } else { "·" };
    let glyph = live_glyph(live, ascii);
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
        },
        ascii,
        width,
    )
}

fn live_glyph(live: &super::live::LiveRow, ascii: bool) -> &'static str {
    let (unicode, fallback) = match live.state {
        LiveState::Planned => ("⏸", "="),
        LiveState::Running => match live.activity {
            Some(Activity::Build) => ("⏵", ">"),
            Some(Activity::Download) => ("↓ ⏵", "v >"),
            None => ("⏵", ">"),
        },
        LiveState::Done => ("✔", "+"),
        LiveState::Failed => ("⚠", "x"),
        LiveState::Blocked => ("■", "!"),
        LiveState::Skipped { .. } => ("⊘", "-"),
    };
    if ascii { fallback } else { unicode }
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
    let mut text = format!("  {prefix}{} {}", live_glyph(live, ascii), row.label);
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
fn progress_row_line(
    row: &TreeRow,
    live: &super::live::LiveRow,
    prefix: &str,
    ascii: bool,
    width: usize,
    color: bool,
    geometry: &ProgressGeometry,
) -> Line<'static> {
    let separator = if ascii { " | " } else { " · " };
    let glyph = live_glyph(live, ascii);
    let hue = live_color(live.state);
    let name_style = accent_bold(color, hue);
    let progress = live.progress.expect("progress rows have bytes");
    let bytes = progress_bytes(progress);

    let mut spans = Vec::new();
    let mut left_remaining = geometry.left_width;
    push_span(
        &mut spans,
        "  ",
        Style::default(),
        &mut left_remaining,
        ascii,
    );
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

    use super::{display_width, render_frame, render_live_frame};
    use crate::frontier::Observed;
    use crate::report::{
        ArtifactState, Cached, PackageView, PrefetchReport, Repo, Section, Snapshot, Update,
    };
    use crate::tui::live::{Activity, LiveRun};
    use crate::tui::tree::Tree;

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
    fn live_frames_read_as_the_frontier_without_invented_state() {
        let snapshot = diamond_snapshot();
        let run = diamond_run();
        let frame = render_live_frame(&snapshot, &run, 120, 40, false, false, false);

        for needle in [
            "⏵ left",
            "✔ base",
            "⏸ right",
            "planned",
            "building",
            "done",
            "waiting for 1 ⏵ 1 ⏸",
            "⏱ 2s",
            "Builds    ⏵ 1   ✔ 0   ⏸ 1",
            "Downloads ↓ 0   ✔ 1   ⏸ 1",
            "Host      mirror.example.org",
            "Σ         4 packages · ⏱ 2s",
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
            frame.contains("⚠ base"),
            "the failed node stays visible\n{frame}"
        );
        assert!(frame.contains("exit 2"), "the evidence is shown\n{frame}");
        for name in ["left", "right", "join"] {
            assert!(
                frame.contains(&format!("■ {name}")),
                "{name} is blocked\n{frame}"
            );
        }
        assert!(
            frame.contains("↓ ⏵ solo"),
            "an independent root keeps running\n{frame}"
        );
        assert!(frame.contains("1 failed"));
        assert!(frame.contains("frozen"));
        assert_eq!(frame.matches("blocked").count(), 3);
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
        assert!(row.contains("⊘"), "skipped glyph\n{frame}");
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
        assert!(row.contains("v > linux"), "the download glyph is ASCII");
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
            wide.contains("Dependency Graph showing 11 of 197 roots:"),
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
            compact.contains("Dependency Graph showing 6 of 197 roots:"),
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
            text_column(&buffer, 120, planned, "⏸").expect("the planned glyph is on the row");
        assert_eq!(
            cell_at(&buffer, 120, planned, planned_glyph).fg,
            Color::Blue,
            "planned is blue"
        );

        let running = find_row(&buffer, 120, "building");
        let glyph =
            text_column(&buffer, 120, running, "⏵").expect("the running glyph is on the row");
        assert_eq!(cell_at(&buffer, 120, running, glyph).fg, Color::Yellow);
        let name = text_column(&buffer, 120, running, "left").expect("the running name");
        assert!(
            cell_at(&buffer, 120, running, name)
                .modifier
                .contains(Modifier::BOLD)
        );

        let done = find_row(&buffer, 120, "✔");
        let done_glyph = text_column(&buffer, 120, done, "✔").expect("done glyph");
        assert_eq!(cell_at(&buffer, 120, done, done_glyph).fg, Color::Green);

        let host = find_row(&buffer, 120, "Host");
        let host_column = text_column(&buffer, 120, host, "mirror").expect("host value");
        assert_eq!(
            cell_at(&buffer, 120, host, host_column).fg,
            Color::Magenta,
            "the host is magenta"
        );
        assert!(
            cell_at(&buffer, 120, host, 0)
                .modifier
                .contains(Modifier::BOLD),
            "summary labels are bold"
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
            text_column(&roots, 120, download, "↓").expect("the download glyph is on the row");
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
        let warning = text_column(&failure, 120, failed, "⚠").expect("the failure glyph");
        assert_eq!(
            cell_at(&failure, 120, failed, warning).fg,
            Color::Red,
            "a failure is red"
        );
        let blocked = find_row(&failure, 120, "blocked");
        let square = text_column(&failure, 120, blocked, "■").expect("the blocked glyph");
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
        let skipped_glyph = text_column(&skipped, 80, skipped_row, "⊘").expect("the skipped glyph");
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
        assert!(frame.contains("ready 1 = 1 cached + 0 built"));
        assert!(frame.contains("Host     mirror.example.org"));
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
        assert!(frame.contains("│"), "a lane continues\n{frame}");
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
    }
}
