use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt as _;
use reqwest::Method;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap as HttpHeaderMap, HeaderValue,
};
use serde::Serialize;

use crate::application::auth::{
    AuthMode, auth_headers, authenticated_request_json, authenticated_send,
    refresh_after_access_token_expired,
};
use crate::application::{AppContext, CliError};
use crate::cli::events::{
    EventAction, EventListArgs, EventShowArgs, EventStreamArgs, EventTailArgs, EventsCommand,
};
use crate::domain::{
    TransformedBody, TransformedEventStreamPayload, parse_poll_interval,
    redact_headers_for_display, transform_body, transform_stream_payload,
};
use crate::infrastructure::api::{ApiClient, ApiClientError};
use crate::infrastructure::sse::{SseDecoder, SseEvent};
use crate::presentation::renderer::{TableColumn, Tone};
use crate::presentation::{
    OutputRenderer, TailView, TerminalCapabilities, print_json, print_json_line, print_lines,
};
use crate::protocol::{
    EventDetail, EventListResponse, EventMetadata, EventStreamPayload, RestErrorCode,
};

pub const STREAM_RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventWithBody {
    pub event: EventDetail,
    pub body: TransformedBody,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatestEventWithBody {
    pub event: Option<EventDetail>,
    pub body: Option<TransformedBody>,
}

pub async fn list_events(
    context: &AppContext,
    endpoint_id: &str,
    limit: Option<&str>,
    after: Option<&str>,
) -> Result<Vec<EventMetadata>, CliError> {
    let path = event_list_path(endpoint_id, limit, after);
    let response: EventListResponse = authenticated_request_json(
        context,
        Method::GET,
        &path,
        HttpHeaderMap::new(),
        None,
        AuthMode::PublicRead,
    )
    .await?;
    Ok(response.events)
}

#[cfg(test)]
async fn list_events_with_headers(
    api: &ApiClient,
    endpoint_id: &str,
    limit: Option<&str>,
    after: Option<&str>,
    headers: HttpHeaderMap,
) -> Result<Vec<EventMetadata>, CliError> {
    let path = event_list_path(endpoint_id, limit, after);
    let response: EventListResponse = api
        .request_json(Method::GET, &path, Some(headers), None)
        .await
        .map_err(map_api_error)?;
    Ok(response.events)
}

fn event_list_path(endpoint_id: &str, limit: Option<&str>, after: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(limit) = limit {
        query.append_pair("limit", limit);
    }
    if let Some(after) = after {
        query.append_pair("after", after);
    }
    let query = query.finish();
    let suffix = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    format!("/v1/endpoints/{endpoint_id}/events{suffix}")
}

pub async fn fetch_event_detail(
    context: &AppContext,
    event_id: &str,
) -> Result<EventDetail, CliError> {
    authenticated_request_json(
        context,
        Method::GET,
        &format!("/v1/events/{event_id}"),
        HttpHeaderMap::new(),
        None,
        AuthMode::PublicRead,
    )
    .await
}

pub async fn fetch_event_body(
    context: &AppContext,
    event: &EventDetail,
) -> Result<TransformedBody, CliError> {
    let response = authenticated_send(
        context,
        Method::GET,
        &format!("/v1/events/{}/body", event.id),
        HttpHeaderMap::new(),
        None,
        AuthMode::PublicRead,
    )
    .await?;
    transform_event_body_response(response, event).await
}

#[cfg(test)]
async fn fetch_event_body_with_headers(
    api: &ApiClient,
    event: &EventDetail,
    headers: HttpHeaderMap,
) -> Result<TransformedBody, CliError> {
    let response = api
        .send(
            Method::GET,
            &format!("/v1/events/{}/body", event.id),
            |builder| builder.headers(headers),
        )
        .await
        .map_err(map_api_error)?;

    transform_event_body_response(response, event).await
}

async fn transform_event_body_response(
    response: reqwest::Response,
    event: &EventDetail,
) -> Result<TransformedBody, CliError> {
    if !response.status().is_success() {
        return match ApiClient::decode_json::<serde_json::Value>(response).await {
            Err(error) => Err(map_api_error(error)),
            Ok(_) => Err(CliError::Connectivity(
                "Barestash API returned an unexpected response status.".into(),
            )),
        };
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| event.request.headers.get("content-type").cloned())
        .unwrap_or_else(|| "-".to_owned());
    let bytes = response.bytes().await.map_err(|error| {
        CliError::Connectivity(format!(
            "failed to read the Barestash API response: {error}"
        ))
    })?;

    Ok(transform_body(&bytes, &content_type))
}

pub async fn show_event(context: &AppContext, event_id: &str) -> Result<EventWithBody, CliError> {
    let event = fetch_event_detail(context, event_id).await?;
    let body = fetch_event_body(context, &event).await?;
    Ok(EventWithBody { event, body })
}

pub async fn show_latest_event(
    context: &AppContext,
    endpoint_id: &str,
) -> Result<LatestEventWithBody, CliError> {
    let Some(event) = list_events(context, endpoint_id, Some("1"), None)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(LatestEventWithBody {
            event: None,
            body: None,
        });
    };
    let shown = show_event(context, &event.id).await?;
    Ok(LatestEventWithBody {
        event: Some(shown.event),
        body: Some(shown.body),
    })
}

#[derive(Debug, Clone, Copy)]
pub struct TailOptions {
    pub last: u64,
    pub poll_interval: Duration,
    /// `None` for the production infinite tail; finite values support tests and
    /// embedded agent callers without changing polling semantics.
    pub max_polls: Option<usize>,
}

impl TailOptions {
    pub const fn watching(last: u64, poll_interval: Duration) -> Self {
        Self {
            last,
            poll_interval,
            max_polls: None,
        }
    }
}

pub async fn tail_events<Ready, Event, EventFuture>(
    context: &AppContext,
    endpoint_id: &str,
    options: TailOptions,
    mut on_ready: Ready,
    mut on_event: Event,
) -> Result<(), CliError>
where
    Ready: FnMut() -> Result<(), CliError>,
    Event: FnMut(EventMetadata) -> EventFuture,
    EventFuture: Future<Output = Result<(), CliError>>,
{
    let mut cursor = None;

    if options.last > 0 {
        let last = options.last.to_string();
        let initial = list_events(context, endpoint_id, Some(&last), None).await?;
        on_ready()?;

        for event in initial.into_iter().rev() {
            let event_id = event.id.clone();
            on_event(event).await?;
            cursor = Some(event_id);
        }
    } else {
        cursor = list_events(context, endpoint_id, Some("1"), None)
            .await?
            .into_iter()
            .next()
            .map(|event| event.id);
        on_ready()?;
    }

    let mut polls = 0usize;
    while options.max_polls.is_none_or(|maximum| polls < maximum) {
        if polls > 0 || options.last > 0 {
            tokio::time::sleep(options.poll_interval).await;
        }

        let requested_after = cursor.clone();
        let mut events =
            list_events(context, endpoint_id, None, requested_after.as_deref()).await?;
        if requested_after.is_none() {
            events.reverse();
        }

        for event in events {
            let event_id = event.id.clone();
            on_event(event).await?;
            cursor = Some(event_id);
        }
        polls += 1;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
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

pub async fn stream_events<Payload>(
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
        let mut headers = match auth_headers(context).await {
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
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        if let Some(id) = &last_event_id {
            let value = HeaderValue::from_str(id)
                .map_err(|_| CliError::Local("SSE event ID is not a valid HTTP header.".into()))?;
            headers.insert("last-event-id", value);
        }

        let response = match send_stream_request(context, endpoint_id, headers).await {
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

        let mut decoder = SseDecoder::new(last_event_id.clone());
        let mut bytes = response.bytes_stream();
        let mut read_error = None;

        while let Some(chunk) = bytes.next().await {
            match chunk {
                Ok(chunk) => {
                    for event in decoder.push(&chunk) {
                        emit_sse_event(event, &mut on_payload)?;
                    }
                }
                Err(error) => {
                    read_error = Some(error);
                    break;
                }
            }
        }

        if let Some(error) = read_error {
            last_event_id = decoder.last_event_id().map(str::to_owned);
            if reconnect_limit_reached(reconnects, options.max_reconnects) {
                return Err(CliError::Local(format!(
                    "Failed to read Barestash event stream.\n{error}"
                )));
            }
            reconnects += 1;
            tokio::time::sleep(options.reconnect_delay).await;
            continue;
        }

        match decoder.finish() {
            Ok((events, completed_last_event_id)) => {
                for event in events {
                    emit_sse_event(event, &mut on_payload)?;
                }
                last_event_id = completed_last_event_id;
            }
            Err(error) => {
                last_event_id = error.last_event_id().map(str::to_owned);
                if reconnect_limit_reached(reconnects, options.max_reconnects) {
                    return Err(CliError::Local(format!(
                        "Failed to read Barestash event stream.\n{error}"
                    )));
                }
                reconnects += 1;
                tokio::time::sleep(options.reconnect_delay).await;
                continue;
            }
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
) -> Result<reqwest::Response, StreamRequestError> {
    let path = format!("/v1/endpoints/{endpoint_id}/events/stream");
    let expired_token = bearer_token_from_headers(&headers).map(str::to_owned);
    let response = context
        .api
        .send(Method::GET, &path, |builder| {
            builder.headers(headers.clone())
        })
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
        .api
        .send(Method::GET, &path, |builder| builder.headers(retry_headers))
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

fn map_api_error(error: ApiClientError) -> CliError {
    match error {
        ApiClientError::Api { error, .. } => CliError::Api(error),
        ApiClientError::InvalidUrl(error) => CliError::Local(error.to_string()),
        other => CliError::Connectivity(other.to_string()),
    }
}

pub async fn run(context: &AppContext, command: EventsCommand) -> Result<(), CliError> {
    match command.action {
        EventAction::List(arguments) => run_list(context, arguments).await,
        EventAction::Latest(arguments) => {
            let endpoint_id = context
                .selected_endpoint(arguments.endpoint.as_deref())
                .await?;
            let value = show_latest_event(context, &endpoint_id).await?;
            if arguments.json {
                let value = redact_latest_for_display(value);
                print_json(&value)
            } else if let Some(event) = value.event {
                print_event_detail(&event, value.body.as_ref())
            } else {
                print_lines(["No events received yet."])
            }
        }
        EventAction::Show(arguments) => run_show(context, arguments).await,
        EventAction::Tail(arguments) => run_tail(context, arguments).await,
        EventAction::Stream(arguments) => run_stream(context, arguments).await,
    }
}

async fn run_list(context: &AppContext, arguments: EventListArgs) -> Result<(), CliError> {
    let endpoint_id = context
        .selected_endpoint(arguments.endpoint.as_deref())
        .await?;
    let events = list_events(context, &endpoint_id, arguments.limit.as_deref(), None).await?;
    if arguments.json {
        #[derive(Serialize)]
        struct Output<'a> {
            events: &'a [EventMetadata],
        }
        let events = events
            .into_iter()
            .map(redact_event_metadata_for_display)
            .collect::<Vec<_>>();
        return print_json(&Output { events: &events });
    }
    print_event_list(&events)
}

async fn run_show(context: &AppContext, arguments: EventShowArgs) -> Result<(), CliError> {
    let value = show_event(context, &arguments.event_id).await?;
    if arguments.json {
        return print_json(&redact_event_with_body_for_display(value));
    }
    print_event_detail(&value.event, Some(&value.body))
}

async fn run_tail(context: &AppContext, arguments: EventTailArgs) -> Result<(), CliError> {
    let capabilities = TerminalCapabilities::detect();
    if arguments.view && !capabilities.interactive {
        return Err(CliError::Local(
            "--view requires an interactive terminal.".into(),
        ));
    }
    if arguments.view && (arguments.headers || arguments.body) {
        return Err(CliError::Local(
            "--view cannot be combined with --headers or --body.".into(),
        ));
    }
    let endpoint_id = context
        .selected_endpoint(arguments.endpoint.as_deref())
        .await?;
    let poll_interval = parse_poll_interval(&arguments.poll_interval)
        .map_err(|error| CliError::Local(error.to_string()))?;
    let poll_interval = Duration::from_millis(poll_interval);
    if std::time::Instant::now()
        .checked_add(poll_interval)
        .is_none()
    {
        return Err(CliError::Local("Poll interval is too large.".into()));
    }
    let last = parse_tail_last(&arguments.last)?;
    let view = arguments
        .view
        .then(|| Arc::new(Mutex::new(TailView::new(&endpoint_id, capabilities))));
    let ready_view = view.clone();
    let event_view = view.clone();
    let include_headers = arguments.headers;
    let include_body = arguments.body;
    let options = TailOptions::watching(last, poll_interval);
    let operation = tail_events(
        context,
        &endpoint_id,
        options,
        || {
            if let Some(view) = &ready_view {
                return view
                    .lock()
                    .map_err(|_| CliError::Infrastructure("tail view lock was poisoned".into()))?
                    .start();
            }
            let renderer = OutputRenderer::new(capabilities);
            let header = renderer.decorate(
                "RECEIVED                   METHOD PATH            SIZE CONTENT-TYPE     EVENT",
                Tone::Muted,
                true,
            );
            print_lines([
                format!("Watching endpoint: {endpoint_id}"),
                String::new(),
                header,
            ])
        },
        |event| {
            let event_view = event_view.clone();
            async move {
                if let Some(view) = event_view {
                    return view
                        .lock()
                        .map_err(|_| {
                            CliError::Infrastructure("tail view lock was poisoned".into())
                        })?
                        .add(event);
                }
                print_event_summary(&event)?;
                if include_headers || include_body {
                    let detail = fetch_event_detail(context, &event.id).await?;
                    if include_headers {
                        print_event_headers(&detail)?;
                    }
                    if include_body {
                        let body = fetch_event_body(context, &detail).await?;
                        print_event_body(&body)?;
                    }
                }
                Ok(())
            }
        },
    );

    let result = tokio::select! {
        biased;
        interrupted = tokio::signal::ctrl_c() => {
            interrupted.map_err(|error| CliError::Infrastructure(error.to_string()))?;
            Ok(())
        }
        result = operation => result,
    };
    drop(view);
    result
}

fn parse_tail_last(value: &str) -> Result<u64, CliError> {
    let value = value.trim();
    let prefixed = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|digits| u64::from_str_radix(digits, 16))
        .or_else(|| {
            value
                .strip_prefix("0b")
                .or_else(|| value.strip_prefix("0B"))
                .map(|digits| u64::from_str_radix(digits, 2))
        })
        .or_else(|| {
            value
                .strip_prefix("0o")
                .or_else(|| value.strip_prefix("0O"))
                .map(|digits| u64::from_str_radix(digits, 8))
        });
    if let Some(parsed) = prefixed {
        return parsed
            .map_err(|_| CliError::Local("--last must be a non-negative integer.".into()));
    }
    let parsed = value
        .parse::<f64>()
        .map_err(|_| CliError::Local("--last must be a non-negative integer.".into()))?;
    if !parsed.is_finite() || parsed < 0.0 || parsed.fract() != 0.0 || parsed > u64::MAX as f64 {
        return Err(CliError::Local(
            "--last must be a non-negative integer.".into(),
        ));
    }
    Ok(parsed as u64)
}

async fn run_stream(context: &AppContext, arguments: EventStreamArgs) -> Result<(), CliError> {
    let endpoint_id = context
        .selected_endpoint(arguments.endpoint.as_deref())
        .await?;
    let operation = stream_events(context, &endpoint_id, StreamOptions::default(), |payload| {
        print_json_line(&payload)
    });

    tokio::select! {
        biased;
        interrupted = tokio::signal::ctrl_c() => {
            interrupted.map_err(|error| CliError::Infrastructure(error.to_string()))?;
            Ok(())
        }
        result = operation => result,
    }
}

fn redact_event_for_display(mut event: EventDetail) -> EventDetail {
    event.request.headers = redact_headers_for_display(&event.request.headers);
    event
}

fn redact_event_metadata_for_display(mut event: EventMetadata) -> EventMetadata {
    event.headers = redact_headers_for_display(&event.headers);
    event
}

fn redact_event_with_body_for_display(mut value: EventWithBody) -> EventWithBody {
    value.event = redact_event_for_display(value.event);
    value
}

fn redact_latest_for_display(mut value: LatestEventWithBody) -> LatestEventWithBody {
    value.event = value.event.map(redact_event_for_display);
    value
}

fn print_event_list(events: &[EventMetadata]) -> Result<(), CliError> {
    if events.is_empty() {
        return print_lines(["No events received yet."]);
    }
    let capabilities = TerminalCapabilities::detect();
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let detail = format!("{} received", events.len());
        let columns = [
            TableColumn::new("ID", 12),
            TableColumn::new("METHOD", 6).tone(Tone::Method),
            TableColumn::new("PATH", 10).flexible(),
            TableColumn::new("CONTENT-TYPE", 10).flexible(),
            TableColumn::new("SIZE", 6),
            TableColumn::new("RECEIVED", 10).flexible(),
        ];
        let rows = events
            .iter()
            .map(|event| {
                vec![
                    event.id.clone(),
                    event.method.clone(),
                    event.request_path.clone(),
                    event_content_type(&event.headers).to_owned(),
                    format_bytes(event.body.size),
                    event.received_at.clone(),
                ]
            })
            .collect::<Vec<_>>();
        let mut lines = vec![renderer.heading("Events", Some(&detail)), String::new()];
        lines.extend(renderer.table(&columns, &rows));
        return print_lines(lines);
    }
    let mut lines = vec![
        "ID              METHOD  PATH              CONTENT-TYPE       SIZE    RECEIVED".to_owned(),
    ];
    lines.extend(events.iter().map(|event| {
        format!(
            "{}  {}  {}  {}  {}  {}",
            event.id,
            event.method,
            event.request_path,
            event_content_type(&event.headers),
            format_bytes(event.body.size),
            event.received_at
        )
    }));
    print_lines(lines)
}

fn print_event_summary(event: &EventMetadata) -> Result<(), CliError> {
    let renderer = OutputRenderer::new(TerminalCapabilities::detect());
    let method = renderer.decorate(&event.method, Tone::Method, true);
    print_lines([format!(
        "[{}] {} {} {} {} {}",
        event.received_at,
        method,
        event.request_path,
        format_bytes(event.body.size),
        event_content_type(&event.headers),
        event.id
    )])
}

fn print_event_detail(event: &EventDetail, body: Option<&TransformedBody>) -> Result<(), CliError> {
    let event = redact_event_for_display(event.clone());
    let capabilities = TerminalCapabilities::detect();
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.heading("Event", Some(&event.id)),
            String::new(),
            renderer.section("Request"),
        ];
        lines.extend(renderer.details([
            ("Endpoint", event.endpoint_id.clone()),
            (
                "Method",
                renderer.decorate(&event.request.method, Tone::Method, true),
            ),
            ("Path", event.request.request_path.clone()),
            ("Received", event.received_at.clone()),
            (
                "Content-Type",
                event_content_type(&event.request.headers).to_owned(),
            ),
            ("Size", format_bytes(event.request.body.size)),
        ]));
        lines.extend([String::new(), renderer.section("Headers")]);
        lines.extend(
            renderer.details(
                event
                    .request
                    .headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.clone())),
            ),
        );
        lines.extend([String::new(), renderer.section("Body")]);
        if let Some(body) = body {
            lines.extend(body_lines(body)?);
        }
        return print_lines(lines);
    }
    let mut lines = vec![
        format!("Event: {}", event.id),
        format!("Endpoint: {}", event.endpoint_id),
        String::new(),
        "Request:".to_owned(),
        format!("  Method:       {}", event.request.method),
        format!("  Path:         {}", event.request.request_path),
        format!("  Received:     {}", event.received_at),
        format!(
            "  Content-Type: {}",
            event_content_type(&event.request.headers)
        ),
        format!("  Size:         {}", format_bytes(event.request.body.size)),
        String::new(),
        "Headers:".to_owned(),
    ];
    lines.extend(
        event
            .request
            .headers
            .iter()
            .map(|(name, value)| format!("  {name}: {value}")),
    );
    lines.extend([String::new(), "Body:".to_owned()]);
    if let Some(body) = body {
        lines.extend(body_lines(body)?);
    }
    print_lines(lines)
}

fn print_event_headers(event: &EventDetail) -> Result<(), CliError> {
    let headers = redact_headers_for_display(&event.request.headers);
    let renderer = OutputRenderer::new(TerminalCapabilities::detect());
    let heading = if renderer.capabilities.interactive {
        renderer.section("Headers")
    } else {
        "Headers:".to_owned()
    };
    let mut lines = vec![String::new(), heading];
    lines.extend(
        headers
            .iter()
            .map(|(name, value)| format!("  {name}: {value}")),
    );
    print_lines(lines)
}

fn print_event_body(body: &TransformedBody) -> Result<(), CliError> {
    let renderer = OutputRenderer::new(TerminalCapabilities::detect());
    let heading = if renderer.capabilities.interactive {
        renderer.section("Body")
    } else {
        "Body:".to_owned()
    };
    let mut lines = vec![String::new(), heading];
    lines.extend(body_lines(body)?);
    print_lines(lines)
}

fn body_lines(body: &TransformedBody) -> Result<Vec<String>, CliError> {
    match body {
        TransformedBody::Metadata(metadata) => Ok(vec![format!(
            "{} ({})",
            metadata.content_type,
            format_bytes(metadata.size)
        )]),
        TransformedBody::Text(text) => Ok(text.split('\n').map(str::to_owned).collect()),
        TransformedBody::Json(value) => serde_json::to_string_pretty(value)
            .map(|value| value.lines().map(str::to_owned).collect())
            .map_err(|error| CliError::Infrastructure(error.to_string())),
    }
}

fn event_content_type(headers: &crate::protocol::HeaderMap) -> &str {
    headers.get("content-type").map_or("-", String::as_str)
}

fn format_bytes(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::infrastructure::api::ApiUrlPolicy;
    use crate::infrastructure::config::FileConfigStore;
    use crate::infrastructure::credentials::CredentialStore;
    use crate::infrastructure::lock::FileLock;
    use crate::protocol::{EventBodyMetadata, HeaderMap, QueryParameters};
    use reqwest::StatusCode;
    use tempfile::TempDir;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn api(server: &MockServer) -> ApiClient {
        ApiClient::new(
            &server.uri(),
            ApiUrlPolicy {
                allow_insecure: true,
            },
        )
        .unwrap_or_else(|error| panic!("mock API URL is valid: {error}"))
    }

    fn context(server: &MockServer) -> (AppContext, TempDir) {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("temporary directory is available: {error}"));
        let config_path = directory.path().join("config.toml");
        let context = AppContext {
            env: HashMap::from([("BARESTASH_TOKEN".to_owned(), "test-token".to_owned())]),
            api: api(server),
            config: FileConfigStore::new(&config_path),
            credentials: Arc::new(CredentialStore::system(
                directory.path().join("credentials.json"),
            )),
            credential_lock: FileLock::new(directory.path().join("credentials.lock")),
        };
        (context, directory)
    }

    fn event(id: &str) -> EventMetadata {
        EventMetadata {
            id: id.to_owned(),
            endpoint_id: "ep_test".to_owned(),
            received_at: "2026-07-05T12:04:32.000Z".to_owned(),
            method: "POST".to_owned(),
            request_path: "/webhook".to_owned(),
            query: QueryParameters::new(),
            headers: HeaderMap::from([("content-type".to_owned(), "application/json".to_owned())]),
            body: EventBodyMetadata {
                size: 2,
                sha256: "hash".to_owned(),
                available: true,
                url: None,
            },
        }
    }

    #[tokio::test]
    async fn list_builds_limit_and_after_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events"))
            .and(query_param("limit", "2"))
            .and(query_param("after", "evt_before"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [event("evt_after")]
            })))
            .mount(&server)
            .await;

        let events = list_events_with_headers(
            &api(&server),
            "ep_test",
            Some("2"),
            Some("evt_before"),
            HttpHeaderMap::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("list succeeds: {error}"));
        assert_eq!(events, vec![event("evt_after")]);
    }

    #[tokio::test]
    async fn body_uses_response_content_type_and_transforms_binary_safely() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/events/evt_test/body"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/octet-stream")
                    .set_body_bytes(vec![0, 1, 2, 255]),
            )
            .mount(&server)
            .await;
        let detail = EventDetail {
            id: "evt_test".to_owned(),
            endpoint_id: "ep_test".to_owned(),
            received_at: "2026-07-05T12:04:32.000Z".to_owned(),
            request: crate::protocol::EventDetailRequest {
                method: "POST".to_owned(),
                ingest_path: "/ep_test".to_owned(),
                request_path: "/".to_owned(),
                query: QueryParameters::new(),
                headers: HeaderMap::from([("content-type".to_owned(), "text/plain".to_owned())]),
                body: EventBodyMetadata {
                    size: 4,
                    sha256: "hash".to_owned(),
                    available: true,
                    url: None,
                },
            },
        };

        let body = fetch_event_body_with_headers(&api(&server), &detail, HttpHeaderMap::new())
            .await
            .unwrap_or_else(|error| panic!("body succeeds: {error}"));
        assert_eq!(
            body,
            TransformedBody::Metadata(crate::domain::BodyMetadata {
                content_type: "application/octet-stream".to_owned(),
                size: 4,
            })
        );
    }

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
    async fn api_errors_preserve_typed_backend_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_missing/events"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": {"code": "endpoint_not_found", "message": "Endpoint missing."}
            })))
            .mount(&server)
            .await;

        let result = list_events_with_headers(
            &api(&server),
            "ep_missing",
            None,
            None,
            HttpHeaderMap::new(),
        )
        .await;
        assert!(matches!(
            result,
            Err(CliError::Api(crate::protocol::RestErrorResponse {
                error: crate::protocol::RestErrorDetail {
                    code: RestErrorCode::EndpointNotFound,
                    ..
                }
            }))
        ));
    }

    #[tokio::test]
    async fn authenticated_stream_request_uses_last_event_id_header() {
        // Header matching is covered at the transport boundary without needing
        // to construct a keyring-backed AppContext.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events/stream"))
            .and(header("last-event-id", "evt_previous"))
            .respond_with(
                ResponseTemplate::new(StatusCode::OK.as_u16())
                    .set_body_raw("id: evt_next\ndata: {}\n\n", "text/event-stream"),
            )
            .mount(&server)
            .await;

        let mut headers = HttpHeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("evt_previous"));
        let response = api(&server)
            .send(
                Method::GET,
                "/v1/endpoints/ep_test/events/stream",
                |builder| builder.headers(headers),
            )
            .await
            .unwrap_or_else(|error| panic!("stream connects: {error}"));
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn tail_without_last_suppresses_existing_event_and_uses_after_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events"))
            .and(query_param("limit", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [event("evt_existing")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events"))
            .and(query_param("after", "evt_existing"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [event("evt_new")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (context, _directory) = context(&server);
        let output = Arc::new(Mutex::new(Vec::new()));
        let callback_output = output.clone();

        tail_events(
            &context,
            "ep_test",
            TailOptions {
                last: 0,
                poll_interval: Duration::ZERO,
                max_polls: Some(1),
            },
            || Ok(()),
            move |event| {
                callback_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(event.id);
                async { Ok(()) }
            },
        )
        .await
        .unwrap_or_else(|error| panic!("tail succeeds: {error}"));

        assert_eq!(
            *output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec!["evt_new"]
        );
    }

    #[tokio::test]
    async fn tail_last_events_are_emitted_oldest_first() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events"))
            .and(query_param("limit", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [event("evt_newer"), event("evt_older")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (context, _directory) = context(&server);
        let output = Arc::new(Mutex::new(Vec::new()));
        let callback_output = output.clone();

        tail_events(
            &context,
            "ep_test",
            TailOptions {
                last: 2,
                poll_interval: Duration::ZERO,
                max_polls: Some(0),
            },
            || Ok(()),
            move |event| {
                callback_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(event.id);
                async { Ok(()) }
            },
        )
        .await
        .unwrap_or_else(|error| panic!("tail succeeds: {error}"));

        assert_eq!(
            *output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec!["evt_older", "evt_newer"]
        );
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
