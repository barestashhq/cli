use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_presentation::{AuthStatusView, print_auth_status};
use barestash_protocol::AccountResponse;

use crate::command::AuthStatusArgs;
use crate::{AppContext, CliError};

use super::{AuthMode, authenticated_request_json, resolve_auth_token};

pub(super) async fn run(context: &AppContext, arguments: AuthStatusArgs) -> Result<(), CliError> {
    let token = resolve_auth_token(context).await?;
    let config = context
        .config
        .read()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let Some(_) = token else {
        return print_auth_status(
            AuthStatusView {
                principal: None,
                default_endpoint: config.default_endpoint.as_deref(),
            },
            arguments.json,
        )
        .map_err(Into::into);
    };

    let principal: AccountResponse = authenticated_request_json(
        context,
        Method::GET,
        "/v1/account",
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;
    print_auth_status(
        AuthStatusView {
            principal: Some(&principal),
            default_endpoint: config.default_endpoint.as_deref(),
        },
        arguments.json,
    )
    .map_err(Into::into)
}
