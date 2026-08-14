//! Clap-independent command inputs consumed by the application layer.

use std::{fmt, str::FromStr};

/// Process-wide execution settings resolved by the CLI facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOptions {
    /// Permit a private or link-local API URL for this invocation.
    pub allow_insecure_api_url: bool,
    /// Version of the invoking CLI, sent during device authorization.
    pub client_version: String,
}

/// Top-level application command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    Auth(AuthCommand),
    Endpoints(EndpointsCommand),
    Events(EventsCommand),
    Tokens(TokensCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCommand {
    pub action: AuthAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthAction {
    Login(AuthLoginArgs),
    Status(AuthStatusArgs),
    Logout(AuthLogoutArgs),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthLoginArgs {
    pub with_token: bool,
    pub insecure_storage: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthStatusArgs {
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthLogoutArgs {
    pub revoke: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointsCommand {
    pub action: EndpointAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointAction {
    Create(EndpointCreateArgs),
    List(EndpointListArgs),
    Show(EndpointShowArgs),
    Delete(EndpointDeleteArgs),
    Secrets(EndpointSecretsCommand),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointCreateArgs {
    pub private: bool,
    pub temporary: bool,
    pub name: Option<String>,
    pub set_default: bool,
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointListArgs {
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointShowArgs {
    pub endpoint_id: String,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointDeleteArgs {
    pub endpoint_id: String,
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSecretsCommand {
    pub action: EndpointSecretsAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointSecretsAction {
    Create(EndpointSecretCreateArgs),
    List(EndpointSecretListArgs),
    Revoke(EndpointSecretRevokeArgs),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointSecretCreateArgs {
    pub endpoint: Option<String>,
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointSecretListArgs {
    pub endpoint: Option<String>,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSecretRevokeArgs {
    pub secret_id: String,
    pub endpoint: Option<String>,
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsCommand {
    pub action: EventAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventAction {
    List(EventListArgs),
    Latest(EventLatestArgs),
    Show(EventShowArgs),
    Tail(EventTailArgs),
    Stream(EventStreamArgs),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventListArgs {
    pub endpoint: Option<String>,
    pub limit: Option<String>,
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventLatestArgs {
    pub endpoint: Option<String>,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventShowArgs {
    pub event_id: String,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventTailArgs {
    pub endpoint: Option<String>,
    pub last: String,
    pub headers: bool,
    pub body: bool,
    pub view: bool,
    pub poll_interval: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventStreamArgs {
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokensCommand {
    pub action: TokenAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenAction {
    Create(TokenCreateArgs),
    List(TokenListArgs),
    Revoke(TokenRevokeArgs),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenCreateArgs {
    pub name: Option<String>,
    pub scopes: Vec<TokenScope>,
    pub preset: Option<TokenPreset>,
    pub expires_in: Option<TokenExpiration>,
    pub no_expiration: bool,
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenListArgs {
    pub all: bool,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRevokeArgs {
    pub token_id: String,
    pub yes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenScope {
    EndpointsRead,
    EndpointsWrite,
    EventsRead,
    TokensRead,
    TokensWrite,
    McpUse,
}

impl TokenScope {
    #[must_use]
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

impl fmt::Display for TokenScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPreset {
    ReadOnly,
    FullAccess,
}

/// Validated Personal Access Token lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenExpiration {
    seconds: u64,
}

impl TokenExpiration {
    #[must_use]
    pub const fn from_seconds(seconds: u64) -> Self {
        Self { seconds }
    }

    #[must_use]
    pub const fn as_seconds(self) -> u64 {
        self.seconds
    }
}

impl fmt::Display for TokenExpiration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}s", self.seconds)
    }
}

impl FromStr for TokenExpiration {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (amount, days_per_unit) = if let Some(amount) = value.strip_suffix('d') {
            (amount, 1_u64)
        } else if let Some(amount) = value.strip_suffix('y') {
            (amount, 365)
        } else {
            return Err("Token expiration must include a unit: d or y.".to_owned());
        };

        if amount.is_empty() || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("Token expiration must include a unit: d or y.".to_owned());
        }

        let amount = amount
            .parse::<u64>()
            .map_err(|_| "Token expiration is too large.".to_owned())?;
        if amount == 0 {
            return Err("Token expiration must be a positive duration.".to_owned());
        }

        let seconds = amount
            .checked_mul(days_per_unit)
            .and_then(|days| days.checked_mul(24 * 60 * 60))
            .ok_or_else(|| "Token expiration is too large.".to_owned())?;

        Ok(Self { seconds })
    }
}
