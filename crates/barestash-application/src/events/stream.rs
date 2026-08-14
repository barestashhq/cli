use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderMap as HttpHeaderMap, HeaderValue};

use barestash_client::{ApiClient, ApiClientError, SseEvent, SseEventStream};
use barestash_domain::{TransformedEventStreamPayload, transform_stream_payload};
use barestash_protocol::{EventStreamPayload, RestErrorCode};

use super::map_api_error;
use crate::auth::{auth_headers, refresh_after_access_token_expired};
use crate::{AppContext, CliError};

const STREAM_RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
pub(super) struct StreamOptions {
    pub reconnect_delay: Duration,
    /// Maximum number of reconnect attempts after the initial connection.
    pub max_reconnects: Option<usize>,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            reconnect_delay: STREAM_RECONNECT_DELAY,
            max_reconnects: None,
        }
    }
}

pub(super) async fn stream_events<Payload>(
    context: &AppContext,
    endpoint_id: &str,
    options: StreamOptions,
    mut on_payload: Payload,
) -> Result<(), CliError>
where
    Payload: FnMut(TransformedEventStreamPayload) -> Result<(), CliError>,
{
    let mut reconnects = 0usize;
    let mut last_event_id: Option<String> = None;

    loop {
        let headers = match auth_headers(context).await {
            Ok(headers) => headers,
            Err(error @ CliError::Connectivity(_)) => {
                if reconnect_limit_reached(reconnects, options.max_reconnects) {
                    return Err(error);
                }
                reconnects += 1;
                tokio::time::sleep(options.reconnect_delay).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        let response = match send_stream_request(
            context,
            endpoint_id,
            headers,
            last_event_id.as_deref(),
        )
        .await
        {
            Ok(response) => response,
            Err(StreamRequestError::Transport(error)) => {
                if reconnect_limit_reached(reconnects, options.max_reconnects) {
                    return Err(error);
                }
                reconnects += 1;
                tokio::time::sleep(options.reconnect_delay).await;
                continue;
            }
            Err(StreamRequestError::Fatal(error)) => return Err(error),
        };

        if !response.status().is_success() {
            return Err(stream_response_error(response).await);
        }

        let mut stream = SseEventStream::new(response, last_event_id.clone());
        let read_result = loop {
            match stream.next_event().await {
                Ok(Some(event)) => emit_sse_event(event, &mut on_payload)?,
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            }
        };
        last_event_id = stream.last_event_id().map(str::to_owned);

        if let Err(error) = read_result {
            if reconnect_limit_reached(reconnects, options.max_reconnects) {
                return Err(CliError::Local(format!(
                    "Failed to read Barestash event stream.\n{error}"
                )));
            }
            reconnects += 1;
            tokio::time::sleep(options.reconnect_delay).await;
            continue;
        }

        if reconnect_limit_reached(reconnects, options.max_reconnects) {
            return Ok(());
        }
        reconnects += 1;
        tokio::time::sleep(options.reconnect_delay).await;
    }
}

enum StreamRequestError {
    Transport(CliError),
    Fatal(CliError),
}

async fn send_stream_request(
    context: &AppContext,
    endpoint_id: &str,
    headers: HttpHeaderMap,
    last_event_id: Option<&str>,
) -> Result<reqwest::Response, StreamRequestError> {
    let path = format!("/v1/endpoints/{endpoint_id}/events/stream");
    let expired_token = bearer_token_from_headers(&headers).map(str::to_owned);
    let response = context
        .api()
        .send_event_stream(&path, headers.clone(), last_event_id)
        .await
        .map_err(classify_stream_request_error)?;

    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(response);
    }
    let Some(expired_token) = expired_token else {
        return Ok(response);
    };

    let error = match ApiClient::decode_json::<serde_json::Value>(response).await {
        Err(ApiClientError::Api { error, .. }) => error,
        Err(error) => return Err(classify_stream_request_error(error)),
        Ok(_) => {
            return Err(StreamRequestError::Fatal(CliError::Connectivity(
                "Barestash API returned an unexpected response status.".into(),
            )));
        }
    };
    if error.error.code != RestErrorCode::AccessTokenExpired {
        return Err(StreamRequestError::Fatal(CliError::Api(error)));
    }

    let Some(refreshed_token) = refresh_after_access_token_expired(context, &expired_token)
        .await
        .map_err(|error| match error {
            error @ CliError::Connectivity(_) => StreamRequestError::Transport(error),
            other => StreamRequestError::Fatal(other),
        })?
    else {
        return Err(StreamRequestError::Fatal(CliError::Api(error)));
    };

    let mut retry_headers = headers;
    let authorization = HeaderValue::from_str(&format!("Bearer {refreshed_token}"))
        .map_err(|error| StreamRequestError::Fatal(CliError::Infrastructure(error.to_string())))?;
    retry_headers.insert(AUTHORIZATION, authorization);
    context
        .api()
        .send_event_stream(&path, retry_headers, last_event_id)
        .await
        .map_err(classify_stream_request_error)
}

fn classify_stream_request_error(error: ApiClientError) -> StreamRequestError {
    match error {
        // A connection failure is transient and follows the one-second
        // reconnect path. Configuration and redirect-policy errors are
        // deterministic and must fail immediately instead of looping forever.
        error @ (ApiClientError::Request(_)
        | ApiClientError::ReadResponse(_)
        | ApiClientError::ResolveHost(_)) => StreamRequestError::Transport(map_api_error(error)),
        other => StreamRequestError::Fatal(map_api_error(other)),
    }
}

fn bearer_token_from_headers(headers: &HttpHeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn emit_sse_event<Payload>(event: SseEvent, on_payload: &mut Payload) -> Result<(), CliError>
where
    Payload: FnMut(TransformedEventStreamPayload) -> Result<(), CliError>,
{
    let Some(data) = event.data else {
        return Ok(());
    };
    let payload: EventStreamPayload = serde_json::from_str(&data).map_err(|error| {
        CliError::Connectivity(format!(
            "Barestash event stream returned invalid JSON: {error}"
        ))
    })?;
    let transformed = transform_stream_payload(payload)
        .map_err(|error| CliError::Connectivity(error.to_string()))?;
    on_payload(transformed)
}

const fn reconnect_limit_reached(reconnects: usize, maximum: Option<usize>) -> bool {
    match maximum {
        Some(maximum) => reconnects >= maximum,
        None => false,
    }
}

async fn stream_response_error(response: reqwest::Response) -> CliError {
    let status = response.status();
    let error = match ApiClient::decode_json::<serde_json::Value>(response).await {
        Err(error) => error,
        Ok(_) => {
            return CliError::Connectivity(format!(
                "Barestash API returned unexpected HTTP status {status}."
            ));
        }
    };

    match error {
        ApiClientError::Api {
            error, retry_after, ..
        } if error.error.code == RestErrorCode::StreamDailyQuotaExceeded => {
            let retry = retry_after.map_or_else(String::new, |seconds| {
                format!("\n\nRetry-After: {seconds} seconds.")
            });
            CliError::Local(format!("{}{retry}", error.error.message))
        }
        ApiClientError::Api { error, .. } => CliError::Api(error),
        other => CliError::Connectivity(format!("Barestash API returned HTTP {status}: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use barestash_client::ApiUrlPolicy;
    use barestash_domain::TransformedBody;
    use barestash_infrastructure::config::FileConfigStore;
    use barestash_infrastructure::credentials::CredentialStore;
    use barestash_infrastructure::lock::FileLock;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::events::test_support::context;

    #[test]
    fn emits_only_complete_valid_sse_payloads() {
        let mut output = Vec::new();
        let payload = serde_json::json!({
            "id": "evt_test",
            "endpoint_id": "ep_test",
            "received_at": "2026-07-05T12:04:32.000Z",
            "request": {
                "method": "POST",
                "path": "/webhook",
                "query": {},
                "headers": {"content-type": "application/json"},
                "body_size": 2,
                "body_sha256": "hash"
            },
            "body": {"encoding": "base64", "data": "e30="}
        });
        emit_sse_event(
            SseEvent {
                id: Some("evt_test".to_owned()),
                data: Some(payload.to_string()),
            },
            &mut |payload| {
                output.push(payload);
                Ok(())
            },
        )
        .unwrap_or_else(|error| panic!("valid payload: {error}"));

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].id, "evt_test");
        assert_eq!(output[0].body, TransformedBody::Json(serde_json::json!({})));
    }

    #[test]
    fn invalid_sse_json_is_fatal() {
        let result = emit_sse_event(
            SseEvent {
                id: Some("evt_test".to_owned()),
                data: Some("{".to_owned()),
            },
            &mut |_| Ok(()),
        );
        assert!(matches!(result, Err(CliError::Connectivity(_))));
    }

    #[tokio::test]
    async fn clean_eof_reconnects_with_last_event_id_and_emits_jsonl_payloads() {
        let server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let responder_calls = calls.clone();
        let payload = |id: &str| {
            serde_json::json!({
                "id": id,
                "endpoint_id": "ep_test",
                "received_at": "2026-07-05T12:04:32.000Z",
                "request": {
                    "method": "POST",
                    "path": "/webhook",
                    "query": {},
                    "headers": {"content-type": "application/json"},
                    "body_size": 2,
                    "body_sha256": "hash"
                },
                "body": {"encoding": "base64", "data": "e30="}
            })
        };
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events/stream"))
            .respond_with(move |_request: &wiremock::Request| {
                let call = responder_calls.fetch_add(1, Ordering::SeqCst);
                let id = if call == 0 { "evt_first" } else { "evt_second" };
                ResponseTemplate::new(200).set_body_raw(
                    format!("id: {id}\ndata: {}\n\n", payload(id)),
                    "text/event-stream",
                )
            })
            .expect(2)
            .mount(&server)
            .await;
        let (context, _directory) = context(&server);
        let output = Arc::new(Mutex::new(Vec::new()));
        let callback_output = output.clone();

        stream_events(
            &context,
            "ep_test",
            StreamOptions {
                reconnect_delay: Duration::ZERO,
                max_reconnects: Some(1),
            },
            move |payload| {
                callback_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(payload.id);
                Ok(())
            },
        )
        .await
        .unwrap_or_else(|error| panic!("stream succeeds: {error}"));

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            *output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec!["evt_first", "evt_second"]
        );
        let requests = server.received_requests().await.unwrap_or_default();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].headers.get("last-event-id").is_none());
        assert_eq!(
            requests[1]
                .headers
                .get("last-event-id")
                .and_then(|value| value.to_str().ok()),
            Some("evt_first")
        );
    }

    #[tokio::test]
    async fn incomplete_sse_tail_keeps_the_previous_complete_id() {
        let server = MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let responder_calls = calls.clone();
        let payload = |id: &str| {
            serde_json::json!({
                "id": id,
                "endpoint_id": "ep_test",
                "received_at": "2026-07-05T12:04:32.000Z",
                "request": {
                    "method": "POST", "path": "/webhook", "query": {},
                    "headers": {"content-type": "application/json"},
                    "body_size": 2, "body_sha256": "hash"
                },
                "body": {"encoding": "base64", "data": "e30="}
            })
        };
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events/stream"))
            .respond_with(move |_request: &wiremock::Request| {
                let call = responder_calls.fetch_add(1, Ordering::SeqCst);
                let body = if call == 0 {
                    format!(
                        "id: evt_complete\ndata: {}\n\nid: evt_partial\ndata: {{",
                        payload("evt_complete")
                    )
                } else {
                    format!("id: evt_after\ndata: {}\n\n", payload("evt_after"))
                };
                ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
            })
            .expect(2)
            .mount(&server)
            .await;
        let (context, _directory) = context(&server);
        let output = Arc::new(Mutex::new(Vec::new()));
        let callback_output = output.clone();

        stream_events(
            &context,
            "ep_test",
            StreamOptions {
                reconnect_delay: Duration::ZERO,
                max_reconnects: Some(1),
            },
            move |payload| {
                callback_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(payload.id);
                Ok(())
            },
        )
        .await
        .unwrap_or_else(|error| panic!("stream reconnect succeeds: {error}"));

        assert_eq!(
            *output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec!["evt_complete", "evt_after"]
        );
        let requests = server.received_requests().await.unwrap_or_default();
        assert_eq!(
            requests[1]
                .headers
                .get("last-event-id")
                .and_then(|value| value.to_str().ok()),
            Some("evt_complete")
        );
    }

    #[tokio::test]
    async fn daily_quota_is_non_retryable_and_preserves_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events/stream"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "10800")
                    .set_body_json(serde_json::json!({
                        "error": {
                            "code": "stream_daily_quota_exceeded",
                            "message": "Daily live stream quota reached."
                        }
                    })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (context, _directory) = context(&server);

        let result = stream_events(
            &context,
            "ep_test",
            StreamOptions {
                reconnect_delay: Duration::ZERO,
                max_reconnects: None,
            },
            |_| panic!("quota rejection must not emit JSONL"),
        )
        .await;

        assert!(matches!(
            result,
            Err(CliError::Local(message))
                if message == "Daily live stream quota reached.\n\nRetry-After: 10800 seconds."
        ));
    }

    #[tokio::test]
    async fn invalid_stream_configuration_fails_without_reconnecting() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("temporary directory is available: {error}"));
        let config_path = directory.path().join("config.toml");
        let context = AppContext {
            env: HashMap::from([("BARESTASH_TOKEN".to_owned(), "test-token".to_owned())]),
            api: ApiClient::new_deferred("not a URL", ApiUrlPolicy::default())
                .unwrap_or_else(|error| panic!("deferred API client builds: {error}")),
            api_host_logged: std::sync::atomic::AtomicBool::new(true),
            config: FileConfigStore::new(&config_path),
            credentials: Arc::new(CredentialStore::system(
                directory.path().join("credentials.json"),
            )),
            credential_lock: FileLock::new(directory.path().join("credentials.lock")),
        };

        let result = tokio::time::timeout(
            Duration::from_millis(100),
            stream_events(&context, "ep_test", StreamOptions::default(), |_| Ok(())),
        )
        .await
        .unwrap_or_else(|_| panic!("deterministic URL errors must not enter the reconnect loop"));

        assert!(matches!(result, Err(CliError::Local(_))));
    }
}
