use serde::Serialize;

use barestash_protocol::{EventId, EventStreamPayload, HeaderMap, QueryParameters};

use super::body::{BodyDecodeError, TransformedBody, transform_stream_body};
use super::headers::redact_headers_for_display;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransformedEventStreamPayload {
    pub id: EventId,
    pub endpoint_id: String,
    pub received_at: String,
    pub request: TransformedEventStreamRequest,
    pub body: TransformedBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransformedEventStreamRequest {
    pub method: String,
    pub path: String,
    pub query: QueryParameters,
    pub headers: HeaderMap,
    pub body_size: u64,
    pub body_sha256: String,
}

pub fn transform_stream_payload(
    payload: EventStreamPayload,
) -> Result<TransformedEventStreamPayload, BodyDecodeError> {
    let body = transform_stream_body(&payload)?;

    Ok(TransformedEventStreamPayload {
        id: payload.id,
        endpoint_id: payload.endpoint_id,
        received_at: payload.received_at,
        request: TransformedEventStreamRequest {
            method: payload.request.method,
            path: payload.request.path,
            query: payload.request.query,
            headers: redact_headers_for_display(&payload.request.headers),
            body_size: payload.request.body_size,
            body_sha256: payload.request.body_sha256,
        },
        body,
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

    use barestash_protocol::{
        EventStreamBody, EventStreamBodyEncoding, EventStreamRequest, QueryParameterValue,
    };

    use super::super::headers::REDACTED_HEADER_VALUE;
    use super::*;

    #[test]
    fn transformed_stream_payload_redacts_headers_and_preserves_metadata() {
        let mut payload = EventStreamPayload {
            id: "evt_body".to_owned(),
            endpoint_id: "ep_body".to_owned(),
            received_at: "2026-07-12T12:00:00.000Z".to_owned(),
            request: EventStreamRequest {
                method: "POST".to_owned(),
                path: "/webhook".to_owned(),
                query: QueryParameters::new(),
                headers: HeaderMap::from([(
                    "content-type".to_owned(),
                    "application/json".to_owned(),
                )]),
                body_size: 0,
                body_sha256: "test-sha256".to_owned(),
            },
            body: EventStreamBody {
                encoding: EventStreamBodyEncoding::Base64,
                data: BASE64_STANDARD.encode(br#"{"ok":true}"#),
            },
        };
        payload.request.query.insert(
            "tag".to_owned(),
            QueryParameterValue::Single("one".to_owned()),
        );
        payload.request.headers.extend([
            ("authorization".to_owned(), "Bearer test-secret".to_owned()),
            (
                "x-barestash-secret".to_owned(),
                "endpoint-secret".to_owned(),
            ),
        ]);
        payload.request.body_size = 11;

        let transformed = transform_stream_payload(payload)
            .unwrap_or_else(|error| panic!("valid base64 body: {error}"));
        assert_eq!(
            transformed.request.headers.get("authorization"),
            Some(&REDACTED_HEADER_VALUE.to_owned())
        );
        assert!(
            !transformed
                .request
                .headers
                .contains_key("x-barestash-secret")
        );
        assert_eq!(transformed.request.body_size, 11);
        assert_eq!(transformed.request.body_sha256, "test-sha256");
    }
}
