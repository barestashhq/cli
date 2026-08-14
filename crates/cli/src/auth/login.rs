use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use reqwest::Method;

use barestash_client::ApiClientError;
use barestash_local_state::StoredCredential;
use barestash_local_state::credentials::CredentialWriteResult;
use barestash_protocol::{
    AUTHORIZATION_SCOPES, AccountCredential, AccountResponse, DeviceAuthorizationCreateRequest,
    DeviceAuthorizationCreateResponse, DeviceTokenRequest, DeviceTokenResponse, RestErrorCode,
};

use crate::output::sanitize_terminal_text;
use crate::platform::browser::open_browser;
use crate::platform::terminal::read_stdin_to_string;
use crate::{AppContext, CliError};

use super::{
    AuthLoginArgs, AuthLoginView, add_seconds_iso, clear_legacy_config_token, print_auth_login,
    revoke_cli_session_best_effort, validate_token_without_refresh,
};

const ISSUED_SESSION_CLEANUP_WARNING: &str = "Unable to revoke the newly issued CLI session. The newly issued remote CLI session may still be active.";
const LEGACY_CONFIG_CLEANUP_WARNING: &str = "Unable to remove the legacy authentication token from the config file. The newly stored credential will still be used.";

pub(super) async fn run(
    context: &AppContext,
    arguments: AuthLoginArgs,
    client_version: &str,
) -> Result<(), CliError> {
    let result = if arguments.with_token {
        login_with_token(context, arguments.insecure_storage).await?
    } else {
        login_with_device_authorization(context, arguments.insecure_storage, client_version).await?
    };

    report_storage(&result.storage);
    if result.replaced_session {
        eprintln!(
            "Replaced a stored CLI session locally. Run `barestash auth logout --revoke` before a future login to revoke the previous session remotely."
        );
    }
    print_auth_login(AuthLoginView {
        principal: &result.principal,
        session_expires_at: result.session_expires_at.as_deref(),
    })
    .map_err(Into::into)
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
    client_version: &str,
) -> Result<LoginResult, CliError> {
    let request = DeviceAuthorizationCreateRequest {
        client_name: "barestash-cli".into(),
        client_version: client_version.into(),
        device_name: device_name(context),
        requested_scopes: AUTHORIZATION_SCOPES.to_vec(),
    };
    let authorization: DeviceAuthorizationCreateResponse = context
        .api()
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
            .api()
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
