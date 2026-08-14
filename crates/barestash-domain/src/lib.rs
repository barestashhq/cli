//! Pure domain transformations and compatibility rules.

mod body;
mod config;
mod credential;
mod duration;
mod event;
mod headers;

pub use body::{
    BodyDecodeError, BodyMetadata, TransformedBody, is_json_content_type,
    is_multipart_content_type, is_text_content_type, transform_body, transform_stream_body,
};
pub use config::{
    CliConfig, parse_config, resolve_config_path, select_endpoint_id, selected_endpoint_id,
    serialize_config,
};
pub use credential::{StoredCredential, parse_stored_credential, serialize_stored_credential};
pub use duration::{DurationParseError, parse_poll_interval, parse_token_duration_seconds};
pub use event::{
    TransformedEventStreamPayload, TransformedEventStreamRequest, transform_stream_payload,
};
pub use headers::{REDACTED_HEADER_VALUE, redact_headers_for_display};
