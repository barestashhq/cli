use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_infrastructure::terminal::confirm;
use barestash_presentation::{print_endpoint_deleted, sanitize_terminal_text};
use barestash_protocol::EndpointDeleteResponse;

use crate::auth::{AuthMode, authenticated_request_json};
use crate::command::EndpointDeleteArgs;
use crate::{AppContext, CliError};

pub(super) async fn execute(
    context: &AppContext,
    args: EndpointDeleteArgs,
) -> Result<(), CliError> {
    if !args.yes
        && !confirm(&format!(
            "Delete endpoint {} and all events?",
            sanitize_terminal_text(&args.endpoint_id)
        ))?
    {
        return Err(CliError::Local("Endpoint deletion cancelled.".into()));
    }

    let response: EndpointDeleteResponse = authenticated_request_json(
        context,
        Method::DELETE,
        &format!("/v1/endpoints/{}", args.endpoint_id),
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;
    print_endpoint_deleted(&response)?;
    Ok(())
}
