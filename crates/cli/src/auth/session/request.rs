use reqwest::header::{AUTHORIZATION, HeaderMap};
use reqwest::{Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;

use barestash_client::ApiClientError;
use barestash_protocol::RestErrorCode;

use crate::{AppContext, CliError};

use super::api::invalid_json_response;
use super::bearer::{authorization_headers, bearer_token, insert_bearer};
use super::credential::{refresh_after_access_token_expired, resolve_auth_token};

/// Whether an authenticated request may continue without a usable local
/// credential. Public reads deliberately degrade to an unauthenticated
/// request when session refresh is rejected or temporarily unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthMode {
    Required,
    PublicRead,
}

/// Builds an Authorization header for the currently resolved credential.
///
/// # Errors
///
/// Returns an error when token resolution, refresh, or header validation fails.
pub(crate) async fn auth_headers(context: &AppContext) -> Result<HeaderMap, CliError> {
    authorization_headers(resolve_auth_token(context).await?)
}

/// Builds optional Authorization headers for a public-read request.
///
/// # Errors
///
/// Returns persistence and local validation failures. Refresh API and network
/// failures intentionally degrade to an empty header map.
async fn public_read_auth_headers(context: &AppContext) -> Result<HeaderMap, CliError> {
    match auth_headers(context).await {
        Ok(headers) => Ok(headers),
        Err(CliError::Api(_) | CliError::Connectivity(_)) => Ok(HeaderMap::new()),
        Err(error) => Err(error),
    }
}

/// Authenticated JSON request with one reactive refresh/retry. Extra headers
/// may not override the resolved Authorization value.
///
/// # Errors
///
/// Returns a credential, transport, API, response-decoding, or refresh error.
pub(crate) async fn authenticated_request_json<T: DeserializeOwned>(
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
        .api()
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
                            .api()
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
                            .api()
                            .request_json(method, path, Some(extra_headers), body)
                            .await
                            .map_err(CliError::from_api_client);
                    }
                    Err(refresh_error) => return Err(refresh_error),
                };
            insert_bearer(&mut extra_headers, &refreshed_token)?;
            context
                .api()
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
pub(crate) async fn authenticated_send(
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
        .api()
        .send(method, path, move |mut request| {
            request = request.headers(headers);
            if let Some(body) = body {
                request = request.json(&body);
            }
            request
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use reqwest::header::HeaderMap;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use barestash_local_state::StoredCredential;
    use barestash_protocol::{AccountResponse, RestErrorCode};

    use super::super::test_support::{session, test_context};
    use super::*;

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
