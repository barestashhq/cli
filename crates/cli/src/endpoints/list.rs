use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_protocol::EndpointListResponse;

use super::{EndpointListArgs, print_endpoint_list};
use crate::auth::{AuthMode, authenticated_request_json};
use crate::{AppContext, CliError};

pub(super) async fn execute(context: &AppContext, args: EndpointListArgs) -> Result<(), CliError> {
    let response: EndpointListResponse = authenticated_request_json(
        context,
        Method::GET,
        "/v1/endpoints",
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;

    print_endpoint_list(&response, args.json)?;
    Ok(())
}
