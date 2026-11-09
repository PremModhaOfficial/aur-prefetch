//! The T1 tree frame. The data comes from the report layer; this module only
//! lays it out and never shells out.

use ratatui::backend::TestBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use std::time::{SystemTime, UNIX_EPOCH};

use crate::report::{ArtifactState, PrefetchReport, Snapshot, Update};
use crate::util::human_bytes;

use super::sanitize::{cell_text, display_width};
use super::tree::{Tree, TreeError, TreeRow};

pub fn render_frame(
    snapshot: &Snapshot,
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
        .draw(|frame| draw(frame, snapshot, ascii, color, help))
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

    let header = header_lines(snapshot, tree.as_ref(), chunks[0].width as usize, ascii);
    frame.render_widget(Paragraph::new(header), chunks[0]);

    let body = body_lines(snapshot, tree.as_ref(), ascii, chunks[1].width as usize);
    frame.render_widget(Paragraph::new(body), chunks[1]);

    let footer = if ascii {
        "q quit | ? help | r refresh"
    } else {
        "q quit · ? help · r refresh"
    };
    let footer_style = if color {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };
    frame.render_widget(Paragraph::new(footer).style(footer_style), chunks[2]);

    if help {
        draw_help(frame, area, ascii);
    }
}

fn header_lines(
    snapshot: &Snapshot,
    tree: Option<&Result<Tree, TreeError>>,
    width: usize,
    ascii: bool,
) -> Vec<Line<'static>> {
    let separator = if ascii { "|" } else { "·" };
    let Some(report) = snapshot.prefetch.data() else {
        let pending = pending_count(snapshot);
        let header = format!(
            "aur-prefetch {separator} {pending} pending {separator} {} action(s)",
            snapshot.actionable
        );
        return vec![Line::styled(
            cell_text(&header, width, ascii),
            Style::default().add_modifier(Modifier::BOLD),
        )];
    };

    let counts = &report.counts;
    let header = format!(
        "aur-prefetch {separator} Progress {separator} {} pending {separator} {} prefetched {separator} {} built",
        counts.total(),
        counts.ready(),
        counts.built
    );
    let roots = match tree {
        Some(Ok(tree)) => tree.roots.len(),
        _ => 0,
    };
    let packages = match tree {
        Some(Ok(tree)) => tree.pending_count(),
        _ => counts.total(),
    };
    // T2a has no viewport, so every root fits. The clipped form is kept for
    // the viewport slice and tested directly.
    let left = graph_title(roots, roots);
    let right = format!(
        "install {} {separator} static prefetch {separator} ghost",
        package_count(packages)
    );
    let padding = width.saturating_sub(display_width(&left) + display_width(&right));
    let title = format!("{left}{}{right}", " ".repeat(padding));

    vec![
        Line::styled(
            cell_text(&header, width, ascii),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            cell_text(&title, width, ascii),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ]
}

/// Nom's graph header. T2a shows every root; the clipped form is ready for the
/// viewport slice.
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

fn body_lines(
    snapshot: &Snapshot,
    tree: Option<&Result<Tree, TreeError>>,
    ascii: bool,
    width: usize,
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
            snapshot.prefetch.at(),
            ascii,
            width,
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
    at: Option<SystemTime>,
    ascii: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let now = at.unwrap_or(UNIX_EPOCH);
    let separator = if ascii { "|" } else { "·" };
    let total = report.counts.total();

    if report.packages.is_empty() {
        lines.push(Line::from("system current"));
    } else {
        match tree {
            Ok(tree) => {
                for row in tree.display_rows() {
                    let prefix = branch_prefix(tree, row, ascii);
                    let text = package_row(row, &prefix, now, ascii, width);
                    lines.push(Line::from(cell_text(&text, width, ascii)));
                }
            }
            Err(error) => lines.push(Line::from(cell_text(
                &format!("error: {error}"),
                width,
                ascii,
            ))),
        }
    }

    let counts = &report.counts;
    lines.push(Line::from(cell_text(
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
    )));
    let held = counts
        .held
        .map_or_else(String::new, |held| format!(" {separator} held {held}"));
    lines.push(Line::from(cell_text(
        &format!(
            "missing {} {separator} build-pending {} {separator} failed {} {separator} unknown {}{held}",
            counts.missing, counts.build_pending, counts.failed, counts.unknown
        ),
        width,
        ascii,
    )));
    let host = report.host.as_deref().unwrap_or("unknown");
    lines.push(Line::from(cell_text(
        &format!("Host     {host}"),
        width,
        ascii,
    )));
    let bytes: u64 = report
        .packages
        .iter()
        .filter_map(|package| package.download_bytes)
        .sum();
    lines.push(Line::from(cell_text(
        &format!(
            "{}        {} {separator} {} {separator} {} ready {separator} {} failed",
            if ascii { "S" } else { "Σ" },
            package_count(total),
            human_bytes(bytes),
            counts.ready(),
            counts.failed
        ),
        width,
        ascii,
    )));
    for note in &report.notes {
        lines.push(Line::from(cell_text(
            &format!("note: {note}"),
            width,
            ascii,
        )));
    }
    lines
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

fn package_row(row: &TreeRow, prefix: &str, now: SystemTime, ascii: bool, width: usize) -> String {
    let separator = if ascii { "|" } else { "·" };
    let artifact = row.artifact.unwrap_or(ArtifactState::Unknown);
    let (glyph, label) = artifact_glyph(artifact, ascii);
    let mut suffix = label.to_string();
    if let Some(detail) = row.detail.as_deref() {
        if !detail.is_empty() && detail != label {
            suffix.push(' ');
            suffix.push_str(separator);
            suffix.push(' ');
            suffix.push_str(detail);
        }
    }
    if matches!(artifact, ArtifactState::Cached | ArtifactState::Built) {
        if let Some(age) = age_label(row.artifact_at, now) {
            suffix.push(' ');
            suffix.push_str(separator);
            suffix.push(' ');
            suffix.push_str(&age);
        }
    }
    let size = row.size.map_or_else(String::new, human_bytes);
    let lead = 2 + display_width(prefix) + display_width(glyph) + 1;
    let name_width = ((width.saturating_sub(lead)) / 3).clamp(12, 36);
    let size_width = if width >= 60 { 8 } else { 0 };
    let state_width = width
        .saturating_sub(lead + name_width + 1 + if size_width > 0 { size_width + 1 } else { 0 });
    let base = format!(
        "  {prefix}{glyph} {} {}",
        fit(&row.label, name_width, ascii),
        fit(&suffix, state_width, ascii)
    );
    if size_width > 0 {
        format!("{base} {}", fit_right(&size, size_width, ascii))
    } else {
        base
    }
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

    use super::render_frame;
    use crate::report::{
        ArtifactState, Cached, PackageView, PrefetchReport, Repo, Section, Snapshot, Update,
    };

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
    }
}
