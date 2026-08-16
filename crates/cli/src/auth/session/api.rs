use reqwest::Method;
use serde_json::Value;

use barestash_client::ApiClientError;
use barestash_protocol::{AccountResponse, RestErrorCode, RestErrorDetail, RestErrorResponse};

use crate::{AppContext, CliError};

use super::bearer::authorization_headers;

pub(in crate::auth) async fn validate_token_without_refresh(
    context: &AppContext,
    token: &str,
) -> Result<AccountResponse, CliError> {
    let headers = authorization_headers(Some(token.to_owned()))?;
    context
        .api()
        .request_json(Method::GET, "/v1/account", Some(headers), None)
        .await
        .map_err(CliError::from_api_client)
}

pub(in crate::auth) async fn revoke_cli_session_best_effort(
    context: &AppContext,
    access_token: &str,
    warning: &str,
) {
    let Ok(headers) = authorization_headers(Some(access_token.to_owned())) else {
        eprintln!("{warning}");
        return;
    };
    let result: Result<Value, ApiClientError> = context
        .api()
        .request_json(
            Method::POST,
            "/v1/auth/sessions/current/revoke",
            Some(headers),
            None,
        )
        .await;
    match result {
        Ok(_) => {}
        Err(ApiClientError::Api { error, .. })
            if matches!(
                error.error.code,
                RestErrorCode::TokenRevoked
                    | RestErrorCode::SessionRevoked
                    | RestErrorCode::SessionExpired
            ) => {}
        Err(_) => eprintln!("{warning}"),
    }
}

pub(in crate::auth) fn invalid_json_response() -> RestErrorResponse {
    RestErrorResponse {
        error: RestErrorDetail {
            code: RestErrorCode::InternalError,
            message: "Barestash API returned a response that was not valid JSON.".into(),
        },
    }
}
