use std::env;
use std::io::{self, IsTerminal};

use terminal_size::{Height, Width, terminal_size};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCapabilities {
    pub interactive: bool,
    pub color: bool,
    pub unicode: bool,
    pub width: usize,
    pub height: usize,
}

impl TerminalCapabilities {
    #[must_use]
    pub fn detect() -> Self {
        let interactive =
            io::stdout().is_terminal() && env::var_os("TERM").is_none_or(|term| term != "dumb");
        let (width, height) = dimensions(interactive);
        Self {
            interactive,
            color: interactive && env::var_os("NO_COLOR").is_none(),
            unicode: interactive,
            width,
            height,
        }
    }

    #[must_use]
    pub fn current(self) -> Self {
        let (width, height) = dimensions(self.interactive);
        Self {
            width,
            height,
            ..self
        }
    }
}

fn dimensions(interactive: bool) -> (usize, usize) {
    if interactive {
        if let Some((Width(width), Height(height))) = terminal_size() {
            if width > 0 && height > 0 {
                return (usize::from(width), usize::from(height));
            }
        }
    }
    (80, 24)
}

#[derive(Clone, Copy, Debug)]
pub enum Tone {
    Accent,
    Danger,
    Method,
    Muted,
    Success,
    Warning,
}

#[derive(Clone, Copy, Debug)]
pub struct TableColumn<'a> {
    pub heading: &'a str,
    pub min_width: usize,
    pub flexible: bool,
    pub tone: Option<Tone>,
}

impl<'a> TableColumn<'a> {
    #[must_use]
    pub const fn new(heading: &'a str, min_width: usize) -> Self {
        Self {
            heading,
            min_width,
            flexible: false,
            tone: None,
        }
    }

    #[must_use]
    pub const fn flexible(mut self) -> Self {
        self.flexible = true;
        self
    }

    #[must_use]
    pub const fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }
}

#[derive(Clone, Debug)]
pub struct OutputRenderer {
    pub capabilities: TerminalCapabilities,
}

impl OutputRenderer {
    #[must_use]
    pub const fn new(capabilities: TerminalCapabilities) -> Self {
        Self { capabilities }
    }

    #[must_use]
    pub fn heading(&self, title: &str, detail: Option<&str>) -> String {
        let text = detail.map_or_else(
            || title.to_uppercase(),
            |detail| format!("{}  {detail}", title.to_uppercase()),
        );
        self.decorate(&text, Tone::Accent, true)
    }

    #[must_use]
    pub fn section(&self, title: &str) -> String {
        self.decorate(title, Tone::Accent, true)
    }

    #[must_use]
    pub fn success(&self, message: &str) -> String {
        let symbol = if self.capabilities.unicode {
            "✓"
        } else {
            "OK"
        };
        format!("{} {message}", self.decorate(symbol, Tone::Success, true))
    }

    #[must_use]
    pub fn details<'a>(&self, entries: impl IntoIterator<Item = (&'a str, String)>) -> Vec<String> {
        let entries: Vec<_> = entries.into_iter().collect();
        let label_width = entries
            .iter()
            .map(|(label, _)| visible_width(label))
            .max()
            .unwrap_or(0);
        entries
            .into_iter()
            .map(|(label, value)| format!("  {}  {value}", pad(label, label_width)))
            .collect()
    }

    #[must_use]
    pub fn table(&self, columns: &[TableColumn<'_>], rows: &[Vec<String>]) -> Vec<String> {
        if columns.is_empty() {
            return Vec::new();
        }
        let separator_width = 2;
        let minimum_width = columns
            .iter()
            .map(|column| column.min_width.max(visible_width(column.heading)).max(1))
            .sum::<usize>()
            + separator_width * columns.len().saturating_sub(1);

        if self.capabilities.width < minimum_width {
            let mut result = Vec::new();
            for (row_index, row) in rows.iter().enumerate() {
                for (index, column) in columns.iter().enumerate() {
                    result.push(truncate(
                        &format!(
                            "{}: {}",
                            column.heading,
                            row.get(index).map_or("", String::as_str)
                        ),
                        self.capabilities.width,
                        self.capabilities.unicode,
                    ));
                }
                if row_index + 1 < rows.len() {
                    result.push(String::new());
                }
            }
            return result;
        }

        let available = self
            .capabilities
            .width
            .saturating_sub(separator_width * columns.len().saturating_sub(1))
            .max(columns.len());
        let mut widths: Vec<usize> = columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                rows.iter()
                    .filter_map(|row| row.get(index))
                    .map(|cell| visible_width(cell))
                    .chain([visible_width(column.heading), column.min_width, 1])
                    .max()
                    .unwrap_or(1)
            })
            .collect();
        let mut overflow = widths.iter().sum::<usize>().saturating_sub(available);
        shrink_columns(&mut widths, columns, &mut overflow, true);
        shrink_columns(&mut widths, columns, &mut overflow, false);

        let headings = columns
            .iter()
            .map(|column| column.heading.to_owned())
            .collect::<Vec<_>>();
        let mut result = vec![self.decorate(
            &render_row(&headings, columns, &widths, self, false),
            Tone::Muted,
            true,
        )];
        result.extend(
            rows.iter()
                .map(|row| render_row(row, columns, &widths, self, true)),
        );
        result
    }

    #[must_use]
    pub fn decorate(&self, value: &str, tone: Tone, bold: bool) -> String {
        if !self.capabilities.color {
            return value.to_owned();
        }
        let color = match tone {
            Tone::Accent => "\x1b[36m",
            Tone::Danger => "\x1b[31m",
            Tone::Method => "\x1b[34m",
            Tone::Muted => "\x1b[90m",
            Tone::Success => "\x1b[32m",
            Tone::Warning => "\x1b[33m",
        };
        let bold = if bold { "\x1b[1m" } else { "" };
        format!("{bold}{color}{value}\x1b[0m")
    }
}

fn shrink_columns(
    widths: &mut [usize],
    columns: &[TableColumn<'_>],
    overflow: &mut usize,
    flexible_only: bool,
) {
    while *overflow > 0 {
        let mut changed = false;
        for index in (0..columns.len()).rev() {
            let minimum = columns[index].min_width.max(1);
            if (!flexible_only || columns[index].flexible) && widths[index] > minimum {
                widths[index] -= 1;
                *overflow -= 1;
                changed = true;
                if *overflow == 0 {
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn render_row(
    cells: &[String],
    columns: &[TableColumn<'_>],
    widths: &[usize],
    renderer: &OutputRenderer,
    decorate: bool,
) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let width = widths.get(index).copied().unwrap_or(1);
            let value = truncate(cell, width, renderer.capabilities.unicode);
            let value = if index + 1 == cells.len() {
                value
            } else {
                pad(&value, width)
            };
            if decorate {
                if let Some(tone) = columns.get(index).and_then(|column| column.tone) {
                    return renderer.decorate(&value, tone, matches!(tone, Tone::Method));
                }
            }
            value
        })
        .collect::<Vec<_>>()
        .join("  ")
}

#[must_use]
pub fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(character);
        }
    }
    result
}

/// Makes untrusted human-readable text safe to write to a terminal.
///
/// The renderer's own SGR color sequences are retained, but cursor movement,
/// OSC commands (including clipboard operations), line controls, and other
/// C0/C1 controls are rendered visibly instead of being executed. Structured
/// JSON and JSONL output bypass this helper and therefore keep their original
/// data values.
#[must_use]
pub fn sanitize_terminal_text(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if character == '\x1b' {
            if let Some(length) = sgr_sequence_length(&value[index..]) {
                result.push_str(&value[index..index + length]);
                while characters
                    .peek()
                    .is_some_and(|(next, _)| *next < index + length)
                {
                    let _ = characters.next();
                }
            } else {
                result.push_str("\\x1b");
            }
            continue;
        }
        match character {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\u{7f}' => result.push_str("\\x7f"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(result, "\\u{{{:04x}}}", u32::from(character));
            }
            _ => result.push(character),
        }
    }
    result
}

fn sgr_sequence_length(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    if bytes.get(..2) != Some(b"\x1b[") {
        return None;
    }
    for (index, byte) in bytes.iter().copied().enumerate().skip(2).take(32) {
        if byte == b'm' {
            return Some(index + 1);
        }
        if !byte.is_ascii_digit() && byte != b';' {
            return None;
        }
    }
    None
}

#[must_use]
pub fn visible_width(value: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(value).as_str())
}

fn truncate(value: &str, width: usize, unicode: bool) -> String {
    if visible_width(value) <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let suffix = if unicode { "…" } else { "." };
    let content_width = width.saturating_sub(visible_width(suffix));
    let mut result = String::new();
    for grapheme in value.graphemes(true) {
        if visible_width(&format!("{result}{grapheme}")) > content_width {
            break;
        }
        result.push_str(grapheme);
    }
    result.push_str(suffix);
    result
}

fn pad(value: &str, width: usize) -> String {
    format!(
        "{value}{}",
        " ".repeat(width.saturating_sub(visible_width(value)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(width: usize) -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: true,
            color: false,
            unicode: true,
            width,
            height: 24,
        }
    }

    #[test]
    fn measures_unicode_display_width() {
        assert_eq!(visible_width("abc"), 3);
        assert_eq!(visible_width("受信"), 4);
        assert_eq!(visible_width("e\u{301}"), 1);
        assert_eq!(visible_width("👨‍👩‍👧‍👦"), 2);
    }

    #[test]
    fn table_falls_back_to_vertical_rows() {
        let renderer = OutputRenderer::new(capabilities(12));
        let lines = renderer.table(
            &[
                TableColumn::new("METHOD", 1),
                TableColumn::new("PATH", 1),
                TableColumn::new("TYPE", 1),
            ],
            &[vec![
                "POST".into(),
                "/long/webhook/path".into(),
                "application/json".into(),
            ]],
        );
        assert!(lines.iter().any(|line| line.starts_with("METHOD:")));
        assert!(lines.iter().all(|line| visible_width(line) <= 12));
    }

    #[test]
    fn ansi_stripping_preserves_text() {
        assert_eq!(strip_ansi("\x1b[1m\x1b[32mOK\x1b[0m"), "OK");
    }

    #[test]
    fn terminal_sanitizer_blocks_cursor_osc_and_control_injection() {
        let malicious = "safe\x1b[2J\x1b]52;c;Y2xpcGJvYXJk\x07\rnext\tvalue";
        let sanitized = sanitize_terminal_text(malicious);
        assert_eq!(
            sanitized,
            "safe\\x1b[2J\\x1b]52;c;Y2xpcGJvYXJk\\u{0007}\\rnext\\tvalue"
        );
        assert!(!sanitized.chars().any(char::is_control));
    }

    #[test]
    fn terminal_sanitizer_preserves_renderer_sgr_only() {
        assert_eq!(
            sanitize_terminal_text("\x1b[1m\x1b[36mBarestash\x1b[0m"),
            "\x1b[1m\x1b[36mBarestash\x1b[0m"
        );
    }
}
