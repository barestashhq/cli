use crate::protocol::{find_sse_message_separator, parse_sse_message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub id: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SseError {
    #[error("Event stream closed with an incomplete SSE message.")]
    IncompleteMessage { last_event_id: Option<String> },
}

impl SseError {
    #[must_use]
    pub fn last_event_id(&self) -> Option<&str> {
        match self {
            Self::IncompleteMessage { last_event_id } => last_event_id.as_deref(),
        }
    }
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    utf8: Utf8Accumulator,
    text: String,
    last_event_id: Option<String>,
}

impl SseDecoder {
    #[must_use]
    pub fn new(initial_last_event_id: Option<String>) -> Self {
        Self {
            last_event_id: initial_last_event_id,
            ..Self::default()
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.text.push_str(&self.utf8.push(bytes));
        self.take_complete_events()
    }

    pub fn finish(mut self) -> Result<(Vec<SseEvent>, Option<String>), SseError> {
        let utf8 = std::mem::take(&mut self.utf8);
        self.text.push_str(&utf8.finish());
        let events = self.take_complete_events();
        if self.text.trim().is_empty() {
            Ok((events, self.last_event_id))
        } else {
            Err(SseError::IncompleteMessage {
                last_event_id: self.last_event_id,
            })
        }
    }

    #[must_use]
    pub fn last_event_id(&self) -> Option<&str> {
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
    use super::*;

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
}
