use reqwest::Method;
use reqwest::header::{CONTENT_TYPE, HeaderMap as HttpHeaderMap};
use serde::Serialize;

use barestash_client::ApiClient;
use barestash_domain::{TransformedBody, transform_body};
use barestash_protocol::{EventDetail, EventListResponse, EventMetadata};

use super::map_api_error;
use crate::auth::{AuthMode, authenticated_request_json, authenticated_send};
use crate::{AppContext, CliError};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct EventWithBody {
    pub event: EventDetail,
    pub body: TransformedBody,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct LatestEventWithBody {
    pub event: Option<EventDetail>,
    pub body: Option<TransformedBody>,
}

pub(super) async fn list_events(
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

pub(super) async fn fetch_event_detail(
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

pub(super) async fn fetch_event_body(
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

pub(super) async fn show_event(
    context: &AppContext,
    event_id: &str,
) -> Result<EventWithBody, CliError> {
    let event = fetch_event_detail(context, event_id).await?;
    let body = fetch_event_body(context, &event).await?;
    Ok(EventWithBody { event, body })
}

pub(super) async fn show_latest_event(
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

#[cfg(test)]
mod tests {
    use barestash_protocol::{
        EventBodyMetadata, EventDetailRequest, HeaderMap, QueryParameters, RestErrorCode,
        RestErrorDetail, RestErrorResponse,
    };
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::events::test_support::{api, event};

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
            request: EventDetailRequest {
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
            TransformedBody::Metadata(barestash_domain::BodyMetadata {
                content_type: "application/octet-stream".to_owned(),
                size: 4,
            })
        );
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
            Err(CliError::Api(RestErrorResponse {
                error: RestErrorDetail {
                    code: RestErrorCode::EndpointNotFound,
                    ..
                }
            }))
        ));
    }
}
