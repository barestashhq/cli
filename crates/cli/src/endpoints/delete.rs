use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_protocol::EndpointDeleteResponse;

use super::{EndpointDeleteArgs, print_endpoint_deleted};
use crate::auth::{AuthMode, authenticated_request_json};
use crate::output::sanitize_terminal_text;
use crate::platform::terminal::confirm;
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
