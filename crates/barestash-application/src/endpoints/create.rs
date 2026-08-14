use reqwest::Method;
use reqwest::header::HeaderMap;
use serde::Serialize;

use barestash_presentation::print_endpoint_created;
use barestash_protocol::{EndpointCreateRequest, EndpointMode, EndpointResponse};

use crate::auth::{AuthMode, authenticated_request_json};
use crate::command::EndpointCreateArgs;
use crate::{AppContext, CliError};

pub(super) async fn execute(
    context: &AppContext,
    args: EndpointCreateArgs,
) -> Result<(), CliError> {
    if args.private && args.temporary {
        return Err(CliError::Local(
            "Choose either --private or --temporary, not both.".into(),
        ));
    }
    let mode = if args.temporary {
        EndpointMode::Temporary
    } else {
        EndpointMode::Private
    };
    let body = json_body(&EndpointCreateRequest {
        mode,
        name: args.name,
    })?;

    // Temporary endpoint creation is intentionally anonymous. Private endpoint
    // creation uses the same refresh-aware authenticated request path as the
    // remaining owner operations.
    let response: EndpointResponse = if mode == EndpointMode::Temporary {
        context
            .api()
            .request_json(Method::POST, "/v1/endpoints", None, Some(body))
            .await
            .map_err(CliError::from_api_client)?
    } else {
        authenticated_request_json(
            context,
            Method::POST,
            "/v1/endpoints",
            HeaderMap::new(),
            Some(body),
            AuthMode::Required,
        )
        .await?
    };

    if args.set_default
        && persist_default_endpoint(context, &response.endpoint.id)
            .await
            .is_err()
    {
        print_endpoint_created(&response, args.json, false)?;
        return Err(default_endpoint_partial_failure(&response.endpoint.id));
    }

    print_endpoint_created(&response, args.json, args.set_default)?;
    Ok(())
}

async fn persist_default_endpoint(context: &AppContext, endpoint_id: &str) -> Result<(), CliError> {
    let _guard = context
        .credential_lock
        .acquire()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let mut config = context
        .config
        .read()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    config.default_endpoint = Some(endpoint_id.to_owned());
    context
        .config
        .write(&config)
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))
}

fn default_endpoint_partial_failure(endpoint_id: &str) -> CliError {
    CliError::Local(format!(
        "Endpoint {endpoint_id} was created, but it could not be saved as the default endpoint.\nUse --endpoint {endpoint_id} or BARESTASH_ENDPOINT={endpoint_id} until local config is writable."
    ))
}

fn json_body(value: &impl Serialize) -> Result<serde_json::Value, CliError> {
    serde_json::to_value(value).map_err(|error| CliError::Infrastructure(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_persistence_failure_reports_recovery_command() {
        let message = default_endpoint_partial_failure("ep_created").to_string();
        assert!(message.contains("Endpoint ep_created was created"));
        assert!(message.contains("--endpoint ep_created"));
        assert!(message.contains("BARESTASH_ENDPOINT=ep_created"));
    }

    #[test]
    fn endpoint_request_omits_absent_name() {
        let body = json_body(&EndpointCreateRequest {
            mode: EndpointMode::Private,
            name: None,
        })
        .unwrap_or_default();
        assert_eq!(body, serde_json::json!({"mode": "private"}));
    }
}
