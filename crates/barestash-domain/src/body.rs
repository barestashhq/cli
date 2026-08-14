use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::Serialize;
use serde_json::Value;

use barestash_protocol::EventStreamPayload;

pub fn is_json_content_type(content_type: &str) -> bool {
    let media_type = normalized_media_type(content_type);
    media_type == "application/json" || media_type.ends_with("+json")
}

pub fn is_text_content_type(content_type: &str) -> bool {
    let media_type = normalized_media_type(content_type);
    media_type.starts_with("text/") || media_type == "application/x-www-form-urlencoded"
}

pub fn is_multipart_content_type(content_type: &str) -> bool {
    normalized_media_type(content_type).starts_with("multipart/")
}

fn normalized_media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BodyMetadata {
    pub content_type: String,
    pub size: u64,
}

/// The enum variant is the synthetic marker. It allows presentation code to
/// distinguish generated metadata from a real JSON object with the same keys,
/// while its untagged serialization stays compatible with the TypeScript CLI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TransformedBody {
    Metadata(BodyMetadata),
    Json(Value),
    Text(String),
}

impl TransformedBody {
    pub const fn as_metadata(&self) -> Option<&BodyMetadata> {
        match self {
            Self::Metadata(metadata) => Some(metadata),
            Self::Json(_) | Self::Text(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvalidUtf8Fallback<'a> {
    Metadata,
    Base64(&'a str),
}

fn byte_length(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

fn transform_body_bytes(
    bytes: &[u8],
    content_type: &str,
    invalid_utf8_fallback: InvalidUtf8Fallback<'_>,
) -> TransformedBody {
    let metadata = || {
        TransformedBody::Metadata(BodyMetadata {
            content_type: content_type.to_owned(),
            size: byte_length(bytes),
        })
    };

    if bytes.is_empty() || is_multipart_content_type(content_type) {
        return metadata();
    }

    let is_json = is_json_content_type(content_type);
    let is_text = is_text_content_type(content_type);
    let Ok(text) = std::str::from_utf8(bytes) else {
        return if is_json || is_text {
            match invalid_utf8_fallback {
                InvalidUtf8Fallback::Metadata => metadata(),
                InvalidUtf8Fallback::Base64(data) => TransformedBody::Text(data.to_owned()),
            }
        } else {
            metadata()
        };
    };

    if is_json {
        return serde_json::from_str(text)
            .map(TransformedBody::Json)
            .unwrap_or_else(|_| TransformedBody::Text(text.to_owned()));
    }

    if is_text {
        return TransformedBody::Text(text.to_owned());
    }

    metadata()
}

pub fn transform_body(bytes: &[u8], content_type: &str) -> TransformedBody {
    transform_body_bytes(bytes, content_type, InvalidUtf8Fallback::Metadata)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyDecodeError;

impl fmt::Display for BodyDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Barestash event stream contained an invalid base64 body.")
    }
}

impl std::error::Error for BodyDecodeError {}

pub fn transform_stream_body(
    payload: &EventStreamPayload,
) -> Result<TransformedBody, BodyDecodeError> {
    let bytes = BASE64_STANDARD
        .decode(payload.body.data.as_bytes())
        .map_err(|_| BodyDecodeError)?;
    let content_type = payload
        .request
        .headers
        .get("content-type")
        .map_or("", String::as_str);

    Ok(transform_body_bytes(
        &bytes,
        content_type,
        InvalidUtf8Fallback::Base64(&payload.body.data),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use barestash_protocol::{
        EventStreamBody, EventStreamBodyEncoding, EventStreamRequest, HeaderMap, QueryParameters,
    };

    fn base64(value: &[u8]) -> String {
        BASE64_STANDARD.encode(value)
    }

    fn stream_payload(content_type: &str, data: String) -> EventStreamPayload {
        EventStreamPayload {
            id: "evt_body".to_owned(),
            endpoint_id: "ep_body".to_owned(),
            received_at: "2026-07-12T12:00:00.000Z".to_owned(),
            request: EventStreamRequest {
                method: "POST".to_owned(),
                path: "/webhook".to_owned(),
                query: QueryParameters::new(),
                headers: HeaderMap::from([("content-type".to_owned(), content_type.to_owned())]),
                body_size: 0,
                body_sha256: "test-sha256".to_owned(),
            },
            body: EventStreamBody {
                encoding: EventStreamBodyEncoding::Base64,
                data,
            },
        }
    }

    #[test]
    fn direct_bodies_follow_content_type_and_utf8_rules() {
        assert_eq!(
            transform_body(
                br#"{"accepted":true}"#,
                "application/problem+json; charset=utf-8"
            ),
            TransformedBody::Json(serde_json::json!({"accepted": true}))
        );
        assert_eq!(
            transform_body(br#"{"event":"#, "application/json"),
            TransformedBody::Text("{\"event\":".to_owned())
        );
        assert_eq!(
            transform_body(b"hello=world", "application/x-www-form-urlencoded"),
            TransformedBody::Text("hello=world".to_owned())
        );
        assert_eq!(
            transform_body(&[0xff, 0xfe], "text/plain"),
            TransformedBody::Metadata(BodyMetadata {
                content_type: "text/plain".to_owned(),
                size: 2
            })
        );
    }

    #[test]
    fn empty_multipart_and_binary_bodies_are_metadata() {
        for (bytes, content_type, size) in [
            (&[][..], "text/plain", 0),
            (
                &b"--barestash--"[..],
                "multipart/form-data; boundary=barestash",
                13,
            ),
            (&[0, 1, 2, 255][..], "application/octet-stream", 4),
        ] {
            assert_eq!(
                transform_body(bytes, content_type),
                TransformedBody::Metadata(BodyMetadata {
                    content_type: content_type.to_owned(),
                    size,
                })
            );
        }
    }

    #[test]
    fn stream_body_decodes_json_and_preserves_invalid_text_as_base64() {
        let json = stream_payload("application/json", base64(br#"{"streamed":true}"#));
        assert_eq!(
            transform_stream_body(&json),
            Ok(TransformedBody::Json(serde_json::json!({"streamed": true})))
        );

        let invalid_data = base64(&[0xff, 0xfe]);
        let invalid_text = stream_payload("text/plain", invalid_data.clone());
        assert_eq!(
            transform_stream_body(&invalid_text),
            Ok(TransformedBody::Text(invalid_data))
        );
    }

    #[test]
    fn synthetic_metadata_is_distinct_but_serializes_like_plain_json() {
        let synthetic = TransformedBody::Metadata(BodyMetadata {
            content_type: "application/json".to_owned(),
            size: 123,
        });
        let real = TransformedBody::Json(serde_json::json!({
            "content_type": "application/json",
            "size": 123
        }));

        assert!(synthetic.as_metadata().is_some());
        assert!(real.as_metadata().is_none());
        assert_eq!(
            serde_json::to_value(&synthetic).unwrap_or_default(),
            serde_json::to_value(&real).unwrap_or_default()
        );
    }
}
