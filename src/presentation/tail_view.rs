use std::io::{self, Write};

use crate::application::CliError;
use crate::presentation::renderer::{
    OutputRenderer, TableColumn, TerminalCapabilities, Tone, sanitize_terminal_text, visible_width,
};
use crate::protocol::EventMetadata;

const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";
const RESTORE_TERMINAL: &str = "\x1b[0m\x1b[K\r\n";
const FULL_TABLE_MINIMUM_WIDTH: usize = 66;

pub struct TailView {
    endpoint_id: String,
    base_capabilities: TerminalCapabilities,
    events: Vec<EventMetadata>,
    row_limit: usize,
    received_count: usize,
    started: bool,
    stopped: bool,
}

impl TailView {
    #[must_use]
    pub fn new(endpoint_id: impl Into<String>, capabilities: TerminalCapabilities) -> Self {
        Self {
            endpoint_id: endpoint_id.into(),
            base_capabilities: capabilities,
            events: Vec::new(),
            row_limit: 10,
            received_count: 0,
            started: false,
            stopped: false,
        }
    }

    pub fn start(&mut self) -> Result<(), CliError> {
        self.started = true;
        self.render()
    }

    pub fn add(&mut self, event: EventMetadata) -> Result<(), CliError> {
        self.started = true;
        self.received_count += 1;
        self.events.insert(0, event);
        self.events.truncate(self.row_limit);
        self.render()
    }

    pub fn stop(&mut self) -> Result<(), CliError> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        self.stop_to(&mut output)
    }

    fn stop_to(&mut self, output: &mut impl Write) -> Result<(), CliError> {
        if !self.started || self.stopped {
            return Ok(());
        }
        self.stopped = true;
        output.write_all(RESTORE_TERMINAL.as_bytes())?;
        output.flush()?;
        Ok(())
    }

    fn render(&self) -> Result<(), CliError> {
        let capabilities = self.base_capabilities.current();
        let width = capabilities.width.max(1);
        let height = capabilities.height.max(1);
        let mut displayed = self.events.len();
        let lines = loop {
            let candidate = self.render_lines(capabilities, displayed);
            if frame_height(&candidate, width) <= height || displayed == 0 {
                break candidate;
            }
            displayed -= 1;
        };
        let fitted = fit_lines(&lines, width, height);
        let frame = compose_frame(&fitted);
        let stdout = io::stdout();
        let mut output = stdout.lock();
        output.write_all(frame.as_bytes())?;
        output.flush()?;
        Ok(())
    }

    fn render_lines(&self, capabilities: TerminalCapabilities, displayed: usize) -> Vec<String> {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.heading("Barestash", Some(&self.endpoint_id)),
            String::new(),
        ];
        lines.extend(
            renderer.details([
                (
                    "Status",
                    renderer.decorate("● watching", Tone::Success, true),
                ),
                ("Requests", self.received_count.to_string()),
                (
                    "Last event",
                    self.events
                        .first()
                        .map_or_else(|| "waiting".into(), |event| event.received_at.clone()),
                ),
            ]),
        );
        lines.push(String::new());
        if self.events.is_empty() {
            lines.push(renderer.decorate("Waiting for webhook events…", Tone::Muted, false));
            return lines;
        }
        let compact = capabilities.width < FULL_TABLE_MINIMUM_WIDTH;
        let columns = if compact {
            vec![
                TableColumn::new("TIME", 8),
                TableColumn::new("METHOD", 6).tone(Tone::Method),
                TableColumn::new("PATH", 12).flexible(),
                TableColumn::new("EVENT", 12).flexible(),
            ]
        } else {
            vec![
                TableColumn::new("TIME", 8),
                TableColumn::new("METHOD", 6).tone(Tone::Method),
                TableColumn::new("PATH", 12).flexible(),
                TableColumn::new("SIZE", 6),
                TableColumn::new("CONTENT-TYPE", 10).flexible(),
                TableColumn::new("EVENT", 12).flexible(),
            ]
        };
        let rows = self
            .events
            .iter()
            .take(displayed)
            .map(|event| {
                let time = event.received_at.get(11..19).unwrap_or(&event.received_at);
                if compact {
                    vec![
                        time.into(),
                        event.method.clone(),
                        event.request_path.clone(),
                        event.id.clone(),
                    ]
                } else {
                    vec![
                        time.into(),
                        event.method.clone(),
                        event.request_path.clone(),
                        format_bytes(event.body.size),
                        event
                            .headers
                            .get("content-type")
                            .cloned()
                            .unwrap_or_else(|| "-".into()),
                        event.id.clone(),
                    ]
                }
            })
            .collect::<Vec<_>>();
        lines.extend(renderer.table(&columns, &rows));
        lines
    }
}

fn compose_frame(lines: &[String]) -> String {
    let safe_lines = lines
        .iter()
        .map(|line| sanitize_terminal_text(line))
        .collect::<Vec<_>>();
    format!("{CLEAR_SCREEN}{}", safe_lines.join("\n"))
}

impl Drop for TailView {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn format_bytes(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    }
}

fn line_height(line: &str, width: usize) -> usize {
    visible_width(line).div_ceil(width).max(1)
}

fn frame_height(lines: &[String], width: usize) -> usize {
    lines.iter().map(|line| line_height(line, width)).sum()
}

fn fit_lines(lines: &[String], width: usize, height: usize) -> Vec<String> {
    let mut used = 0;
    lines
        .iter()
        .take_while(|line| {
            let next = used + line_height(line, width);
            if next > height {
                false
            } else {
                used = next;
                true
            }
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::protocol::EventBodyMetadata;

    use super::*;

    fn capabilities(width: usize, height: usize) -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: true,
            color: false,
            unicode: true,
            width,
            height,
        }
    }

    fn event(id: &str, path: &str) -> EventMetadata {
        EventMetadata {
            id: id.into(),
            endpoint_id: "ep_test".into(),
            received_at: "2026-08-13T01:02:03.000Z".into(),
            method: "POST".into(),
            request_path: path.into(),
            query: BTreeMap::new(),
            headers: BTreeMap::from([("content-type".into(), "application/json".into())]),
            body: EventBodyMetadata {
                size: 42,
                sha256: "sha256".into(),
                available: true,
                url: None,
            },
        }
    }

    #[test]
    fn compact_layout_drops_size_and_content_type_columns() {
        let mut view = TailView::new("ep_test", capabilities(65, 24));
        view.events.push(event("evt_test", "/hook"));
        let lines = view.render_lines(capabilities(65, 24), 1).join("\n");
        assert!(lines.contains("METHOD"));
        assert!(!lines.contains("CONTENT-TYPE"));
        assert!(!lines.contains("SIZE"));

        let full = view.render_lines(capabilities(100, 24), 1).join("\n");
        assert!(full.contains("CONTENT-TYPE"));
        assert!(full.contains("SIZE"));
    }

    #[test]
    fn fitted_frame_never_exceeds_terminal_height() {
        let lines = vec!["123456789".into(), "abcdefghij".into(), "last".into()];
        let fitted = fit_lines(&lines, 5, 3);
        assert!(frame_height(&fitted, 5) <= 3);
        assert_eq!(fitted, vec!["123456789"]);
    }

    #[test]
    fn frame_blocks_untrusted_terminal_commands_but_keeps_own_clear_prefix() {
        let frame = compose_frame(&["/hook\x1b]52;c;c2VjcmV0\x07\x1b[2J".into()]);
        assert!(frame.starts_with(CLEAR_SCREEN));
        assert_eq!(frame.matches(CLEAR_SCREEN).count(), 1);
        assert!(frame.contains("\\x1b]52"));
        assert!(frame.contains("\\x1b[2J"));
    }

    #[test]
    fn terminal_restoration_is_emitted_exactly_once_after_start() {
        let mut view = TailView::new("ep_test", capabilities(80, 24));
        let mut output = Vec::new();
        view.stop_to(&mut output)
            .unwrap_or_else(|error| panic!("stop: {error}"));
        assert!(output.is_empty());
        view.started = true;
        view.stop_to(&mut output)
            .unwrap_or_else(|error| panic!("stop: {error}"));
        view.stop_to(&mut output)
            .unwrap_or_else(|error| panic!("stop: {error}"));
        assert_eq!(output, RESTORE_TERMINAL.as_bytes());
    }
}
