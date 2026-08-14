use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;

use barestash_client::ApiClientError;
use barestash_local_state::StoredCredential;
use barestash_protocol::{
    AccountCredential, BearerTokenType, RestErrorCode, TOKEN_ID_PREFIX, parse_bearer_token_string,
};

use crate::{AppContext, CliError};

use super::{
    AuthLogoutArgs, authorization_headers, clear_stored_credential, invalid_json_response,
    print_logged_out, refresh_stored_session_after_access_token_expired,
    validate_token_without_refresh,
};

pub(super) async fn run(context: &AppContext, arguments: AuthLogoutArgs) -> Result<(), CliError> {
    if !arguments.revoke {
        let _guard = context
            .credential_lock
            .acquire()
            .await
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        clear_stored_credential(context).await?;
        return print_logged_out().map_err(Into::into);
    }

    let credential = {
        let _guard = context
            .credential_lock
            .acquire()
            .await
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        context.stored_credential().await?
    };

    let Some(stored) = credential.as_ref() else {
        return Err(CliError::Local(
            "No stored authentication credential is configured.".into(),
        ));
    };
    if let Some(target) = resolve_logout_revoke_target(context, stored).await? {
        revoke_logout_target(context, &target).await?;
    }

    let _guard = context
        .credential_lock
        .acquire()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let current = context.stored_credential().await?;
    if credentials_equal(current.as_ref(), credential.as_ref()) {
        clear_stored_credential(context).await?;
    }
    print_logged_out().map_err(Into::into)
}

#[derive(Debug)]
pub(super) struct LogoutRevokeTarget {
    pub(super) path: String,
    pub(super) method: Method,
    pub(super) token: String,
    pub(super) is_cli_session: bool,
    pub(super) allow_access_token_refresh: bool,
    pub(super) session_id: Option<String>,
}

async fn resolve_logout_revoke_target(
    context: &AppContext,
    credential: &StoredCredential,
) -> Result<Option<LogoutRevokeTarget>, CliError> {
    let token = match credential {
        StoredCredential::CliSession { access_token, .. } => access_token.clone(),
        StoredCredential::PersonalAccessToken { token } => token.clone(),
    };
    if let StoredCredential::CliSession { session_id, .. } = credential {
        return Ok(Some(LogoutRevokeTarget {
            path: "/v1/auth/sessions/current/revoke".into(),
            method: Method::POST,
            token,
            is_cli_session: true,
            allow_access_token_refresh: true,
            session_id: Some(session_id.clone()),
        }));
    }
    if let Some(parsed) = parse_bearer_token_string(&token) {
        match parsed.token_type {
            BearerTokenType::Access => {
                return Ok(Some(LogoutRevokeTarget {
                    path: "/v1/auth/sessions/current/revoke".into(),
                    method: Method::POST,
                    token,
                    is_cli_session: true,
                    allow_access_token_refresh: false,
                    session_id: None,
                }));
            }
            BearerTokenType::Pat => {
                return Ok(Some(LogoutRevokeTarget {
                    path: format!("/v1/tokens/{TOKEN_ID_PREFIX}{}", parsed.token_id_suffix),
                    method: Method::DELETE,
                    token,
                    is_cli_session: false,
                    allow_access_token_refresh: false,
                    session_id: None,
                }));
            }
            BearerTokenType::Refresh => {}
        }
    }

    let account = match validate_token_without_refresh(context, &token).await {
        Ok(account) => account,
        Err(CliError::Api(error))
            if matches!(
                error.error.code,
                RestErrorCode::TokenRevoked | RestErrorCode::PersonalAccessTokenExpired
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    Ok(Some(match account.credential {
        AccountCredential::CliAccessToken { .. } => LogoutRevokeTarget {
            path: "/v1/auth/sessions/current/revoke".into(),
            method: Method::POST,
            token,
            is_cli_session: true,
            allow_access_token_refresh: false,
            session_id: None,
        },
        AccountCredential::PersonalAccessToken { id, .. } => LogoutRevokeTarget {
            path: format!("/v1/tokens/{id}"),
            method: Method::DELETE,
            token,
            is_cli_session: false,
            allow_access_token_refresh: false,
            session_id: None,
        },
    }))
}

pub(super) async fn revoke_logout_target(
    context: &AppContext,
    target: &LogoutRevokeTarget,
) -> Result<(), CliError> {
    let mut token = target.token.clone();
    let mut result =
        explicit_json_request::<Value>(context, target.method.clone(), &target.path, &token).await;

    if matches!(
        &result,
        Err(ApiClientError::Api { status, error, .. })
            if *status == StatusCode::UNAUTHORIZED
                && error.error.code == RestErrorCode::AccessTokenExpired
    ) && target.allow_access_token_refresh
    {
        match refresh_stored_session_after_access_token_expired(
            context,
            &token,
            target.session_id.as_deref(),
            true,
        )
        .await
        {
            Ok(Some(refreshed)) => {
                token = refreshed;
                result = explicit_json_request::<Value>(
                    context,
                    target.method.clone(),
                    &target.path,
                    &token,
                )
                .await;
            }
            Ok(None) => {}
            Err(CliError::Api(error))
                if target.is_cli_session
                    && matches!(
                        error.error.code,
                        RestErrorCode::SessionRevoked | RestErrorCode::SessionExpired
                    ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }

    match result {
        Ok(_) => Ok(()),
        Err(ApiClientError::Api { error, .. })
            if logout_error_confirms_revocation(error.error.code, target.is_cli_session) =>
        {
            Ok(())
        }
        Err(error) => Err(CliError::from_api_client(error)),
    }
}

async fn explicit_json_request<T: DeserializeOwned>(
    context: &AppContext,
    method: Method,
    path: &str,
    token: &str,
) -> Result<T, ApiClientError> {
    // Header construction failure cannot safely include the token. Convert it
    // through a synthetic API error rather than exposing header contents.
    let Ok(headers) = authorization_headers(Some(token.to_owned())) else {
        return Err(ApiClientError::Api {
            status: StatusCode::BAD_REQUEST,
            error: invalid_json_response(),
            retry_after: None,
        });
    };
    context
        .api()
        .request_json(method, path, Some(headers), None)
        .await
}

pub(super) fn logout_error_confirms_revocation(code: RestErrorCode, is_cli_session: bool) -> bool {
    if is_cli_session {
        matches!(
            code,
            RestErrorCode::TokenRevoked
                | RestErrorCode::SessionRevoked
                | RestErrorCode::SessionExpired
        )
    } else {
        matches!(
            code,
            RestErrorCode::TokenRevoked | RestErrorCode::PersonalAccessTokenExpired
        )
    }
}

pub(super) fn credentials_equal(
    left: Option<&StoredCredential>,
    right: Option<&StoredCredential>,
) -> bool {
    if left == right {
        return true;
    }
    matches!(
        (left, right),
        (
            Some(StoredCredential::CliSession { session_id: left, .. }),
            Some(StoredCredential::CliSession { session_id: right, .. })
        ) if left == right
    )
}
