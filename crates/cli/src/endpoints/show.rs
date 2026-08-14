use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_protocol::EndpointResponse;

use super::{EndpointShowArgs, print_endpoint_detail};
use crate::auth::{AuthMode, authenticated_request_json};
use crate::{AppContext, CliError};

pub(super) async fn execute(context: &AppContext, args: EndpointShowArgs) -> Result<(), CliError> {
    let response: EndpointResponse = authenticated_request_json(
        context,
        Method::GET,
        &format!("/v1/endpoints/{}", args.endpoint_id),
        HeaderMap::new(),
        None,
        AuthMode::PublicRead,
    )
    .await?;

    print_endpoint_detail(&response, args.json)?;
    Ok(())
}
