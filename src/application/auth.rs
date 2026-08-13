use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::{AppContext, CliError};
use crate::cli::auth::{AuthAction, AuthCommand, AuthLoginArgs, AuthLogoutArgs, AuthStatusArgs};
use crate::domain::StoredCredential;
use crate::infrastructure::api::ApiClientError;
use crate::infrastructure::browser::open_browser;
use crate::infrastructure::credentials::CredentialWriteResult;
use crate::infrastructure::terminal::read_stdin_to_string;
use crate::presentation::{
    OutputRenderer, TerminalCapabilities, print_json, print_lines, sanitize_terminal_text,
};
use crate::protocol::{
    AUTHORIZATION_SCOPES, AccountCredential, AccountResponse, BearerTokenType,
    DeviceAuthorizationCreateRequest, DeviceAuthorizationCreateResponse, DeviceTokenRequest,
    DeviceTokenResponse, RefreshGrantType, RefreshTokenRequest, RefreshTokenResponse,
    RestErrorCode, RestErrorDetail, RestErrorResponse, TOKEN_ID_PREFIX, parse_bearer_token_string,
};

const REFRESH_WINDOW: TimeDelta = TimeDelta::minutes(5);
const ISSUED_SESSION_CLEANUP_WARNING: &str = "Unable to revoke the newly issued CLI session. The newly issued remote CLI session may still be active.";
const EXPIRED_CREDENTIAL_CLEANUP_WARNING: &str = "Unable to clear the expired stored authentication credential. Run `barestash auth logout` after the credential store becomes available.";
const ROTATED_CREDENTIAL_CLEANUP_WARNING: &str = "Unable to clear the stale stored authentication credential after refresh persistence failed. Run `barestash auth logout` before authenticating again.";
const ROTATED_SESSION_CLEANUP_WARNING: &str = "Unable to revoke the rotated CLI session after refresh persistence failed. The remote CLI session may still be active.";
const LEGACY_CONFIG_CLEANUP_WARNING: &str = "Unable to remove the legacy authentication token from the config file. The newly stored credential will still be used.";

/// Whether an authenticated request may continue without a usable local
/// credential. Public reads deliberately degrade to an unauthenticated
/// request when session refresh is rejected or temporarily unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthMode {
    Required,
    PublicRead,
}

/// Executes an authentication subcommand.
///
/// # Errors
///
/// Returns a local, persistence, transport, API, or output error encountered
/// while executing the command.
pub async fn run(context: &AppContext, command: AuthCommand) -> Result<(), CliError> {
    match command.action {
        AuthAction::Login(arguments) => login(context, arguments).await,
        AuthAction::Status(arguments) => status(context, arguments).await,
        AuthAction::Logout(arguments) => logout(context, arguments).await,
    }
}

/// Resolves the effective bearer token in compatibility order: non-empty
/// `BARESTASH_TOKEN`, stored credential, then the legacy config token.
///
/// # Errors
///
/// Returns an error when credential/config access or session refresh fails.
pub async fn resolve_auth_token(context: &AppContext) -> Result<Option<String>, CliError> {
    if let Some(token) = context.environment_token() {
        return Ok(Some(token.to_owned()));
    }

    let _guard = context
        .credential_lock
        .acquire()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let credential = context.stored_credential().await?;
    match credential {
        None => Ok(None),
        Some(StoredCredential::PersonalAccessToken { token }) => Ok(Some(token)),
        Some(StoredCredential::CliSession {
            session_id,
            access_token,
            refresh_token,
            access_token_expires_at,
            refresh_token_expires_at,
            scopes,
        }) => {
            let credential = StoredCredential::CliSession {
                session_id,
                access_token: access_token.clone(),
                refresh_token,
                access_token_expires_at,
                refresh_token_expires_at,
                scopes,
            };
            if access_token_is_fresh(&credential, context.now()) {
                Ok(Some(access_token))
            } else {
                refresh_credential_locked(context, credential)
                    .await
                    .map(Some)
            }
        }
    }
}

/// Builds an Authorization header for the currently resolved credential.
///
/// # Errors
///
/// Returns an error when token resolution, refresh, or header validation fails.
pub async fn auth_headers(context: &AppContext) -> Result<HeaderMap, CliError> {
    authorization_headers(resolve_auth_token(context).await?)
}

/// Builds optional Authorization headers for a public-read request.
///
/// # Errors
///
/// Returns persistence and local validation failures. Refresh API and network
/// failures intentionally degrade to an empty header map.
pub async fn public_read_auth_headers(context: &AppContext) -> Result<HeaderMap, CliError> {
    match auth_headers(context).await {
        Ok(headers) => Ok(headers),
        Err(CliError::Api(_) | CliError::Connectivity(_)) => Ok(HeaderMap::new()),
        Err(error) => Err(error),
    }
}

/// Refreshes a stored CLI session only when it still contains the rejected
/// access token. This compare-under-lock rule prevents concurrent requests
/// from rotating the same refresh token twice.
///
/// # Errors
///
/// Returns an error when credential access, refresh, or rotation persistence
/// fails.
pub async fn refresh_after_access_token_expired(
    context: &AppContext,
    expired_access_token: &str,
) -> Result<Option<String>, CliError> {
    refresh_stored_session_after_access_token_expired(context, expired_access_token, None, false)
        .await
}

async fn refresh_stored_session_after_access_token_expired(
    context: &AppContext,
    expired_access_token: &str,
    expected_session_id: Option<&str>,
    ignore_environment_token: bool,
) -> Result<Option<String>, CliError> {
    if !ignore_environment_token && context.environment_token().is_some() {
        return Ok(None);
    }
    let _guard = context
        .credential_lock
        .acquire()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let Some(StoredCredential::CliSession {
        session_id,
        access_token,
        refresh_token,
        access_token_expires_at,
        refresh_token_expires_at,
        scopes,
    }) = context.stored_credential().await?
    else {
        return Ok(None);
    };
    if expected_session_id.is_some_and(|expected| expected != session_id) {
        return Ok(None);
    }
    if access_token != expired_access_token {
        return Ok(Some(access_token));
    }
    let credential = StoredCredential::CliSession {
        session_id,
        access_token,
        refresh_token,
        access_token_expires_at,
        refresh_token_expires_at,
        scopes,
    };
    refresh_credential_locked(context, credential)
        .await
        .map(Some)
}

/// Authenticated JSON request with one reactive refresh/retry. Extra headers
/// may not override the resolved Authorization value.
///
/// # Errors
///
/// Returns a credential, transport, API, response-decoding, or refresh error.
pub async fn authenticated_request_json<T: DeserializeOwned>(
    context: &AppContext,
    method: Method,
    path: &str,
    mut extra_headers: HeaderMap,
    body: Option<Value>,
    mode: AuthMode,
) -> Result<T, CliError> {
    extra_headers.remove(AUTHORIZATION);
    let resolved = match mode {
        AuthMode::Required => auth_headers(context).await?,
        AuthMode::PublicRead => public_read_auth_headers(context).await?,
    };
    extra_headers.extend(resolved);
    let expired_token = bearer_token(&extra_headers).map(str::to_owned);

    match context
        .api
        .request_json(
            method.clone(),
            path,
            Some(extra_headers.clone()),
            body.clone(),
        )
        .await
    {
        Ok(value) => Ok(value),
        Err(ApiClientError::Api { status, error, .. })
            if status == StatusCode::UNAUTHORIZED
                && error.error.code == RestErrorCode::AccessTokenExpired =>
        {
            let Some(expired_token) = expired_token else {
                return Err(CliError::Api(error));
            };
            let refreshed_token =
                match refresh_after_access_token_expired(context, &expired_token).await {
                    Ok(Some(token)) => token,
                    Ok(None) if mode == AuthMode::PublicRead => {
                        extra_headers.remove(AUTHORIZATION);
                        return context
                            .api
                            .request_json(method, path, Some(extra_headers), body)
                            .await
                            .map_err(CliError::from_api_client);
                    }
                    Ok(None) => return Err(CliError::Api(error)),
                    Err(CliError::Api(_) | CliError::Connectivity(_))
                        if mode == AuthMode::PublicRead =>
                    {
                        extra_headers.remove(AUTHORIZATION);
                        return context
                            .api
                            .request_json(method, path, Some(extra_headers), body)
                            .await
                            .map_err(CliError::from_api_client);
                    }
                    Err(refresh_error) => return Err(refresh_error),
                };
            insert_bearer(&mut extra_headers, &refreshed_token)?;
            context
                .api
                .request_json(method, path, Some(extra_headers), body)
                .await
                .map_err(|retry_error| match retry_error {
                    ApiClientError::Api { error, .. } => CliError::Api(error),
                    other => CliError::from_api_client(other),
                })
        }
        Err(error) => Err(CliError::from_api_client(error)),
    }
}

/// Raw-response equivalent of [`authenticated_request_json`]. A 401 JSON
/// response with `access_token_expired` is consumed, refreshed, and retried
/// exactly once; other responses remain available to streaming callers.
///
/// # Errors
///
/// Returns a credential, transport, API, response-decoding, or refresh error.
pub async fn authenticated_send(
    context: &AppContext,
    method: Method,
    path: &str,
    mut extra_headers: HeaderMap,
    body: Option<Value>,
    mode: AuthMode,
) -> Result<Response, CliError> {
    extra_headers.remove(AUTHORIZATION);
    let resolved = match mode {
        AuthMode::Required => auth_headers(context).await?,
        AuthMode::PublicRead => public_read_auth_headers(context).await?,
    };
    extra_headers.extend(resolved);
    let expired_token = bearer_token(&extra_headers).map(str::to_owned);
    let response = send_raw_once(
        context,
        method.clone(),
        path,
        extra_headers.clone(),
        body.clone(),
    )
    .await
    .map_err(CliError::from_api_client)?;
    if response.status() != StatusCode::UNAUTHORIZED {
        return Ok(response);
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| CliError::Connectivity(error.to_string()))?;
    let error = serde_json::from_slice(&bytes).unwrap_or_else(|_| invalid_json_response());
    if error.error.code != RestErrorCode::AccessTokenExpired {
        return Err(CliError::Api(error));
    }
    let Some(expired_token) = expired_token else {
        return Err(CliError::Api(error));
    };
    let Some(refreshed_token) = refresh_after_access_token_expired(context, &expired_token).await?
    else {
        return Err(CliError::Api(error));
    };
    insert_bearer(&mut extra_headers, &refreshed_token)?;
    send_raw_once(context, method, path, extra_headers, body)
        .await
        .map_err(CliError::from_api_client)
}

async fn send_raw_once(
    context: &AppContext,
    method: Method,
    path: &str,
    headers: HeaderMap,
    body: Option<Value>,
) -> Result<Response, ApiClientError> {
    context
        .api
        .send(method, path, move |mut request| {
            request = request.headers(headers);
            if let Some(body) = body {
                request = request.json(&body);
            }
            request
        })
        .await
}

async fn login(context: &AppContext, arguments: AuthLoginArgs) -> Result<(), CliError> {
    let result = if arguments.with_token {
        login_with_token(context, arguments.insecure_storage).await?
    } else {
        login_with_device_authorization(context, arguments.insecure_storage).await?
    };

    report_storage(&result.storage);
    if result.replaced_session {
        eprintln!(
            "Replaced a stored CLI session locally. Run `barestash auth logout --revoke` before a future login to revoke the previous session remotely."
        );
    }
    print_login_result(&result)
}

async fn login_with_token(
    context: &AppContext,
    insecure_storage: bool,
) -> Result<LoginResult, CliError> {
    let token = read_stdin_to_string()?.trim().to_owned();
    if token.is_empty() {
        return Err(CliError::Local("No token provided on stdin.".into()));
    }
    let principal = validate_token_without_refresh(context, &token).await?;
    if !matches!(
        principal.credential,
        AccountCredential::PersonalAccessToken { .. }
    ) {
        return Err(CliError::Local(
            "auth login --with-token requires a Personal Access Token.".into(),
        ));
    }
    let persisted = persist_login_credential(
        context,
        &StoredCredential::PersonalAccessToken { token },
        insecure_storage,
    )
    .await?;
    Ok(LoginResult {
        principal,
        storage: persisted.0,
        replaced_session: persisted.1,
        session_expires_at: None,
    })
}

#[allow(clippy::too_many_lines)]
async fn login_with_device_authorization(
    context: &AppContext,
    insecure_storage: bool,
) -> Result<LoginResult, CliError> {
    let request = DeviceAuthorizationCreateRequest {
        client_name: "barestash-cli".into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
        device_name: device_name(context),
        requested_scopes: AUTHORIZATION_SCOPES.to_vec(),
    };
    let authorization: DeviceAuthorizationCreateResponse = context
        .api
        .request_json(
            Method::POST,
            "/v1/auth/device/authorizations",
            None,
            Some(
                serde_json::to_value(request)
                    .map_err(|error| CliError::Infrastructure(error.to_string()))?,
            ),
        )
        .await
        .map_err(CliError::from_api_client)?;

    eprintln!("Open this URL in your browser:");
    eprintln!();
    eprintln!(
        "  {}",
        sanitize_terminal_text(&authorization.verification_uri)
    );
    eprintln!();
    eprintln!("Enter this one-time code:");
    eprintln!();
    eprintln!("  {}", sanitize_terminal_text(&authorization.user_code));
    eprintln!();
    eprintln!("Waiting for authorization...");
    let _ = open_browser(&authorization.verification_uri_complete);

    let mut interval = authorization.interval;
    let expires_at = context
        .now()
        .checked_add_signed(TimeDelta::seconds(
            i64::try_from(authorization.expires_in).unwrap_or(i64::MAX),
        ))
        .unwrap_or(DateTime::<Utc>::MAX_UTC);

    while context.now() < expires_at {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let polled: Result<DeviceTokenResponse, ApiClientError> = context
            .api
            .request_json(
                Method::POST,
                "/v1/auth/device/token",
                None,
                Some(
                    serde_json::to_value(DeviceTokenRequest {
                        device_code: authorization.device_code.clone(),
                    })
                    .map_err(|error| CliError::Infrastructure(error.to_string()))?,
                ),
            )
            .await;
        let issued = match polled {
            Ok(issued) => issued,
            Err(ApiClientError::Api { error, .. })
                if error.error.code == RestErrorCode::AuthorizationPending =>
            {
                continue;
            }
            Err(ApiClientError::Api { error, .. })
                if error.error.code == RestErrorCode::SlowDown =>
            {
                interval = interval.saturating_add(5);
                continue;
            }
            Err(error) => return Err(CliError::from_api_client(error)),
        };

        let principal = match validate_token_without_refresh(context, &issued.access_token).await {
            Ok(principal) => principal,
            Err(error) => {
                revoke_cli_session_best_effort(
                    context,
                    &issued.access_token,
                    ISSUED_SESSION_CLEANUP_WARNING,
                )
                .await;
                return Err(error);
            }
        };
        let AccountCredential::CliAccessToken { session_id, .. } = &principal.credential else {
            revoke_cli_session_best_effort(
                context,
                &issued.access_token,
                ISSUED_SESSION_CLEANUP_WARNING,
            )
            .await;
            return Err(CliError::Local(
                "Device Authorization did not issue a CLI session.".into(),
            ));
        };
        let now = context.now();
        let session_expires_at = add_seconds_iso(now, issued.refresh_token_expires_in);
        let credential = StoredCredential::CliSession {
            session_id: session_id.clone(),
            access_token: issued.access_token.clone(),
            refresh_token: issued.refresh_token,
            access_token_expires_at: add_seconds_iso(now, issued.expires_in),
            refresh_token_expires_at: session_expires_at.clone(),
            scopes: issued
                .scopes
                .into_iter()
                .map(|scope| scope.to_string())
                .collect(),
        };
        let persisted = match persist_login_credential(context, &credential, insecure_storage).await
        {
            Ok(persisted) => persisted,
            Err(error) => {
                revoke_cli_session_best_effort(
                    context,
                    &issued.access_token,
                    ISSUED_SESSION_CLEANUP_WARNING,
                )
                .await;
                return Err(error);
            }
        };
        return Ok(LoginResult {
            principal,
            storage: persisted.0,
            replaced_session: persisted.1,
            session_expires_at: Some(session_expires_at),
        });
    }

    Err(CliError::Local(
        "Device Authorization expired. Run auth login again.".into(),
    ))
}

async fn status(context: &AppContext, arguments: AuthStatusArgs) -> Result<(), CliError> {
    let token = resolve_auth_token(context).await?;
    let config = context
        .config
        .read()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let Some(_) = token else {
        if arguments.json {
            return print_json(&json!({
                "authenticated": false,
                "account": Value::Null,
                "credential": Value::Null,
                "default_endpoint": config.default_endpoint,
            }));
        }
        let capabilities = TerminalCapabilities::detect();
        if capabilities.interactive {
            let renderer = OutputRenderer::new(capabilities);
            return print_lines([renderer.heading("Authentication", Some("not authenticated"))]);
        }
        return print_lines(["Not authenticated.".to_owned()]);
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
    if arguments.json {
        return print_json(&json!({
            "authenticated": true,
            "account": principal.account,
            "credential": principal.credential,
            "default_endpoint": config.default_endpoint,
        }));
    }
    print_status_human(&principal, config.default_endpoint.as_deref())
}

async fn logout(context: &AppContext, arguments: AuthLogoutArgs) -> Result<(), CliError> {
    if !arguments.revoke {
        let _guard = context
            .credential_lock
            .acquire()
            .await
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        clear_stored_credential(context).await?;
        return print_logged_out();
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
    print_logged_out()
}

fn print_logged_out() -> Result<(), CliError> {
    let capabilities = TerminalCapabilities::detect();
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        print_lines([renderer.success("Logged out")])
    } else {
        print_lines(["Logged out.".to_owned()])
    }
}

#[derive(Debug)]
struct LoginResult {
    principal: AccountResponse,
    storage: CredentialWriteResult,
    replaced_session: bool,
    session_expires_at: Option<String>,
}

async fn persist_login_credential(
    context: &AppContext,
    credential: &StoredCredential,
    insecure: bool,
) -> Result<(CredentialWriteResult, bool), CliError> {
    let _guard = context
        .credential_lock
        .acquire()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let previous = context.stored_credential().await?;
    let storage = context
        .credentials
        .write(credential, insecure)
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    if clear_legacy_config_token(context).await.is_err() {
        eprintln!("{LEGACY_CONFIG_CLEANUP_WARNING}");
    }
    Ok((
        storage,
        matches!(previous, Some(StoredCredential::CliSession { .. })),
    ))
}

async fn refresh_credential_locked(
    context: &AppContext,
    credential: StoredCredential,
) -> Result<String, CliError> {
    let StoredCredential::CliSession {
        session_id,
        refresh_token,
        scopes,
        ..
    } = credential
    else {
        return Err(CliError::Infrastructure(
            "attempted to refresh a non-session credential".into(),
        ));
    };
    let request = RefreshTokenRequest {
        grant_type: RefreshGrantType::RefreshToken,
        refresh_token,
    };
    let refreshed: RefreshTokenResponse = match context
        .api
        .request_json(
            Method::POST,
            "/v1/auth/token/refresh",
            None,
            Some(
                serde_json::to_value(request)
                    .map_err(|error| CliError::Infrastructure(error.to_string()))?,
            ),
        )
        .await
    {
        Ok(value) => value,
        Err(ApiClientError::Api { error, .. }) => {
            if refresh_failure_invalidates_local_credential(error.error.code)
                && clear_stored_credential(context).await.is_err()
            {
                eprintln!("{EXPIRED_CREDENTIAL_CLEANUP_WARNING}");
            }
            return Err(CliError::Api(error));
        }
        Err(error) => return Err(CliError::from_api_client(error)),
    };

    let now = context.now();
    let updated = StoredCredential::CliSession {
        session_id,
        access_token: refreshed.access_token.clone(),
        refresh_token: refreshed.refresh_token,
        access_token_expires_at: add_seconds_iso(now, refreshed.expires_in),
        refresh_token_expires_at: add_seconds_iso(now, refreshed.refresh_token_expires_in),
        scopes,
    };
    let storage = match context.credentials.replace(&updated).await {
        Ok(storage) => storage,
        Err(error) => {
            revoke_cli_session_best_effort(
                context,
                &refreshed.access_token,
                ROTATED_SESSION_CLEANUP_WARNING,
            )
            .await;
            if clear_stored_credential(context).await.is_err() {
                eprintln!("{ROTATED_CREDENTIAL_CLEANUP_WARNING}");
            }
            return Err(CliError::Infrastructure(error.to_string()));
        }
    };
    if let Some(warning) = refresh_storage_warning(&storage) {
        eprintln!("{warning}");
    }
    Ok(refreshed.access_token)
}

fn refresh_storage_warning(storage: &CredentialWriteResult) -> Option<String> {
    let CredentialWriteResult::Plaintext { path, .. } = storage else {
        return None;
    };
    Some(format!(
        "The OS credential store was unavailable; refreshed credentials were stored in plaintext at {}.",
        path.display()
    ))
}

fn refresh_failure_invalidates_local_credential(code: RestErrorCode) -> bool {
    matches!(
        code,
        RestErrorCode::RefreshTokenExpired
            | RestErrorCode::RefreshTokenRevoked
            | RestErrorCode::RefreshTokenReuseDetected
            | RestErrorCode::SessionExpired
            | RestErrorCode::SessionRevoked
            | RestErrorCode::AccountDisabled
    )
}

async fn validate_token_without_refresh(
    context: &AppContext,
    token: &str,
) -> Result<AccountResponse, CliError> {
    let headers = authorization_headers(Some(token.to_owned()))?;
    context
        .api
        .request_json(Method::GET, "/v1/account", Some(headers), None)
        .await
        .map_err(CliError::from_api_client)
}

async fn revoke_cli_session_best_effort(context: &AppContext, access_token: &str, warning: &str) {
    let Ok(headers) = authorization_headers(Some(access_token.to_owned())) else {
        eprintln!("{warning}");
        return;
    };
    let result: Result<Value, ApiClientError> = context
        .api
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

#[derive(Debug)]
struct LogoutRevokeTarget {
    path: String,
    method: Method,
    token: String,
    is_cli_session: bool,
    allow_access_token_refresh: bool,
    session_id: Option<String>,
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

async fn revoke_logout_target(
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
        .api
        .request_json(method, path, Some(headers), None)
        .await
}

fn invalid_json_response() -> RestErrorResponse {
    RestErrorResponse {
        error: RestErrorDetail {
            code: RestErrorCode::InternalError,
            message: "Barestash API returned a response that was not valid JSON.".into(),
        },
    }
}

fn logout_error_confirms_revocation(code: RestErrorCode, is_cli_session: bool) -> bool {
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

async fn clear_stored_credential(context: &AppContext) -> Result<(), CliError> {
    clear_legacy_config_token(context).await?;
    context
        .credentials
        .delete()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))
}

async fn clear_legacy_config_token(context: &AppContext) -> Result<(), CliError> {
    let mut config = context
        .config
        .read()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    if config.token.take().is_none() {
        return Ok(());
    }
    context
        .config
        .write(&config)
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))
}

fn credentials_equal(left: Option<&StoredCredential>, right: Option<&StoredCredential>) -> bool {
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

fn access_token_is_fresh(credential: &StoredCredential, now: DateTime<Utc>) -> bool {
    let StoredCredential::CliSession {
        access_token_expires_at,
        ..
    } = credential
    else {
        return true;
    };
    DateTime::parse_from_rfc3339(access_token_expires_at)
        .map(|expires| expires.with_timezone(&Utc) - now > REFRESH_WINDOW)
        .unwrap_or(false)
}

fn authorization_headers(token: Option<String>) -> Result<HeaderMap, CliError> {
    let mut headers = HeaderMap::new();
    if let Some(token) = token {
        insert_bearer(&mut headers, &token)?;
    }
    Ok(headers)
}

fn insert_bearer(headers: &mut HeaderMap, token: &str) -> Result<(), CliError> {
    let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
        CliError::Local("The authentication token is not valid for an HTTP header.".into())
    })?;
    headers.insert(AUTHORIZATION, value);
    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn add_seconds_iso(now: DateTime<Utc>, seconds: u64) -> String {
    now.checked_add_signed(TimeDelta::seconds(
        i64::try_from(seconds).unwrap_or(i64::MAX),
    ))
    .unwrap_or(DateTime::<Utc>::MAX_UTC)
    .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn device_name(context: &AppContext) -> String {
    hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            context
                .env
                .get("HOSTNAME")
                .or_else(|| context.env.get("COMPUTERNAME"))
                .filter(|value| !value.is_empty())
                .cloned()
        })
        .unwrap_or_else(|| "barestash-cli".into())
}

fn report_storage(storage: &CredentialWriteResult) {
    if let CredentialWriteResult::Plaintext { path, fallback } = storage {
        if *fallback {
            eprintln!(
                "The OS credential store was unavailable; falling back to plaintext credential storage."
            );
        } else {
            eprintln!(
                "Using plaintext credential storage because --insecure-storage was specified."
            );
        }
        eprintln!("Credential file: {}", path.display());
    }
}

fn print_login_result(result: &LoginResult) -> Result<(), CliError> {
    let identity = result
        .principal
        .account
        .primary_email
        .as_deref()
        .unwrap_or(&result.principal.account.id);
    let id = match &result.principal.credential {
        AccountCredential::CliAccessToken { session_id, .. } => session_id,
        AccountCredential::PersonalAccessToken { id, .. } => id,
    };
    let capabilities = TerminalCapabilities::detect();
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.success(&format!("Authenticated as {identity}")),
            String::new(),
        ];
        lines.extend(
            renderer.details([
                ("Credential", id.clone()),
                (
                    "Session expires",
                    result
                        .session_expires_at
                        .as_deref()
                        .unwrap_or("never")
                        .to_owned(),
                ),
            ]),
        );
        print_lines(lines)
    } else {
        let mut lines = vec![format!("Authenticated as {identity} ({id})")];
        if let Some(expires_at) = &result.session_expires_at {
            lines.push(format!("Session expires: {expires_at}"));
        }
        print_lines(lines)
    }
}

fn print_status_human(
    principal: &AccountResponse,
    default_endpoint: Option<&str>,
) -> Result<(), CliError> {
    let identity = principal
        .account
        .primary_email
        .as_deref()
        .unwrap_or(&principal.account.id);
    let (kind, id, scopes, expires) = match &principal.credential {
        AccountCredential::CliAccessToken {
            id,
            scopes,
            expires_at,
            ..
        } => ("cli_access_token", id, scopes, Some(expires_at.as_str())),
        AccountCredential::PersonalAccessToken {
            id,
            scopes,
            expires_at,
        } => ("personal_access_token", id, scopes, expires_at.as_deref()),
    };
    let scope_text = scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    let capabilities = TerminalCapabilities::detect();
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.heading("Authentication", Some(identity)),
            String::new(),
        ];
        lines.extend(renderer.details([
            ("Credential", format!("{kind} ({id})")),
            ("Scopes", scope_text),
            ("Expires", expires.unwrap_or("never").to_owned()),
            (
                "Default endpoint",
                default_endpoint.unwrap_or("none").to_owned(),
            ),
        ]));
        print_lines(lines)
    } else {
        print_lines([
            format!("Authenticated as {identity}"),
            format!("Credential: {kind} ({id})"),
            format!("Scopes: {scope_text}"),
            format!("Expires: {}", expires.unwrap_or("never")),
            format!("Default endpoint: {}", default_endpoint.unwrap_or("none")),
        ])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::infrastructure::api::{ApiClient, ApiUrlPolicy};
    use crate::infrastructure::config::FileConfigStore;
    use crate::infrastructure::credentials::{
        CredentialStore, KeyringBackend, KeyringBackendError,
    };
    use crate::infrastructure::lock::FileLock;
    use crate::protocol::AuthorizationScope;

    use super::*;

    fn session(session_id: &str, access_token: &str) -> StoredCredential {
        StoredCredential::CliSession {
            session_id: session_id.into(),
            access_token: access_token.into(),
            refresh_token: "refresh".into(),
            access_token_expires_at: "2026-08-13T01:00:00.000Z".into(),
            refresh_token_expires_at: "2026-11-13T00:00:00.000Z".into(),
            scopes: vec![AuthorizationScope::EventsRead.to_string()],
        }
    }

    #[derive(Default)]
    struct TestKeyring {
        value: Mutex<Option<String>>,
    }

    impl KeyringBackend for TestKeyring {
        fn get_password(
            &self,
            _service: &str,
            _account: &str,
        ) -> Result<Option<String>, KeyringBackendError> {
            self.value
                .lock()
                .map(|value| value.clone())
                .map_err(|error| KeyringBackendError::new(error.to_string()))
        }

        fn set_password(
            &self,
            _service: &str,
            _account: &str,
            password: &str,
        ) -> Result<(), KeyringBackendError> {
            *self
                .value
                .lock()
                .map_err(|error| KeyringBackendError::new(error.to_string()))? =
                Some(password.to_owned());
            Ok(())
        }

        fn delete_password(
            &self,
            _service: &str,
            _account: &str,
        ) -> Result<bool, KeyringBackendError> {
            Ok(self
                .value
                .lock()
                .map_err(|error| KeyringBackendError::new(error.to_string()))?
                .take()
                .is_some())
        }
    }

    fn test_context(
        server: &MockServer,
        credential: Option<&StoredCredential>,
        environment_token: Option<&str>,
    ) -> (tempfile::TempDir, AppContext) {
        let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let keyring = Arc::new(TestKeyring::default());
        if let Some(credential) = credential {
            *keyring
                .value
                .lock()
                .unwrap_or_else(|error| panic!("keyring lock: {error}")) = Some(
                serde_json::to_string(credential)
                    .unwrap_or_else(|error| panic!("credential serialization: {error}")),
            );
        }
        let credentials = CredentialStore::new(
            Arc::clone(&keyring) as Arc<dyn KeyringBackend>,
            directory.path().join("credentials.json"),
        );
        let mut env = HashMap::new();
        if let Some(token) = environment_token {
            env.insert("BARESTASH_TOKEN".into(), token.into());
        }
        let context = AppContext {
            env,
            api: ApiClient::new(&server.uri(), ApiUrlPolicy::default())
                .unwrap_or_else(|error| panic!("API client: {error}")),
            config: FileConfigStore::new(directory.path().join("config.toml")),
            credentials: Arc::new(credentials),
            credential_lock: FileLock::new(directory.path().join("credentials.lock")),
        };
        (directory, context)
    }

    #[test]
    fn rotated_credentials_for_the_same_session_compare_equal() {
        let before = session("cls_same", "old");
        let after = session("cls_same", "new");
        let replacement = session("cls_other", "new");
        assert!(credentials_equal(Some(&before), Some(&after)));
        assert!(!credentials_equal(Some(&before), Some(&replacement)));
    }

    #[test]
    fn refresh_window_is_five_minutes_and_invalid_dates_refresh() {
        let now = DateTime::parse_from_rfc3339("2026-08-13T00:00:00Z")
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        let mut credential = session("cls_test", "access");
        if let StoredCredential::CliSession {
            access_token_expires_at,
            ..
        } = &mut credential
        {
            *access_token_expires_at = "2026-08-13T00:05:01Z".into();
        }
        assert!(access_token_is_fresh(&credential, now));
        if let StoredCredential::CliSession {
            access_token_expires_at,
            ..
        } = &mut credential
        {
            *access_token_expires_at = "2026-08-13T00:05:00Z".into();
        }
        assert!(!access_token_is_fresh(&credential, now));
        if let StoredCredential::CliSession {
            access_token_expires_at,
            ..
        } = &mut credential
        {
            *access_token_expires_at = "invalid".into();
        }
        assert!(!access_token_is_fresh(&credential, now));
    }

    #[test]
    fn logout_idempotency_codes_depend_on_credential_type() {
        assert!(logout_error_confirms_revocation(
            RestErrorCode::SessionExpired,
            true
        ));
        assert!(logout_error_confirms_revocation(
            RestErrorCode::PersonalAccessTokenExpired,
            false
        ));
        assert!(!logout_error_confirms_revocation(
            RestErrorCode::PersonalAccessTokenExpired,
            true
        ));
    }

    #[test]
    fn authorization_header_does_not_expose_invalid_token_in_errors() {
        let error = authorization_headers(Some("secret\nvalue".into()))
            .expect_err("invalid header should fail")
            .to_string();
        assert!(!error.contains("secret"));
    }

    #[test]
    fn plaintext_refresh_warnings_include_the_path_for_both_storage_reasons() {
        for fallback in [false, true] {
            let storage = CredentialWriteResult::Plaintext {
                path: "/safe/barestash/credentials.json".into(),
                fallback,
            };
            let warning = refresh_storage_warning(&storage)
                .unwrap_or_else(|| panic!("plaintext storage must produce a warning"));
            assert!(warning.contains("/safe/barestash/credentials.json"));
            assert!(warning.contains("plaintext"));
        }
        assert!(refresh_storage_warning(&CredentialWriteResult::Keyring).is_none());
    }

    #[tokio::test]
    async fn environment_token_wins_without_reading_or_refreshing_the_session() {
        let server = MockServer::start().await;
        let stored = session("cls_stored", "stored-access");
        let (_directory, context) = test_context(&server, Some(&stored), Some("environment-pat"));

        let resolved = resolve_auth_token(&context)
            .await
            .unwrap_or_else(|error| panic!("token resolution: {error}"));
        assert_eq!(resolved, Some("environment-pat".into()));
        assert!(
            server
                .received_requests()
                .await
                .unwrap_or_else(|| panic!("mock server did not record requests"))
                .is_empty()
        );
    }

    #[tokio::test]
    async fn concurrent_proactive_refresh_rotates_only_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/token/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token_expires_in": 7_776_000,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut expired = session("cls_same", "old-access");
        if let StoredCredential::CliSession {
            access_token_expires_at,
            ..
        } = &mut expired
        {
            *access_token_expires_at = "2000-01-01T00:00:00.000Z".into();
        }
        let (_directory, context) = test_context(&server, Some(&expired), None);
        let context = Arc::new(context);

        let (first, second) = tokio::join!(
            resolve_auth_token(context.as_ref()),
            resolve_auth_token(context.as_ref())
        );
        assert_eq!(
            first.unwrap_or_else(|error| panic!("first token resolution: {error}")),
            Some("new-access".into())
        );
        assert_eq!(
            second.unwrap_or_else(|error| panic!("second token resolution: {error}")),
            Some("new-access".into())
        );
        let stored = context
            .credentials
            .read()
            .await
            .unwrap_or_else(|error| panic!("credential read: {error}"));
        assert!(matches!(
            stored,
            Some(StoredCredential::CliSession {
                access_token,
                refresh_token,
                ..
            }) if access_token == "new-access" && refresh_token == "new-refresh"
        ));
    }

    #[tokio::test]
    async fn access_token_expired_refreshes_and_retries_once() {
        let server = MockServer::start().await;
        let account_calls = Arc::new(AtomicUsize::new(0));
        let responder_calls = Arc::clone(&account_calls);
        Mock::given(method("GET"))
            .and(path("/v1/account"))
            .respond_with(move |_request: &wiremock::Request| {
                if responder_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(401).set_body_json(json!({
                        "error": {
                            "code": "access_token_expired",
                            "message": "The access token has expired."
                        }
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "account": { "id": "acc_test", "primary_email": null },
                        "credential": {
                            "type": "cli_access_token",
                            "id": "atk_test",
                            "session_id": "cls_same",
                            "scopes": ["events:read"],
                            "expires_at": "2099-01-01T01:00:00.000Z"
                        }
                    }))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/token/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token_expires_in": 7_776_000,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut stored = session("cls_same", "old-access");
        if let StoredCredential::CliSession {
            access_token_expires_at,
            ..
        } = &mut stored
        {
            *access_token_expires_at = "2099-01-01T00:30:00.000Z".into();
        }
        let (_directory, context) = test_context(&server, Some(&stored), None);

        let account: AccountResponse = authenticated_request_json(
            &context,
            Method::GET,
            "/v1/account",
            HeaderMap::new(),
            None,
            AuthMode::Required,
        )
        .await
        .unwrap_or_else(|error| panic!("authenticated request: {error}"));
        assert_eq!(account.account.id, "acc_test");
        assert_eq!(account_calls.load(Ordering::SeqCst), 2);

        let requests = server
            .received_requests()
            .await
            .unwrap_or_else(|| panic!("mock server did not record requests"));
        let account_authorizations = requests
            .iter()
            .filter(|request| request.url.path() == "/v1/account")
            .filter_map(|request| request.headers.get("authorization"))
            .filter_map(|values| values.to_str().ok())
            .collect::<Vec<_>>();
        assert_eq!(
            account_authorizations,
            vec!["Bearer old-access", "Bearer new-access"]
        );
    }

    #[tokio::test]
    async fn access_token_expired_code_without_http_401_does_not_refresh() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/account"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {
                    "code": "access_token_expired",
                    "message": "A non-authentication failure reused the code."
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/token/refresh"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let mut stored = session("cls_same", "old-access");
        if let StoredCredential::CliSession {
            access_token_expires_at,
            ..
        } = &mut stored
        {
            *access_token_expires_at = "2099-01-01T00:30:00.000Z".into();
        }
        let (_directory, context) = test_context(&server, Some(&stored), None);

        let error = authenticated_request_json::<AccountResponse>(
            &context,
            Method::GET,
            "/v1/account",
            HeaderMap::new(),
            None,
            AuthMode::Required,
        )
        .await
        .expect_err("HTTP 400 must not trigger authentication refresh");
        assert!(matches!(
            error,
            CliError::Api(response)
                if response.error.code == RestErrorCode::AccessTokenExpired
        ));
    }

    #[tokio::test]
    async fn logout_refresh_ignores_environment_token_for_the_snapshotted_session() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/token/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "rotated-access",
                "refresh_token": "rotated-refresh",
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token_expires_in": 7_776_000,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let stored = session("cls_snapshot", "expired-access");
        let (_directory, context) = test_context(&server, Some(&stored), Some("environment-token"));

        let refreshed = refresh_stored_session_after_access_token_expired(
            &context,
            "expired-access",
            Some("cls_snapshot"),
            true,
        )
        .await
        .unwrap_or_else(|error| panic!("logout refresh: {error}"));

        assert_eq!(refreshed.as_deref(), Some("rotated-access"));
    }

    #[tokio::test]
    async fn logout_refresh_never_switches_to_a_concurrent_login_session() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/token/refresh"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let replacement = session("cls_replacement", "replacement-access");
        let (_directory, context) = test_context(&server, Some(&replacement), None);

        let refreshed = refresh_stored_session_after_access_token_expired(
            &context,
            "expired-snapshot-access",
            Some("cls_snapshot"),
            true,
        )
        .await
        .unwrap_or_else(|error| panic!("logout refresh: {error}"));

        assert_eq!(refreshed, None);
        let stored = context
            .stored_credential()
            .await
            .unwrap_or_else(|error| panic!("credential read: {error}"));
        assert!(matches!(
            stored,
            Some(StoredCredential::CliSession {
                session_id,
                access_token,
                ..
            }) if session_id == "cls_replacement" && access_token == "replacement-access"
        ));
    }

    #[tokio::test]
    async fn logout_revoke_does_not_send_a_concurrent_sessions_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/sessions/current/revoke"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": {
                    "code": "access_token_expired",
                    "message": "The snapshot access token expired."
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/token/refresh"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let replacement = session("cls_replacement", "replacement-access");
        let (_directory, context) = test_context(&server, Some(&replacement), None);
        let target = LogoutRevokeTarget {
            path: "/v1/auth/sessions/current/revoke".into(),
            method: Method::POST,
            token: "expired-snapshot-access".into(),
            is_cli_session: true,
            allow_access_token_refresh: true,
            session_id: Some("cls_snapshot".into()),
        };

        let error = revoke_logout_target(&context, &target)
            .await
            .expect_err("the replacement session must not be revoked");
        assert!(matches!(
            error,
            CliError::Api(response)
                if response.error.code == RestErrorCode::AccessTokenExpired
        ));
    }

    #[tokio::test]
    async fn public_read_retries_anonymously_when_reactive_refresh_is_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/account"))
            .respond_with(|request: &wiremock::Request| {
                if request.headers.contains_key("authorization") {
                    ResponseTemplate::new(401).set_body_json(json!({
                        "error": {
                            "code": "access_token_expired",
                            "message": "The access token expired."
                        }
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "account": { "id": "acc_public", "primary_email": null },
                        "credential": {
                            "type": "personal_access_token",
                            "id": "tok_public",
                            "scopes": ["events:read"],
                            "expires_at": null
                        }
                    }))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/token/refresh"))
            .respond_with(ResponseTemplate::new(503).set_body_json(json!({
                "error": { "code": "internal_error", "message": "Unavailable." }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut stored = session("cls_same", "old-access");
        if let StoredCredential::CliSession {
            access_token_expires_at,
            ..
        } = &mut stored
        {
            *access_token_expires_at = "2099-01-01T00:30:00.000Z".into();
        }
        let (_directory, context) = test_context(&server, Some(&stored), None);

        let account: AccountResponse = authenticated_request_json(
            &context,
            Method::GET,
            "/v1/account",
            HeaderMap::new(),
            None,
            AuthMode::PublicRead,
        )
        .await
        .unwrap_or_else(|error| panic!("public read: {error}"));
        assert_eq!(account.account.id, "acc_public");
    }
}
