//! Wire-level contracts shared by the CLI and the Barestash HTTP API.
//!
//! Field names and enum spellings in this module are part of the compatibility
//! contract. Domain-level presentation transformations belong in `domain`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub type AccessTokenId = String;
pub type AccountId = String;
pub type CliSessionId = String;
pub type EndpointId = String;
pub type EventId = String;
pub type TokenId = String;
pub type SecretId = String;

pub const ACCESS_TOKEN_ID_PREFIX: &str = "atk_";
pub const ACCOUNT_ID_PREFIX: &str = "acc_";
pub const CLI_SESSION_ID_PREFIX: &str = "cls_";
pub const ENDPOINT_ID_PREFIX: &str = "ep_";
pub const EVENT_ID_PREFIX: &str = "evt_";
pub const TOKEN_ID_PREFIX: &str = "tok_";
pub const SECRET_ID_PREFIX: &str = "sec_";
pub const TOKEN_ID_SUFFIX_LENGTH: usize = 24;
pub const TOKEN_ID_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    AccessToken,
    Account,
    CliSession,
    Endpoint,
    Event,
    Token,
    Secret,
}

impl IdKind {
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::AccessToken => ACCESS_TOKEN_ID_PREFIX,
            Self::Account => ACCOUNT_ID_PREFIX,
            Self::CliSession => CLI_SESSION_ID_PREFIX,
            Self::Endpoint => ENDPOINT_ID_PREFIX,
            Self::Event => EVENT_ID_PREFIX,
            Self::Token => TOKEN_ID_PREFIX,
            Self::Secret => SECRET_ID_PREFIX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdParseError {
    kind: IdKind,
}

impl IdParseError {
    pub const fn kind(&self) -> IdKind {
        self.kind
    }
}

impl fmt::Display for IdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == IdKind::Token {
            write!(
                formatter,
                "Token ID must be tok_ followed by {TOKEN_ID_SUFFIX_LENGTH} ASCII alphanumeric characters."
            )
        } else {
            write!(
                formatter,
                "ID must start with {} and have a non-empty suffix.",
                self.kind.prefix()
            )
        }
    }
}

impl std::error::Error for IdParseError {}

pub fn validate_id(value: &str, kind: IdKind) -> Result<(), IdParseError> {
    let Some(suffix) = value.strip_prefix(kind.prefix()) else {
        return Err(IdParseError { kind });
    };

    let valid = if kind == IdKind::Token {
        suffix.len() == TOKEN_ID_SUFFIX_LENGTH
            && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
    } else {
        !suffix.is_empty()
    };

    if valid {
        Ok(())
    } else {
        Err(IdParseError { kind })
    }
}

pub fn is_token_id(value: &str) -> bool {
    validate_id(value, IdKind::Token).is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomByteLengthError {
    required: usize,
}

impl fmt::Display for RandomByteLengthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Token generation requires at least {} bytes of randomness.",
            self.required
        )
    }
}

impl std::error::Error for RandomByteLengthError {}

fn alphanumeric_from_random_bytes(
    random_bytes: &[u8],
    length: usize,
) -> Result<String, RandomByteLengthError> {
    if random_bytes.len() < length {
        return Err(RandomByteLengthError { required: length });
    }

    Ok(random_bytes
        .iter()
        .take(length)
        .map(|byte| TOKEN_ID_ALPHABET[usize::from(*byte) % TOKEN_ID_ALPHABET.len()] as char)
        .collect())
}

pub fn generate_token_id_from_random_bytes(
    random_bytes: &[u8],
) -> Result<TokenId, RandomByteLengthError> {
    alphanumeric_from_random_bytes(random_bytes, TOKEN_ID_SUFFIX_LENGTH)
        .map(|suffix| format!("{TOKEN_ID_PREFIX}{suffix}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BearerTokenType {
    Access,
    Refresh,
    Pat,
}

impl BearerTokenType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Refresh => "refresh",
            Self::Pat => "pat",
        }
    }
}

pub const BEARER_TOKEN_SECRET_LENGTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBearerToken {
    pub token_type: BearerTokenType,
    pub token_id_suffix: String,
    pub secret: String,
}

pub fn parse_bearer_token_string(value: &str) -> Option<ParsedBearerToken> {
    let mut parts = value.split('_');
    let prefix = parts.next()?;
    let token_type = match parts.next()? {
        "access" => BearerTokenType::Access,
        "refresh" => BearerTokenType::Refresh,
        "pat" => BearerTokenType::Pat,
        _ => return None,
    };
    let token_id_suffix = parts.next()?;
    let secret = parts.next()?;

    if parts.next().is_some()
        || prefix != "bst"
        || token_id_suffix.len() != TOKEN_ID_SUFFIX_LENGTH
        || secret.len() != BEARER_TOKEN_SECRET_LENGTH
        || !token_id_suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
        || !secret.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }

    Some(ParsedBearerToken {
        token_type,
        token_id_suffix: token_id_suffix.to_owned(),
        secret: secret.to_owned(),
    })
}

pub fn format_bearer_token_string(parts: &ParsedBearerToken) -> String {
    format!(
        "bst_{}_{}_{}",
        parts.token_type.as_str(),
        parts.token_id_suffix,
        parts.secret
    )
}

pub fn token_id_from_bearer_token_string(value: &str) -> Option<TokenId> {
    let parsed = parse_bearer_token_string(value)?;
    let token_id = format!("{TOKEN_ID_PREFIX}{}", parsed.token_id_suffix);
    is_token_id(&token_id).then_some(token_id)
}

pub fn generate_bearer_token_secret_from_random_bytes(
    random_bytes: &[u8],
) -> Result<String, RandomByteLengthError> {
    alphanumeric_from_random_bytes(random_bytes, BEARER_TOKEN_SECRET_LENGTH)
}

pub fn format_pat_bearer_token_string(token_id: &str, secret: &str) -> Option<String> {
    if !is_token_id(token_id)
        || secret.len() != BEARER_TOKEN_SECRET_LENGTH
        || !secret.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }

    Some(format_bearer_token_string(&ParsedBearerToken {
        token_type: BearerTokenType::Pat,
        token_id_suffix: token_id[TOKEN_ID_PREFIX.len()..].to_owned(),
        secret: secret.to_owned(),
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthorizationScope {
    #[serde(rename = "endpoints:read")]
    EndpointsRead,
    #[serde(rename = "endpoints:write")]
    EndpointsWrite,
    #[serde(rename = "events:read")]
    EventsRead,
    #[serde(rename = "tokens:read")]
    TokensRead,
    #[serde(rename = "tokens:write")]
    TokensWrite,
    #[serde(rename = "mcp:use")]
    McpUse,
}

impl AuthorizationScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EndpointsRead => "endpoints:read",
            Self::EndpointsWrite => "endpoints:write",
            Self::EventsRead => "events:read",
            Self::TokensRead => "tokens:read",
            Self::TokensWrite => "tokens:write",
            Self::McpUse => "mcp:use",
        }
    }
}

impl fmt::Display for AuthorizationScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationScopeParseError(String);

impl fmt::Display for AuthorizationScopeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Unsupported authorization scope: {}", self.0)
    }
}

impl std::error::Error for AuthorizationScopeParseError {}

impl std::str::FromStr for AuthorizationScope {
    type Err = AuthorizationScopeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "endpoints:read" => Ok(Self::EndpointsRead),
            "endpoints:write" => Ok(Self::EndpointsWrite),
            "events:read" => Ok(Self::EventsRead),
            "tokens:read" => Ok(Self::TokensRead),
            "tokens:write" => Ok(Self::TokensWrite),
            "mcp:use" => Ok(Self::McpUse),
            _ => Err(AuthorizationScopeParseError(value.to_owned())),
        }
    }
}

pub const AUTHORIZATION_SCOPES: [AuthorizationScope; 6] = [
    AuthorizationScope::EndpointsRead,
    AuthorizationScope::EndpointsWrite,
    AuthorizationScope::EventsRead,
    AuthorizationScope::TokensRead,
    AuthorizationScope::TokensWrite,
    AuthorizationScope::McpUse,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAuthorizationCreateRequest {
    pub client_name: String,
    pub client_version: String,
    pub device_name: String,
    pub requested_scopes: Vec<AuthorizationScope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAuthorizationCreateResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTokenRequest {
    pub device_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: BearerTokenScheme,
    pub expires_in: u64,
    pub refresh_token_expires_in: u64,
    pub scopes: Vec<AuthorizationScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BearerTokenScheme {
    Bearer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshTokenRequest {
    pub grant_type: RefreshGrantType,
    pub refresh_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshGrantType {
    RefreshToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: BearerTokenScheme,
    pub expires_in: u64,
    pub refresh_token_expires_in: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountResponse {
    pub account: AccountMetadata,
    pub credential: AccountCredential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountMetadata {
    pub id: AccountId,
    pub primary_email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AccountCredential {
    #[serde(rename = "cli_access_token")]
    CliAccessToken {
        id: AccessTokenId,
        session_id: CliSessionId,
        scopes: Vec<AuthorizationScope>,
        expires_at: String,
    },
    #[serde(rename = "personal_access_token")]
    PersonalAccessToken {
        id: TokenId,
        scopes: Vec<AuthorizationScope>,
        expires_at: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointMode {
    Private,
    Temporary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointStatus {
    Active,
    Disabled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointCreateRequest {
    pub mode: EndpointMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointMetadata {
    pub id: EndpointId,
    pub name: Option<String>,
    pub mode: EndpointMode,
    pub status: EndpointStatus,
    pub public_read: bool,
    pub event_count: u64,
    pub event_limit: Option<u64>,
    pub expires_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub ingest_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointResponse {
    pub endpoint: EndpointMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointListResponse {
    pub endpoints: Vec<EndpointMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointDeleteResponse {
    pub endpoint: EndpointMetadata,
    pub deleted_events: u64,
    pub deleted_body_objects: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointSecretStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSecretMetadata {
    pub id: SecretId,
    pub endpoint_id: EndpointId,
    pub status: EndpointSecretStatus,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSecretCreateResponse {
    pub endpoint_secret: EndpointSecretMetadata,
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSecretListResponse {
    pub endpoint_secrets: Vec<EndpointSecretMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSecretRevokeResponse {
    pub endpoint_secret: EndpointSecretMetadata,
}

pub type HeaderMap = BTreeMap<String, String>;
pub type QueryParameters = BTreeMap<String, QueryParameterValue>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryParameterValue {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventBodyMetadata {
    pub size: u64,
    pub sha256: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventMetadata {
    pub id: EventId,
    pub endpoint_id: EndpointId,
    pub received_at: String,
    pub method: String,
    pub request_path: String,
    pub query: QueryParameters,
    pub headers: HeaderMap,
    pub body: EventBodyMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDetail {
    pub id: EventId,
    pub endpoint_id: EndpointId,
    pub received_at: String,
    pub request: EventDetailRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDetailRequest {
    pub method: String,
    pub ingest_path: String,
    pub request_path: String,
    pub query: QueryParameters,
    pub headers: HeaderMap,
    pub body: EventBodyMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventListResponse {
    pub events: Vec<EventMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventStreamPayload {
    pub id: EventId,
    pub endpoint_id: EndpointId,
    pub received_at: String,
    pub request: EventStreamRequest,
    pub body: EventStreamBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventStreamRequest {
    pub method: String,
    pub path: String,
    pub query: QueryParameters,
    pub headers: HeaderMap,
    pub body_size: u64,
    pub body_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventStreamBody {
    pub encoding: EventStreamBodyEncoding,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStreamBodyEncoding {
    Base64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalAccessTokenStatus {
    Active,
    Revoked,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccessTokenMetadata {
    pub id: TokenId,
    pub name: Option<String>,
    pub status: PersonalAccessTokenStatus,
    pub scopes: Vec<AuthorizationScope>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccessTokenCreateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub scopes: Vec<AuthorizationScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<Option<u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccessTokenCreateResponse {
    pub id: TokenId,
    pub name: Option<String>,
    pub status: PersonalAccessTokenStatus,
    pub scopes: Vec<AuthorizationScope>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccessTokenListResponse {
    pub tokens: Vec<PersonalAccessTokenMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccessTokenRevokeResponse {
    pub token: PersonalAccessTokenMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestErrorCode {
    AuthorizationPending,
    AuthorizationDenied,
    DeviceCodeExpired,
    DeviceCodeConsumed,
    DeviceAuthorizationUnavailable,
    InvalidDeviceCode,
    InvalidUserCode,
    SlowDown,
    InvalidRequest,
    EndpointNotFound,
    EndpointExpired,
    NotAuthenticated,
    NotAuthorized,
    InvalidToken,
    AccessTokenExpired,
    TokenRevoked,
    PersonalAccessTokenExpired,
    InsufficientScope,
    RefreshTokenExpired,
    RefreshTokenRevoked,
    RefreshTokenReuseDetected,
    SessionExpired,
    SessionRevoked,
    AccountDisabled,
    IdempotencyKeyRequired,
    IdempotencyKeyConflict,
    TemporaryEndpointDeleteNotSupported,
    TemporaryEndpointStreamNotSupported,
    EventLimitExceeded,
    RateLimitExceeded,
    RateLimitUnavailable,
    StreamConcurrencyLimitExceeded,
    StreamDailyQuotaExceeded,
    EventNotFound,
    BodyNotFound,
    InternalError,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestErrorResponse {
    pub error: RestErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestErrorDetail {
    pub code: RestErrorCode,
    pub message: String,
}

impl fmt::Display for RestErrorResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error.message)
    }
}

impl std::error::Error for RestErrorResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_scopes_preserve_wire_names_and_order() {
        let value = serde_json::to_value(AUTHORIZATION_SCOPES).unwrap_or_default();
        assert_eq!(
            value,
            serde_json::json!([
                "endpoints:read",
                "endpoints:write",
                "events:read",
                "tokens:read",
                "tokens:write",
                "mcp:use"
            ])
        );
        assert_eq!(
            "events:read".parse::<AuthorizationScope>(),
            Ok(AuthorizationScope::EventsRead)
        );
        assert!("events:write".parse::<AuthorizationScope>().is_err());
    }

    #[test]
    fn token_expiration_distinguishes_omitted_null_and_seconds() {
        let request = |expires_in| PersonalAccessTokenCreateRequest {
            name: None,
            scopes: vec![AuthorizationScope::EventsRead],
            expires_in,
        };

        assert_eq!(
            serde_json::to_value(request(None)).unwrap_or_default(),
            serde_json::json!({"scopes": ["events:read"]})
        );
        assert_eq!(
            serde_json::to_value(request(Some(None))).unwrap_or_default(),
            serde_json::json!({"scopes": ["events:read"], "expires_in": null})
        );
        assert_eq!(
            serde_json::to_value(request(Some(Some(86_400)))).unwrap_or_default(),
            serde_json::json!({"scopes": ["events:read"], "expires_in": 86_400})
        );
    }

    #[test]
    fn token_id_validation_is_strict() {
        let valid = format!("tok_{}", "A0z".repeat(8));
        assert!(is_token_id(&valid));
        assert!(!is_token_id("tok_short"));
        assert!(!is_token_id(&format!("tok_{}_-", "A".repeat(22))));
        assert!(validate_id("ep_example", IdKind::Endpoint).is_ok());
        assert!(validate_id("example", IdKind::Endpoint).is_err());
    }

    #[test]
    fn bearer_token_round_trips_and_rejects_ambiguous_segments() {
        let token_id = format!("tok_{}", "A0z".repeat(8));
        let secret = "B1y".repeat(10) + "B1";
        let bearer = format_pat_bearer_token_string(&token_id, &secret).unwrap_or_default();
        let parsed = parse_bearer_token_string(&bearer);

        assert_eq!(
            parsed,
            Some(ParsedBearerToken {
                token_type: BearerTokenType::Pat,
                token_id_suffix: "A0z".repeat(8),
                secret: secret.clone(),
            })
        );
        assert_eq!(token_id_from_bearer_token_string(&bearer), Some(token_id));
        assert!(parse_bearer_token_string(&bearer.replace("pat", "pat_bad")).is_none());
        assert!(parse_bearer_token_string("bst_pat_short_secret").is_none());
    }

    #[test]
    fn event_contract_uses_distinct_rest_and_stream_path_fields() {
        let payload: EventStreamPayload = serde_json::from_value(serde_json::json!({
            "id": "evt_1",
            "endpoint_id": "ep_1",
            "received_at": "2026-07-05T12:04:32.000Z",
            "request": {
                "method": "POST",
                "path": "/webhook",
                "query": {"tag": ["one", "two"]},
                "headers": {"content-type": "application/json"},
                "body_size": 2,
                "body_sha256": "hash"
            },
            "body": {"encoding": "base64", "data": "e30="}
        }))
        .unwrap_or_else(|error| panic!("valid event payload: {error}"));

        assert_eq!(payload.request.path, "/webhook");
        assert_eq!(
            payload.request.query.get("tag"),
            Some(&QueryParameterValue::Multiple(vec![
                "one".to_owned(),
                "two".to_owned()
            ]))
        );
        assert_eq!(
            serde_json::to_value(payload.body.encoding).unwrap_or_default(),
            "base64"
        );
    }

    #[test]
    fn unknown_error_codes_keep_the_backend_message_parseable() {
        let error: RestErrorResponse = serde_json::from_value(serde_json::json!({
            "error": {"code": "future_error", "message": "Future backend error."}
        }))
        .unwrap_or_else(|error| panic!("error response should parse: {error}"));

        assert_eq!(error.error.code, RestErrorCode::Unknown);
        assert_eq!(error.error.message, "Future backend error.");
    }
}
