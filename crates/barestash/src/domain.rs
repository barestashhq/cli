//! Pure domain transformations and compatibility rules.

use std::collections::BTreeMap;
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use barestash_protocol::{EventId, EventStreamPayload, HeaderMap, QueryParameters};

pub const REDACTED_HEADER_VALUE: &str = "[REDACTED]";

const REMOVED_HEADER_NAMES: [&str; 2] = ["x-barestash-secret", "x-barestash-bootstrap-token"];

const REDACTED_HEADER_NAMES: [&str; 12] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
    "stripe-signature",
    "x-hub-signature",
    "x-hub-signature-256",
    "x-slack-signature",
    "x-shopify-hmac-sha256",
];

/// Lowercases header names, removes Barestash credentials, and redacts other
/// known authentication/signature headers.
pub fn redact_headers_for_display(headers: &HeaderMap) -> HeaderMap {
    let mut display_headers = HeaderMap::new();

    for (raw_name, value) in headers {
        let name = raw_name.to_ascii_lowercase();

        if REMOVED_HEADER_NAMES.contains(&name.as_str()) {
            continue;
        }

        let display_value = if REDACTED_HEADER_NAMES.contains(&name.as_str()) {
            REDACTED_HEADER_VALUE.to_owned()
        } else {
            value.clone()
        };
        display_headers.insert(name, display_value);
    }

    display_headers
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationParseError {
    PollIntervalUnit,
    TokenExpirationUnit,
    TokenExpirationNotPositive,
    TooLarge,
}

impl fmt::Display for DurationParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PollIntervalUnit => {
                formatter.write_str("Poll interval must include a unit: ms, s, or m.")
            }
            Self::TokenExpirationUnit => {
                formatter.write_str("Token expiration must include a unit: d or y.")
            }
            Self::TokenExpirationNotPositive => {
                formatter.write_str("Token expiration must be a positive duration.")
            }
            Self::TooLarge => formatter.write_str("Duration is too large."),
        }
    }
}

impl std::error::Error for DurationParseError {}

fn parse_ascii_digits(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub fn parse_poll_interval(value: &str) -> Result<u64, DurationParseError> {
    let (amount, multiplier) = if let Some(amount) = value.strip_suffix("ms") {
        (amount, 1)
    } else if let Some(amount) = value.strip_suffix('s') {
        (amount, 1_000)
    } else if let Some(amount) = value.strip_suffix('m') {
        (amount, 60_000)
    } else {
        return Err(DurationParseError::PollIntervalUnit);
    };
    let amount = parse_ascii_digits(amount).ok_or(DurationParseError::PollIntervalUnit)?;
    amount
        .checked_mul(multiplier)
        .ok_or(DurationParseError::TooLarge)
}

pub fn parse_token_duration_seconds(value: &str) -> Result<u64, DurationParseError> {
    let (amount, days_multiplier) = if let Some(amount) = value.strip_suffix('d') {
        (amount, 1_u64)
    } else if let Some(amount) = value.strip_suffix('y') {
        (amount, 365_u64)
    } else {
        return Err(DurationParseError::TokenExpirationUnit);
    };
    let amount = parse_ascii_digits(amount).ok_or(DurationParseError::TokenExpirationUnit)?;

    if amount == 0 {
        return Err(DurationParseError::TokenExpirationNotPositive);
    }

    amount
        .checked_mul(days_multiplier)
        .and_then(|days| days.checked_mul(24 * 60 * 60))
        .ok_or(DurationParseError::TooLarge)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_endpoint: Option<String>,
}

pub fn resolve_config_path(
    env: &BTreeMap<String, String>,
    platform_name: &str,
    home_directory: &str,
) -> String {
    if let Some(config_file) = env.get("BARESTASH_CONFIG_FILE") {
        return config_file.clone();
    }

    if let Some(xdg_config_home) = env.get("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return format!("{xdg_config_home}/barestash/config.toml");
    }

    match platform_name {
        "darwin" => format!("{home_directory}/Library/Application Support/barestash/config.toml"),
        "win32" => {
            let app_data = env
                .get("APPDATA")
                .cloned()
                .unwrap_or_else(|| format!("{home_directory}/AppData/Roaming"));
            format!("{app_data}/barestash/config.toml")
        }
        _ => format!("{home_directory}/.config/barestash/config.toml"),
    }
}

pub fn parse_config(text: Option<&str>) -> CliConfig {
    let Some(text) = text.filter(|value| !value.trim().is_empty()) else {
        return CliConfig::default();
    };
    let Ok(toml::Value::Table(table)) = toml::from_str::<toml::Value>(text) else {
        return CliConfig::default();
    };

    CliConfig {
        token: table
            .get("token")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
        default_endpoint: table
            .get("default_endpoint")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
    }
}

pub fn serialize_config(config: &CliConfig) -> String {
    let mut serialized = toml::to_string_pretty(config).unwrap_or_default();
    if !serialized.ends_with('\n') {
        serialized.push('\n');
    }
    serialized
}

pub fn select_endpoint_id(
    endpoint_flag: Option<&str>,
    environment_endpoint: Option<&str>,
    configured_endpoint: Option<&str>,
) -> Option<String> {
    endpoint_flag
        .or(environment_endpoint)
        .or(configured_endpoint)
        .map(str::to_owned)
}

pub fn selected_endpoint_id(
    endpoint_flag: Option<&str>,
    env: &BTreeMap<String, String>,
    config: &CliConfig,
) -> Option<String> {
    select_endpoint_id(
        endpoint_flag,
        env.get("BARESTASH_ENDPOINT").map(String::as_str),
        config.default_endpoint.as_deref(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoredCredential {
    PersonalAccessToken {
        token: String,
    },
    CliSession {
        session_id: String,
        access_token: String,
        refresh_token: String,
        access_token_expires_at: String,
        refresh_token_expires_at: String,
        // Stored sessions accept future server scopes as opaque strings. The
        // reference credential parser required strings, but intentionally did
        // not reject scopes introduced after this CLI version.
        scopes: Vec<String>,
    },
}

pub fn parse_stored_credential(value: Option<&str>) -> Option<StoredCredential> {
    value.and_then(|value| serde_json::from_str(value).ok())
}

pub fn serialize_stored_credential(
    credential: &StoredCredential,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(credential)
}

#[cfg(test)]
mod tests {
    use super::*;
    use barestash_protocol::{
        AuthorizationScope, EventStreamBody, EventStreamBodyEncoding, EventStreamRequest,
        QueryParameterValue,
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
    fn redacts_sensitive_headers_and_removes_barestash_credentials() {
        let headers = HeaderMap::from([
            ("Authorization".to_owned(), "Bearer raw-token".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Stripe-Signature".to_owned(), "t=raw,v1=raw".to_owned()),
            (
                "X-Barestash-Bootstrap-Token".to_owned(),
                "bootstrap-secret".to_owned(),
            ),
            (
                "X-Barestash-Secret".to_owned(),
                "endpoint-secret".to_owned(),
            ),
            ("X-Custom".to_owned(), "safe for display".to_owned()),
        ]);

        assert_eq!(
            redact_headers_for_display(&headers),
            HeaderMap::from([
                ("authorization".to_owned(), REDACTED_HEADER_VALUE.to_owned()),
                ("content-type".to_owned(), "application/json".to_owned()),
                (
                    "stripe-signature".to_owned(),
                    REDACTED_HEADER_VALUE.to_owned()
                ),
                ("x-custom".to_owned(), "safe for display".to_owned()),
            ])
        );
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

    #[test]
    fn transformed_stream_payload_redacts_headers_and_preserves_metadata() {
        let mut payload = stream_payload("application/json", base64(br#"{"ok":true}"#));
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

    #[test]
    fn parses_poll_and_token_durations() {
        assert_eq!(parse_poll_interval("500ms"), Ok(500));
        assert_eq!(parse_poll_interval("2s"), Ok(2_000));
        assert_eq!(parse_poll_interval("1m"), Ok(60_000));
        assert_eq!(parse_poll_interval("0ms"), Ok(0));
        assert_eq!(
            parse_poll_interval("2"),
            Err(DurationParseError::PollIntervalUnit)
        );

        assert_eq!(parse_token_duration_seconds("30d"), Ok(2_592_000));
        assert_eq!(parse_token_duration_seconds("1y"), Ok(31_536_000));
        assert_eq!(
            parse_token_duration_seconds("0d"),
            Err(DurationParseError::TokenExpirationNotPositive)
        );
        assert_eq!(
            parse_token_duration_seconds("90days"),
            Err(DurationParseError::TokenExpirationUnit)
        );
    }

    #[test]
    fn resolves_toml_config_paths_with_environment_precedence() {
        let env = BTreeMap::from([
            (
                "BARESTASH_CONFIG_FILE".to_owned(),
                "/override/barestash.conf".to_owned(),
            ),
            ("XDG_CONFIG_HOME".to_owned(), "/xdg".to_owned()),
            ("APPDATA".to_owned(), "C:/AppData".to_owned()),
        ]);
        assert_eq!(
            resolve_config_path(&env, "win32", "/home/tester"),
            "/override/barestash.conf"
        );
        assert_eq!(
            resolve_config_path(
                &BTreeMap::from([("XDG_CONFIG_HOME".to_owned(), "/xdg".to_owned())]),
                "darwin",
                "/Users/tester"
            ),
            "/xdg/barestash/config.toml"
        );
        assert_eq!(
            resolve_config_path(&BTreeMap::new(), "darwin", "/Users/tester"),
            "/Users/tester/Library/Application Support/barestash/config.toml"
        );
        assert_eq!(
            resolve_config_path(&BTreeMap::new(), "linux", "/home/tester"),
            "/home/tester/.config/barestash/config.toml"
        );
        assert_eq!(
            resolve_config_path(&BTreeMap::new(), "win32", "C:/Users/tester"),
            "C:/Users/tester/AppData/Roaming/barestash/config.toml"
        );
    }

    #[test]
    fn parses_and_serializes_config_without_exposing_invalid_values() {
        for invalid in [None, Some(""), Some("{"), Some("null"), Some("\"text\"")] {
            assert_eq!(parse_config(invalid), CliConfig::default());
        }

        let config = CliConfig {
            token: Some("test-token".to_owned()),
            default_endpoint: Some("ep_test".to_owned()),
        };
        let serialized = serialize_config(&config);
        assert!(serialized.ends_with('\n'));
        assert!(serialized.contains("token = \"test-token\""));
        assert!(serialized.contains("default_endpoint = \"ep_test\""));
        assert!(!serialized.trim_start().starts_with('{'));
        assert_eq!(parse_config(Some(&serialized)), config);
    }

    #[test]
    fn endpoint_selection_prefers_flag_then_environment_then_config() {
        assert_eq!(
            select_endpoint_id(Some("ep_flag"), Some("ep_env"), Some("ep_config")),
            Some("ep_flag".to_owned())
        );
        assert_eq!(
            select_endpoint_id(None, Some("ep_env"), Some("ep_config")),
            Some("ep_env".to_owned())
        );
        assert_eq!(
            select_endpoint_id(None, None, Some("ep_config")),
            Some("ep_config".to_owned())
        );
        assert_eq!(select_endpoint_id(None, None, None), None);
        assert_eq!(
            select_endpoint_id(Some(""), Some("ep_env"), Some("ep_config")),
            Some(String::new())
        );
    }

    #[test]
    fn stored_credentials_round_trip_and_invalid_storage_is_empty() {
        let credential = StoredCredential::CliSession {
            session_id: "cls_test".to_owned(),
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            access_token_expires_at: "2026-07-12T13:00:00.000Z".to_owned(),
            refresh_token_expires_at: "2026-10-12T13:00:00.000Z".to_owned(),
            scopes: vec![AuthorizationScope::EventsRead.to_string()],
        };
        let serialized = serialize_stored_credential(&credential)
            .unwrap_or_else(|error| panic!("credential should serialize: {error}"));

        assert_eq!(parse_stored_credential(Some(&serialized)), Some(credential));
        assert_eq!(parse_stored_credential(Some("not-json")), None);
        assert_eq!(parse_stored_credential(None), None);
    }
}
