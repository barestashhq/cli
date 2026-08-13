use std::io::{self, Write};

use crate::error::CliError;
use crate::presentation::sanitize_terminal_text;
use barestash_protocol::{RestErrorCode, RestErrorResponse};

pub fn print_cli_error(error: &CliError) {
    if matches!(error, CliError::AlreadyReported) {
        return;
    }

    let mut stderr = io::stderr().lock();
    match error {
        CliError::Api(response) => write_api_error(&mut stderr, response, None),
        _ => {
            for line in error.to_string().split('\n') {
                let _ = writeln!(stderr, "{}", sanitize_terminal_text(line));
            }
        }
    }
}

fn write_api_error(
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

    if response.error.code == RestErrorCode::StreamDailyQuotaExceeded
        && let Some(seconds) = retry_after
    {
        let _ = writeln!(output);
        let _ = writeln!(output, "Retry-After: {seconds} seconds.");
    }
}
