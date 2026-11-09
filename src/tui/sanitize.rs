//! Text gates for every string that can reach a terminal cell.
//!
//! Package names, versions, paths and child output are untrusted. Escape
//! sequences can move the cursor or write the clipboard; bidi controls can
//! reorder a path. A cell gets only clean, width-measured text.

use unicode_width::UnicodeWidthChar;

/// Remove escape sequences, control characters and bidi controls.
pub fn sanitize_cell(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            },
            ch if ch.is_control() => {}
            ch if is_bidi(ch) => {}
            _ => out.push(ch),
        }
    }
    out
}

/// Replace every non-ASCII character with `?` for `--ascii`.
pub fn to_ascii(input: &str) -> String {
    input
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { '?' })
        .collect()
}

/// Truncate to a display width, appending a marker only when it fits.
pub fn truncate_width(input: &str, max: usize, unicode: bool) -> String {
    if max == 0 {
        return String::new();
    }

    let total: usize = input
        .chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum();
    if total <= max {
        return input.to_string();
    }

    let marker = if unicode { "…" } else { "..." };
    let marker_width = if unicode { 1 } else { 3 };
    let marker_fits = max >= marker_width;
    let budget = if marker_fits { max - marker_width } else { max };

    let mut out = String::new();
    let mut width = 0;
    for ch in input.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + char_width > budget {
            break;
        }
        out.push(ch);
        width += char_width;
    }
    if marker_fits {
        out.push_str(marker);
    }
    out
}

/// The single gate used by the renderer.
pub fn cell_text(input: &str, max: usize, ascii: bool) -> String {
    let clean = sanitize_cell(input);
    if ascii {
        truncate_width(&to_ascii(&clean), max, false)
    } else {
        truncate_width(&clean, max, true)
    }
}

fn is_bidi(ch: char) -> bool {
    matches!(
        ch,
        '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::{cell_text, sanitize_cell, to_ascii, truncate_width};

    #[test]
    fn strips_csi_sequences() {
        assert_eq!(sanitize_cell("a\u{1b}[31mred\u{1b}[0m"), "ared");
    }

    #[test]
    fn strips_osc_sequences() {
        assert_eq!(sanitize_cell("a\u{1b}]8;;https://example.test\u{7}b"), "ab");
    }

    #[test]
    fn strips_control_and_bidi_characters() {
        assert_eq!(sanitize_cell("a\u{0}b\u{202e}c\u{7f}d"), "abcd");
    }

    #[test]
    fn truncation_counts_cells_not_chars() {
        assert_eq!(truncate_width("界界", 3, true), "界…");
        assert_eq!(truncate_width("abcde", 4, true), "abc…");
        assert_eq!(truncate_width("abcde", 4, false), "a...");
        assert_eq!(truncate_width("abc", 3, true), "abc");
        assert_eq!(truncate_width("abc", 2, false), "ab");
    }

    #[test]
    fn ascii_mode_replaces_wide_text() {
        assert_eq!(to_ascii("café"), "caf?");
        assert_eq!(cell_text("界", 4, true), "?");
    }
}
