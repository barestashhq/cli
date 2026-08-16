use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use reqwest::Method;

use barestash_client::ApiClientError;
use barestash_local_state::StoredCredential;
use barestash_local_state::credentials::CredentialWriteResult;
use barestash_protocol::{
    RefreshGrantType, RefreshTokenRequest, RefreshTokenResponse, RestErrorCode,
};

use crate::{AppContext, CliError};

use super::api::revoke_cli_session_best_effort;
use super::storage::clear_stored_credential;

const REFRESH_WINDOW: TimeDelta = TimeDelta::minutes(5);
const EXPIRED_CREDENTIAL_CLEANUP_WARNING: &str = "Unable to clear the expired stored authentication credential. Run `barestash auth logout` after the credential store becomes available.";
const ROTATED_CREDENTIAL_CLEANUP_WARNING: &str = "Unable to clear the stale stored authentication credential after refresh persistence failed. Run `barestash auth logout` before authenticating again.";
const ROTATED_SESSION_CLEANUP_WARNING: &str = "Unable to revoke the rotated CLI session after refresh persistence failed. The remote CLI session may still be active.";

/// Resolves the effective bearer token in compatibility order: non-empty
/// `BARESTASH_TOKEN`, stored credential, then the legacy config token.
///
/// # Errors
///
/// Returns an error when credential/config access or session refresh fails.
pub(in crate::auth) async fn resolve_auth_token(
    context: &AppContext,
) -> Result<Option<String>, CliError> {
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

/// Refreshes a stored CLI session only when it still contains the rejected
/// access token. This compare-under-lock rule prevents concurrent requests
/// from rotating the same refresh token twice.
///
/// # Errors
///
/// Returns an error when credential access, refresh, or rotation persistence
/// fails.
pub(crate) async fn refresh_after_access_token_expired(
    context: &AppContext,
    expired_access_token: &str,
) -> Result<Option<String>, CliError> {
    refresh_stored_session_after_access_token_expired(context, expired_access_token, None, false)
        .await
}

pub(in crate::auth) async fn refresh_stored_session_after_access_token_expired(
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
        .api()
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

pub(in crate::auth) fn add_seconds_iso(now: DateTime<Utc>, seconds: u64) -> String {
    now.checked_add_signed(TimeDelta::seconds(
        i64::try_from(seconds).unwrap_or(i64::MAX),
    ))
    .unwrap_or(DateTime::<Utc>::MAX_UTC)
    .to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{DateTime, Utc};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use barestash_local_state::StoredCredential;
    use barestash_local_state::credentials::CredentialWriteResult;

    use super::super::test_support::{session, test_context};
    use super::*;

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
}
