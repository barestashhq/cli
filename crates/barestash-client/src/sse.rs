use std::collections::VecDeque;

use reqwest::Response;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedSseMessage {
    id: Option<String>,
    data: Option<String>,
}

fn parse_sse_message(message: &str) -> ParsedSseMessage {
    let mut id = None;
    let mut data_lines = Vec::new();

    for raw_line in message.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        if let Some(value) = line.strip_prefix("id:") {
            id = Some(value.trim_start().to_owned());
        }
        if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start());
        }
    }

    ParsedSseMessage {
        id,
        data: (!data_lines.is_empty()).then(|| data_lines.join("\n")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SseMessageSeparator {
    index: usize,
    length: usize,
}

fn find_sse_message_separator(buffer: &str) -> Option<SseMessageSeparator> {
    const SEPARATORS: [&[u8]; 4] = [b"\r\n\r\n", b"\r\n\n", b"\n\r\n", b"\n\n"];
    let bytes = buffer.as_bytes();

    for index in 0..bytes.len() {
        for separator in SEPARATORS {
            if bytes[index..].starts_with(separator) {
                return Some(SseMessageSeparator {
                    index,
                    length: separator.len(),
                });
            }
        }
    }

    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub id: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum SseDecodeError {
    #[error("Event stream closed with an incomplete SSE message.")]
    IncompleteMessage { last_event_id: Option<String> },
}

impl SseDecodeError {
    #[cfg(test)]
    #[must_use]
    fn last_event_id(&self) -> Option<&str> {
        match self {
            Self::IncompleteMessage { last_event_id } => last_event_id.as_deref(),
        }
    }
}

#[derive(Debug, Default)]
struct SseDecoder {
    utf8: Utf8Accumulator,
    text: String,
    last_event_id: Option<String>,
}

impl SseDecoder {
    #[must_use]
    fn new(initial_last_event_id: Option<String>) -> Self {
        Self {
            last_event_id: initial_last_event_id,
            ..Self::default()
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.text.push_str(&self.utf8.push(bytes));
        self.take_complete_events()
    }

    fn finish(mut self) -> Result<(Vec<SseEvent>, Option<String>), SseDecodeError> {
        let utf8 = std::mem::take(&mut self.utf8);
        self.text.push_str(&utf8.finish());
        let events = self.take_complete_events();
        if self.text.trim().is_empty() {
            Ok((events, self.last_event_id))
        } else {
            Err(SseDecodeError::IncompleteMessage {
                last_event_id: self.last_event_id,
            })
        }
    }

    #[must_use]
    #[cfg(test)]
    fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    fn take_complete_events(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        while let Some(separator) = find_sse_message_separator(&self.text) {
            let message = self.text[..separator.index].to_owned();
            self.text.drain(..separator.index + separator.length);
            let parsed = parse_sse_message(&message);
            if let Some(id) = parsed.id.as_ref().filter(|id| !id.is_empty()) {
                self.last_event_id = Some(id.clone());
            }
            events.push(SseEvent {
                id: parsed.id,
                data: parsed.data,
            });
        }
        events
    }
}

/// Failure encountered while reading or decoding an SSE response body.
#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct SseStreamError {
    #[source]
    kind: SseStreamErrorKind,
    last_event_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum SseStreamErrorKind {
    #[error("{0}")]
    Read(#[source] reqwest::Error),
    #[error("Event stream closed with an incomplete SSE message.")]
    IncompleteMessage,
}

impl SseStreamError {
    #[must_use]
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }
}

/// Incrementally reads and decodes the SSE messages in an HTTP response.
pub struct SseEventStream {
    response: Response,
    decoder: SseDecoder,
    pending: VecDeque<SseEvent>,
    last_event_id: Option<String>,
    finished: bool,
}

impl SseEventStream {
    #[must_use]
    pub fn new(response: Response, initial_last_event_id: Option<String>) -> Self {
        Self {
            response,
            decoder: SseDecoder::new(initial_last_event_id.clone()),
            pending: VecDeque::new(),
            last_event_id: initial_last_event_id,
            finished: false,
        }
    }

    /// Returns the next complete SSE message, or `None` after a clean EOF.
    pub async fn next_event(&mut self) -> Result<Option<SseEvent>, SseStreamError> {
        loop {
            if let Some(event) = self.take_pending_event() {
                return Ok(Some(event));
            }
            if self.finished {
                return Ok(None);
            }

            match self.response.chunk().await {
                Ok(Some(chunk)) => self.pending.extend(self.decoder.push(&chunk)),
                Ok(None) => {
                    self.finished = true;
                    let decoder = std::mem::take(&mut self.decoder);
                    match decoder.finish() {
                        Ok((events, _)) => self.pending.extend(events),
                        Err(SseDecodeError::IncompleteMessage { .. }) => {
                            return Err(SseStreamError {
                                kind: SseStreamErrorKind::IncompleteMessage,
                                last_event_id: self.last_event_id.clone(),
                            });
                        }
                    }
                }
                Err(source) => {
                    self.finished = true;
                    return Err(SseStreamError {
                        kind: SseStreamErrorKind::Read(source),
                        last_event_id: self.last_event_id.clone(),
                    });
                }
            }
        }
    }

    #[must_use]
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    fn take_pending_event(&mut self) -> Option<SseEvent> {
        let event = self.pending.pop_front()?;
        if let Some(id) = event.id.as_ref().filter(|id| !id.is_empty()) {
            self.last_event_id = Some(id.clone());
        }
        Some(event)
    }
}

#[derive(Debug, Default)]
struct Utf8Accumulator {
    pending: Vec<u8>,
}

impl Utf8Accumulator {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(value) => {
                    output.push_str(value);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        // The prefix is known valid by `Utf8Error`.
                        if let Ok(prefix) = std::str::from_utf8(&self.pending[..valid]) {
                            output.push_str(prefix);
                        }
                        self.pending.drain(..valid);
                    }
                    match error.error_len() {
                        Some(length) => {
                            output.push('\u{fffd}');
                            self.pending.drain(..length.min(self.pending.len()));
                        }
                        None => break,
                    }
                }
            }
        }
        output
    }

    fn finish(mut self) -> String {
        let value = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        value
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    async fn response_from_raw_http(response: Vec<u8>) -> Response {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind raw HTTP server");
        let address = listener.local_addr().expect("raw HTTP server address");
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().expect("accept HTTP request");
            let mut request = [0_u8; 2048];
            let _ = connection.read(&mut request).expect("read HTTP request");
            connection
                .write_all(&response)
                .expect("write raw HTTP response");
        });

        let response = reqwest::get(format!("http://{address}"))
            .await
            .expect("receive HTTP response headers");
        server.join().expect("raw HTTP server finishes");
        response
    }

    fn chunked_response(chunks: &[&str], clean_eof: bool) -> Vec<u8> {
        let mut response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        for chunk in chunks {
            response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            response.extend_from_slice(chunk.as_bytes());
            response.extend_from_slice(b"\r\n");
        }
        if clean_eof {
            response.extend_from_slice(b"0\r\n\r\n");
        }
        response
    }

    #[test]
    fn parses_multiple_and_chunked_events() {
        let mut decoder = SseDecoder::new(None);
        assert!(decoder.push(b"id: one\nda").is_empty());
        let events = decoder.push(b"ta: {\"n\":1}\n\nid: two\ndata: x\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id.as_deref(), Some("one"));
        assert_eq!(events[1].data.as_deref(), Some("x"));
        assert_eq!(decoder.last_event_id(), Some("two"));
    }

    #[test]
    fn safely_decodes_utf8_split_across_chunks() {
        let text = "id: one\ndata: {\"value\":\"受信\"}\n\n";
        let bytes = text.as_bytes();
        let split = text.find('受').expect("multibyte character") + 1;
        let mut decoder = SseDecoder::new(None);
        assert!(decoder.push(&bytes[..split]).is_empty());
        let events = decoder.push(&bytes[split..]);
        assert_eq!(events[0].data.as_deref(), Some("{\"value\":\"受信\"}"));
    }

    #[test]
    fn incomplete_event_keeps_only_previous_complete_id() {
        let mut decoder = SseDecoder::new(None);
        let events = decoder.push(b"id: complete\ndata: {}\n\nid: incomplete\ndata: {");
        assert_eq!(events.len(), 1);
        let error = decoder.finish().expect_err("incomplete stream");
        assert_eq!(error.last_event_id(), Some("complete"));
    }

    #[test]
    fn accepts_crlf_boundaries() {
        let mut decoder = SseDecoder::new(None);
        let events = decoder.push(b"id: one\r\ndata: {}\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("one"));
    }

    #[test]
    fn parses_lf_crlf_and_multiline_sse_messages() {
        assert_eq!(
            parse_sse_message("id: evt_1\ndata: {\"a\":1}\ndata: tail"),
            ParsedSseMessage {
                id: Some("evt_1".to_owned()),
                data: Some("{\"a\":1}\ntail".to_owned()),
            }
        );
        assert_eq!(
            parse_sse_message("id: evt_2\r\ndata: {}\r\n"),
            ParsedSseMessage {
                id: Some("evt_2".to_owned()),
                data: Some("{}".to_owned()),
            }
        );
        assert_eq!(
            find_sse_message_separator("id: 1\ndata: {}\n\nrest"),
            Some(SseMessageSeparator {
                index: 14,
                length: 2,
            })
        );
        assert_eq!(
            find_sse_message_separator("id: 1\r\ndata: {}\r\n\r\nrest"),
            Some(SseMessageSeparator {
                index: 15,
                length: 4,
            })
        );
        assert_eq!(find_sse_message_separator("id: 1\ndata: {}\n"), None);
    }

    #[tokio::test]
    async fn event_stream_reads_chunks_and_finishes_at_clean_eof() {
        let response = response_from_raw_http(chunked_response(
            &["id: one\nda", "ta: first\n\nid: two\n", "data: second\n\n"],
            true,
        ))
        .await;
        let mut stream = SseEventStream::new(response, None);

        let first = stream
            .next_event()
            .await
            .expect("first event read")
            .expect("first event");
        assert_eq!(first.id.as_deref(), Some("one"));
        assert_eq!(first.data.as_deref(), Some("first"));

        let second = stream
            .next_event()
            .await
            .expect("second event read")
            .expect("second event");
        assert_eq!(second.id.as_deref(), Some("two"));
        assert_eq!(second.data.as_deref(), Some("second"));
        assert!(stream.next_event().await.expect("clean EOF").is_none());
        assert_eq!(stream.last_event_id(), Some("two"));
    }

    #[tokio::test]
    async fn event_stream_reports_incomplete_eof_with_last_complete_id() {
        let response = response_from_raw_http(chunked_response(
            &["id: complete\ndata: {}\n\nid: incomplete\ndata: {"],
            true,
        ))
        .await;
        let mut stream = SseEventStream::new(response, Some("previous".to_owned()));

        let event = stream
            .next_event()
            .await
            .expect("complete event read")
            .expect("complete event");
        assert_eq!(event.id.as_deref(), Some("complete"));
        let error = stream.next_event().await.expect_err("incomplete EOF");
        assert!(matches!(error.kind, SseStreamErrorKind::IncompleteMessage));
        assert_eq!(error.last_event_id(), Some("complete"));
    }

    #[tokio::test]
    async fn event_stream_reports_body_read_errors_with_last_complete_id() {
        let complete = "id: complete\ndata: {}\n\n";
        let mut response = chunked_response(&[complete], false);
        response.extend_from_slice(b"5\r\nab");
        let response = response_from_raw_http(response).await;
        let mut stream = SseEventStream::new(response, None);

        let event = stream
            .next_event()
            .await
            .expect("complete event read")
            .expect("complete event");
        assert_eq!(event.id.as_deref(), Some("complete"));
        let error = stream.next_event().await.expect_err("truncated HTTP body");
        assert!(matches!(error.kind, SseStreamErrorKind::Read(_)));
        assert_eq!(error.last_event_id(), Some("complete"));
    }
}
