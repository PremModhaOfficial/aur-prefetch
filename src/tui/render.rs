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

use crate::report::{ArtifactState, PackageView, PrefetchReport, Snapshot, Update};
use crate::util::human_bytes;

use super::sanitize::{cell_text, display_width};

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

    let header = header_lines(snapshot, chunks[0].width as usize, ascii);
    frame.render_widget(Paragraph::new(header), chunks[0]);

    let body = body_lines(snapshot, ascii, chunks[1].width as usize);
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

fn header_lines(snapshot: &Snapshot, width: usize, ascii: bool) -> Vec<Line<'static>> {
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
    let left = "Dependency tree";
    let right = if ascii {
        "static prefetch | ghost"
    } else {
        "static prefetch · ghost"
    };
    let padding = width.saturating_sub(left.chars().count() + right.chars().count());
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

fn body_lines(snapshot: &Snapshot, ascii: bool, width: usize) -> Vec<Line<'static>> {
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
        lines.extend(prefetch_lines(report, snapshot.prefetch.at(), ascii, width));
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
        let root = if ascii {
            format!("v install {}", package_count(total))
        } else {
            format!("▾ install {}", package_count(total))
        };
        let ghost = if ascii {
            "ghost | read-only"
        } else {
            "ghost · read-only"
        };
        let padding = width.saturating_sub(root.chars().count() + ghost.chars().count());
        lines.push(Line::from(cell_text(
            &format!("{root}{}{ghost}", " ".repeat(padding)),
            width,
            ascii,
        )));

        let mut packages: Vec<&PackageView> = report.packages.iter().collect();
        packages.sort_by(|left, right| {
            left.artifact
                .rank()
                .cmp(&right.artifact.rank())
                .then_with(|| left.name.cmp(&right.name))
        });
        let last = packages.len().saturating_sub(1);
        for (index, package) in packages.into_iter().enumerate() {
            let prefix = if ascii {
                if index == last { "`-- " } else { "|-- " }
            } else if index == last {
                "└── "
            } else {
                "├── "
            };
            lines.push(Line::from(cell_text(
                &package_row(package, prefix, now, ascii, width),
                width,
                ascii,
            )));
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

fn package_count(count: usize) -> String {
    if count == 1 {
        "1 package".to_string()
    } else {
        format!("{count} packages")
    }
}

fn package_row(
    package: &PackageView,
    prefix: &str,
    now: SystemTime,
    ascii: bool,
    width: usize,
) -> String {
    let separator = if ascii { "|" } else { "·" };
    let (glyph, label) = artifact_glyph(package.artifact, ascii);
    let mut suffix = label.to_string();
    if let Some(detail) = package.detail.as_deref() {
        if !detail.is_empty() && detail != label {
            suffix.push(' ');
            suffix.push_str(separator);
            suffix.push(' ');
            suffix.push_str(detail);
        }
    }
    if matches!(
        package.artifact,
        ArtifactState::Cached | ArtifactState::Built
    ) {
        if let Some(age) = age_label(package.artifact_at, now) {
            suffix.push(' ');
            suffix.push_str(separator);
            suffix.push(' ');
            suffix.push_str(&age);
        }
    }
    let size = package.download_bytes.map_or_else(String::new, human_bytes);
    let version = if package.available.is_empty() {
        String::new()
    } else {
        format!(" {}", package.available)
    };
    let name = format!("{}{version}", package.name);
    let name_width = (width / 3).clamp(12, 36);
    let size_width = if width >= 60 { 8 } else { 0 };
    let state_width =
        width.saturating_sub(8 + name_width + 1 + if size_width > 0 { size_width + 1 } else { 0 });
    let base = format!(
        "  {prefix}{glyph} {} {}",
        fit(&name, name_width, ascii),
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
            depends: Vec::new(),
        }
    }

    fn prefetch_snapshot() -> Snapshot {
        let now = at();
        let packages = vec![
            prefetch_package(
                "bash",
                "5.2.037-2",
                ArtifactState::Cached,
                "cached",
                now - Duration::from_secs(240),
            ),
            prefetch_package(
                "linux",
                "6.17.2.arch1-1",
                ArtifactState::Stale,
                "older cache file 6.16.1-1",
                now - Duration::from_secs(2 * 86_400),
            ),
            prefetch_package(
                "yay-bin",
                "12.4.1-1",
                ArtifactState::BuildPending,
                "build pending",
                now,
            ),
            prefetch_package(
                "simapp",
                "2.0.0-1",
                ArtifactState::Missing,
                "not in cache",
                now,
            ),
        ];
        let report =
            PrefetchReport::new(packages, Some("mirror.example.org".to_string()), Vec::new());
        Snapshot {
            prefetch: Section::ready(report, now),
            actionable: 1,
            ..Snapshot::default()
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
