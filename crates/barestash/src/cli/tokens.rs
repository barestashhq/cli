use std::{fmt, str::FromStr};

use clap::{Args, Subcommand, ValueEnum};

/// `barestash tokens` arguments.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct TokensCommand {
    #[command(subcommand)]
    pub action: TokenAction,
}

/// Personal Access Token actions.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum TokenAction {
    /// Issue a Personal Access Token.
    Create(TokenCreateArgs),

    /// List API tokens.
    List(TokenListArgs),

    /// Revoke an API token.
    Revoke(TokenRevokeArgs),
}

/// Arguments for `tokens create`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct TokenCreateArgs {
    /// Assign a human-readable name.
    #[arg(long, value_name = "name")]
    pub name: Option<String>,

    /// Add a token scope; may be repeated.
    #[arg(
        long = "scope",
        value_name = "scope",
        value_enum,
        action = clap::ArgAction::Append
    )]
    pub scopes: Vec<TokenScope>,

    /// Use a predefined scope set.
    #[arg(long, value_name = "preset", value_enum)]
    pub preset: Option<TokenPreset>,

    /// Set expiration using days or years, for example `30d` or `1y`.
    #[arg(long, value_name = "duration")]
    pub expires_in: Option<TokenExpiration>,

    /// Create a token that does not expire.
    #[arg(long)]
    pub no_expiration: bool,

    /// Print JSON output, including the one-time token secret.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `tokens list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct TokenListArgs {
    /// Include revoked and expired tokens.
    #[arg(long)]
    pub all: bool,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `tokens revoke`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct TokenRevokeArgs {
    /// Token ID to revoke.
    #[arg(value_name = "token-id")]
    pub token_id: String,

    /// Revoke without prompting.
    #[arg(long)]
    pub yes: bool,
}

/// Scope accepted by `tokens create --scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum TokenScope {
    #[value(name = "endpoints:read")]
    EndpointsRead,
    #[value(name = "endpoints:write")]
    EndpointsWrite,
    #[value(name = "events:read")]
    EventsRead,
    #[value(name = "tokens:read")]
    TokensRead,
    #[value(name = "tokens:write")]
    TokensWrite,
    #[value(name = "mcp:use")]
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

/// Scope preset accepted by `tokens create --preset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_expiration_to_api_seconds() {
        assert_eq!(
            "30d"
                .parse::<TokenExpiration>()
                .expect("30 days")
                .as_seconds(),
            2_592_000
        );
        assert_eq!(
            "90d"
                .parse::<TokenExpiration>()
                .expect("90 days")
                .as_seconds(),
            7_776_000
        );
        assert_eq!(
            "1y".parse::<TokenExpiration>()
                .expect("one year")
                .as_seconds(),
            31_536_000
        );
    }

    #[test]
    fn rejects_invalid_token_expiration() {
        for value in ["0d", "90days", "-1d", "1s", "18446744073709551615y"] {
            assert!(
                value.parse::<TokenExpiration>().is_err(),
                "accepted {value}"
            );
        }
    }
}
