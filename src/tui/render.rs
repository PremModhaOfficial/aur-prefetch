//! The T1 tree frame. The data comes from the report layer; this module only
//! lays it out and never shells out.

use ratatui::backend::TestBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};

use crate::report::{Snapshot, Update};

use super::sanitize::cell_text;

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

    let pending = pending_count(snapshot);
    let separator = if ascii { "|" } else { "·" };
    let header = format!(
        "aur-prefetch {separator} {pending} pending {separator} {} action(s)",
        snapshot.actionable
    );
    let header = cell_text(&header, chunks[0].width as usize, ascii);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().add_modifier(Modifier::BOLD)),
        chunks[0],
    );

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

    lines.push(Line::from(format!("install {} packages", all.len())));
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
    use crate::report::{Cached, Repo, Section, Snapshot, Update};

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
    }
}
