use barestash_protocol::{EventDetail, EventMetadata, HeaderMap};

use super::{TransformedBody, redact_headers_for_display};
use crate::output::{
    OutputRenderer, PresentationError, TableColumn, TerminalCapabilities, Tone, print_lines,
};

pub fn print_event_list(events: &[EventMetadata]) -> Result<(), PresentationError> {
    print_lines(event_list_lines(events, TerminalCapabilities::detect()))
}

pub fn print_event_summary(event: &EventMetadata) -> Result<(), PresentationError> {
    print_lines(event_summary_lines(event, TerminalCapabilities::detect()))
}

pub fn print_event_detail(
    event: &EventDetail,
    body: Option<&TransformedBody>,
) -> Result<(), PresentationError> {
    print_lines(event_detail_lines(
        event,
        body,
        TerminalCapabilities::detect(),
    )?)
}

pub fn print_event_headers(event: &EventDetail) -> Result<(), PresentationError> {
    print_lines(event_header_lines(event, TerminalCapabilities::detect()))
}

pub fn print_event_body(body: &TransformedBody) -> Result<(), PresentationError> {
    print_lines(event_body_lines(body, TerminalCapabilities::detect())?)
}

pub fn print_tail_header(
    endpoint_id: &str,
    capabilities: TerminalCapabilities,
) -> Result<(), PresentationError> {
    let renderer = OutputRenderer::new(capabilities);
    let header = renderer.decorate(
        "RECEIVED                   METHOD PATH            SIZE CONTENT-TYPE     EVENT",
        Tone::Muted,
        true,
    );
    print_lines([
        format!("Watching endpoint: {endpoint_id}"),
        String::new(),
        header,
    ])
}

fn event_list_lines(events: &[EventMetadata], capabilities: TerminalCapabilities) -> Vec<String> {
    if events.is_empty() {
        return vec!["No events received yet.".to_owned()];
    }
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let detail = format!("{} received", events.len());
        let columns = [
            TableColumn::new("ID", 12),
            TableColumn::new("METHOD", 6).tone(Tone::Method),
            TableColumn::new("PATH", 10).flexible(),
            TableColumn::new("CONTENT-TYPE", 10).flexible(),
            TableColumn::new("SIZE", 6),
            TableColumn::new("RECEIVED", 10).flexible(),
        ];
        let rows = events
            .iter()
            .map(|event| {
                vec![
                    event.id.clone(),
                    event.method.clone(),
                    event.request_path.clone(),
                    event_content_type(&event.headers).to_owned(),
                    format_bytes(event.body.size),
                    event.received_at.clone(),
                ]
            })
            .collect::<Vec<_>>();
        let mut lines = vec![renderer.heading("Events", Some(&detail)), String::new()];
        lines.extend(renderer.table(&columns, &rows));
        return lines;
    }
    let mut lines = vec![
        "ID              METHOD  PATH              CONTENT-TYPE       SIZE    RECEIVED".to_owned(),
    ];
    lines.extend(events.iter().map(|event| {
        format!(
            "{}  {}  {}  {}  {}  {}",
            event.id,
            event.method,
            event.request_path,
            event_content_type(&event.headers),
            format_bytes(event.body.size),
            event.received_at
        )
    }));
    lines
}

fn event_summary_lines(event: &EventMetadata, capabilities: TerminalCapabilities) -> Vec<String> {
    let renderer = OutputRenderer::new(capabilities);
    let method = renderer.decorate(&event.method, Tone::Method, true);
    vec![format!(
        "[{}] {} {} {} {} {}",
        event.received_at,
        method,
        event.request_path,
        format_bytes(event.body.size),
        event_content_type(&event.headers),
        event.id
    )]
}

fn event_detail_lines(
    event: &EventDetail,
    body: Option<&TransformedBody>,
    capabilities: TerminalCapabilities,
) -> Result<Vec<String>, PresentationError> {
    let mut event = event.clone();
    event.request.headers = redact_headers_for_display(&event.request.headers);
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.heading("Event", Some(&event.id)),
            String::new(),
            renderer.section("Request"),
        ];
        lines.extend(renderer.details([
            ("Endpoint", event.endpoint_id.clone()),
            (
                "Method",
                renderer.decorate(&event.request.method, Tone::Method, true),
            ),
            ("Path", event.request.request_path.clone()),
            ("Received", event.received_at.clone()),
            (
                "Content-Type",
                event_content_type(&event.request.headers).to_owned(),
            ),
            ("Size", format_bytes(event.request.body.size)),
        ]));
        lines.extend([String::new(), renderer.section("Headers")]);
        lines.extend(
            renderer.details(
                event
                    .request
                    .headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.clone())),
            ),
        );
        lines.extend([String::new(), renderer.section("Body")]);
        if let Some(body) = body {
            lines.extend(body_lines(body)?);
        }
        return Ok(lines);
    }
    let mut lines = vec![
        format!("Event: {}", event.id),
        format!("Endpoint: {}", event.endpoint_id),
        String::new(),
        "Request:".to_owned(),
        format!("  Method:       {}", event.request.method),
        format!("  Path:         {}", event.request.request_path),
        format!("  Received:     {}", event.received_at),
        format!(
            "  Content-Type: {}",
            event_content_type(&event.request.headers)
        ),
        format!("  Size:         {}", format_bytes(event.request.body.size)),
        String::new(),
        "Headers:".to_owned(),
    ];
    lines.extend(
        event
            .request
            .headers
            .iter()
            .map(|(name, value)| format!("  {name}: {value}")),
    );
    lines.extend([String::new(), "Body:".to_owned()]);
    if let Some(body) = body {
        lines.extend(body_lines(body)?);
    }
    Ok(lines)
}

fn event_header_lines(event: &EventDetail, capabilities: TerminalCapabilities) -> Vec<String> {
    let headers = redact_headers_for_display(&event.request.headers);
    let renderer = OutputRenderer::new(capabilities);
    let heading = if renderer.capabilities.interactive {
        renderer.section("Headers")
    } else {
        "Headers:".to_owned()
    };
    let mut lines = vec![String::new(), heading];
    lines.extend(
        headers
            .iter()
            .map(|(name, value)| format!("  {name}: {value}")),
    );
    lines
}

fn event_body_lines(
    body: &TransformedBody,
    capabilities: TerminalCapabilities,
) -> Result<Vec<String>, PresentationError> {
    let renderer = OutputRenderer::new(capabilities);
    let heading = if renderer.capabilities.interactive {
        renderer.section("Body")
    } else {
        "Body:".to_owned()
    };
    let mut lines = vec![String::new(), heading];
    lines.extend(body_lines(body)?);
    Ok(lines)
}

fn body_lines(body: &TransformedBody) -> Result<Vec<String>, PresentationError> {
    match body {
        TransformedBody::Metadata(metadata) => Ok(vec![format!(
            "{} ({})",
            metadata.content_type,
            format_bytes(metadata.size)
        )]),
        TransformedBody::Text(text) => Ok(text.split('\n').map(str::to_owned).collect()),
        TransformedBody::Json(value) => serde_json::to_string_pretty(value)
            .map(|value| value.lines().map(str::to_owned).collect())
            .map_err(PresentationError::from),
    }
}

fn event_content_type(headers: &HeaderMap) -> &str {
    headers.get("content-type").map_or("-", String::as_str)
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

#[cfg(test)]
mod tests {
    use barestash_protocol::{EventBodyMetadata, EventDetailRequest, QueryParameters};

    use super::*;

    fn capabilities(interactive: bool) -> TerminalCapabilities {
        TerminalCapabilities {
            interactive,
            color: false,
            unicode: false,
            width: 120,
            height: 24,
        }
    }

    fn metadata() -> EventMetadata {
        EventMetadata {
            id: "evt_test".to_owned(),
            endpoint_id: "ep_test".to_owned(),
            received_at: "2026-07-05T12:04:32.000Z".to_owned(),
            method: "POST".to_owned(),
            request_path: "/webhook".to_owned(),
            query: QueryParameters::new(),
            headers: HeaderMap::from([("content-type".to_owned(), "application/json".to_owned())]),
            body: EventBodyMetadata {
                size: 2,
                sha256: "hash".to_owned(),
                available: true,
                url: None,
            },
        }
    }

    fn detail() -> EventDetail {
        EventDetail {
            id: "evt_test".to_owned(),
            endpoint_id: "ep_test".to_owned(),
            received_at: "2026-07-05T12:04:32.000Z".to_owned(),
            request: EventDetailRequest {
                method: "POST".to_owned(),
                ingest_path: "/ep_test".to_owned(),
                request_path: "/webhook".to_owned(),
                query: QueryParameters::new(),
                headers: HeaderMap::from([
                    ("content-type".to_owned(), "application/json".to_owned()),
                    ("authorization".to_owned(), "Bearer secret".to_owned()),
                ]),
                body: EventBodyMetadata {
                    size: 2,
                    sha256: "hash".to_owned(),
                    available: true,
                    url: None,
                },
            },
        }
    }

    #[test]
    fn non_interactive_list_preserves_script_friendly_columns() {
        let lines = event_list_lines(&[metadata()], capabilities(false));

        assert_eq!(
            lines,
            [
                "ID              METHOD  PATH              CONTENT-TYPE       SIZE    RECEIVED",
                "evt_test  POST  /webhook  application/json  2 B  2026-07-05T12:04:32.000Z",
            ]
        );
    }

    #[test]
    fn event_detail_redacts_sensitive_headers() {
        let lines = event_detail_lines(
            &detail(),
            Some(&TransformedBody::Json(serde_json::json!({}))),
            capabilities(false),
        )
        .unwrap_or_else(|error| panic!("render succeeds: {error}"));
        let output = lines.join("\n");

        assert!(output.contains("authorization: [REDACTED]"));
        assert!(!output.contains("Bearer secret"));
        assert!(output.contains("Body:\n{}"));
    }

    #[test]
    fn body_metadata_uses_human_readable_size() {
        let lines = event_body_lines(
            &TransformedBody::Metadata(crate::events::BodyMetadata {
                content_type: "application/octet-stream".to_owned(),
                size: 2048,
            }),
            capabilities(false),
        )
        .unwrap_or_else(|error| panic!("render succeeds: {error}"));

        assert_eq!(lines, ["", "Body:", "application/octet-stream (2.0 KB)"]);
    }
}
