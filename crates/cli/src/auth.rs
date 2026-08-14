use crate::{AppContext, CliError};

mod args;
mod login;
mod logout;
mod session;
mod status;
mod view;

pub(crate) use args::*;
pub(crate) use view::*;

pub(crate) use session::{
    AuthMode, auth_headers, authenticated_request_json, authenticated_send,
    refresh_after_access_token_expired,
};

use session::{
    add_seconds_iso, authorization_headers, clear_legacy_config_token, clear_stored_credential,
    invalid_json_response, refresh_stored_session_after_access_token_expired, resolve_auth_token,
    revoke_cli_session_best_effort, validate_token_without_refresh,
};

/// Executes an authentication subcommand.
///
/// # Errors
///
/// Returns a local, persistence, transport, API, or output error encountered
/// while executing the command.
pub(crate) async fn run(
    context: &AppContext,
    command: AuthCommand,
    client_version: &str,
) -> Result<(), CliError> {
    match command.action {
        AuthAction::Login(arguments) => login::run(context, arguments, client_version).await,
        AuthAction::Status(arguments) => status::run(context, arguments).await,
        AuthAction::Logout(arguments) => logout::run(context, arguments).await,
    }
}
