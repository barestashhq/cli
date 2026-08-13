use std::io::{self, Write};

use crate::infrastructure::api::ApiClientError;
use crate::presentation::sanitize_terminal_text;
use crate::protocol::{RestErrorCode, RestErrorResponse};

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Local(String),
    #[error("{0}")]
    Api(#[from] RestErrorResponse),
    #[error("Failed to reach Barestash API.\n{0}")]
    Connectivity(String),
    #[error("{0}")]
    Infrastructure(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("diagnostic already reported")]
    AlreadyReported,
}

impl CliError {
    pub fn print(&self) {
        if matches!(self, Self::Cancelled | Self::AlreadyReported) {
            return;
        }

        let mut stderr = io::stderr().lock();
        match self {
            Self::Api(response) => print_api_error(&mut stderr, response, None),
            _ => {
                for line in self.to_string().split('\n') {
                    let _ = writeln!(stderr, "{}", sanitize_terminal_text(line));
                }
            }
        }
    }

    pub fn api_with_retry_after(response: &RestErrorResponse, retry_after: Option<u64>) {
        let mut stderr = io::stderr().lock();
        print_api_error(&mut stderr, response, retry_after);
    }

    /// Converts the infrastructure error without exposing request internals or
    /// credentials in user-facing diagnostics.
    pub fn from_api_client(error: ApiClientError) -> Self {
        match error {
            ApiClientError::InvalidUrl(error) => Self::Local(error.to_string()),
            ApiClientError::Api { error, .. } => Self::Api(error),
            other => Self::Connectivity(other.to_string()),
        }
    }
}

fn print_api_error(
    output: &mut impl Write,
    response: &RestErrorResponse,
    retry_after: Option<u64>,
) {
    let _ = writeln!(
        output,
        "{}",
        sanitize_terminal_text(&response.error.message)
    );

    let lines: &[&str] = match response.error.code {
        RestErrorCode::StreamConcurrencyLimitExceeded => {
            &["", "Close another live stream before retrying."]
        }
        RestErrorCode::EndpointExpired => &[
            "",
            "Create and set a new default endpoint:",
            "  barestash endpoints create --set-default",
        ],
        RestErrorCode::EventLimitExceeded => &[
            "",
            "Create a new endpoint if you need to continue capturing events:",
            "  barestash endpoints create --set-default",
        ],
        RestErrorCode::TemporaryEndpointDeleteNotSupported => &[
            "",
            "Temporary endpoints expire automatically after 24 hours.",
            "Deletion is not supported in MVP.",
            "",
            "Create a new temporary endpoint if needed:",
            "  barestash endpoints create --temporary",
        ],
        RestErrorCode::TemporaryEndpointStreamNotSupported => &[
            "",
            "Use polling instead:",
            "  barestash events tail --endpoint ep_abc123",
            "",
            "Or create and set a private endpoint:",
            "  barestash endpoints create --private --set-default",
        ],
        RestErrorCode::EndpointNotFound => &[
            "",
            "Create and set a new default endpoint:",
            "  barestash endpoints create --set-default",
            "",
            "List available endpoints:",
            "  barestash endpoints list",
        ],
        RestErrorCode::NotAuthenticated => &["", "Run:", "  barestash auth login"],
        RestErrorCode::RefreshTokenExpired
        | RestErrorCode::RefreshTokenRevoked
        | RestErrorCode::RefreshTokenReuseDetected
        | RestErrorCode::SessionExpired
        | RestErrorCode::SessionRevoked
        | RestErrorCode::AccountDisabled => &["", "Authenticate again:", "  barestash auth login"],
        RestErrorCode::PersonalAccessTokenExpired => &[
            "",
            "Create a new Personal Access Token from an interactive session:",
            "  barestash tokens create",
        ],
        RestErrorCode::InsufficientScope => &[
            "",
            "Create a token with the required scopes from an interactive session:",
            "  barestash tokens create",
        ],
        _ => &[],
    };

    for line in lines {
        let _ = writeln!(output, "{line}");
    }

    if response.error.code == RestErrorCode::StreamDailyQuotaExceeded {
        if let Some(seconds) = retry_after {
            let _ = writeln!(output);
            let _ = writeln!(output, "Retry-After: {seconds} seconds.");
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(value: std::io::Error) -> Self {
        Self::Infrastructure(value.to_string())
    }
}

impl From<ApiClientError> for CliError {
    fn from(value: ApiClientError) -> Self {
        Self::from_api_client(value)
    }
}
