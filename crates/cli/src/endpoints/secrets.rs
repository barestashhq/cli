use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_protocol::{
    EndpointSecretCreateResponse, EndpointSecretListResponse, EndpointSecretRevokeResponse,
};

use super::{
    EndpointSecretCreateArgs, EndpointSecretListArgs, EndpointSecretRevokeArgs,
    print_secret_created, print_secret_list, print_secret_revoked,
};
use crate::auth::{AuthMode, authenticated_request_json};
use crate::output::sanitize_terminal_text;
use crate::platform::terminal::confirm;
use crate::{AppContext, CliError};

pub(super) async fn create(
    context: &AppContext,
    args: EndpointSecretCreateArgs,
) -> Result<(), CliError> {
    let endpoint_id = context.selected_endpoint(args.endpoint.as_deref()).await?;
    let response: EndpointSecretCreateResponse = authenticated_request_json(
        context,
        Method::POST,
        &format!("/v1/endpoints/{endpoint_id}/secrets"),
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;

    print_secret_created(&response, args.json)?;
    Ok(())
}

pub(super) async fn list(
    context: &AppContext,
    args: EndpointSecretListArgs,
) -> Result<(), CliError> {
    let endpoint_id = context.selected_endpoint(args.endpoint.as_deref()).await?;
    let response: EndpointSecretListResponse = authenticated_request_json(
        context,
        Method::GET,
        &format!("/v1/endpoints/{endpoint_id}/secrets"),
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;

    print_secret_list(&response, args.json)?;
    Ok(())
}

pub(super) async fn revoke(
    context: &AppContext,
    args: EndpointSecretRevokeArgs,
) -> Result<(), CliError> {
    let endpoint_id = context.selected_endpoint(args.endpoint.as_deref()).await?;
    if !args.yes
        && !confirm(&format!(
            "Revoke secret {}?",
            sanitize_terminal_text(&args.secret_id)
        ))?
    {
        return Err(CliError::Local(
            "Endpoint secret revocation cancelled.".into(),
        ));
    }

    let response: EndpointSecretRevokeResponse = authenticated_request_json(
        context,
        Method::DELETE,
        &format!("/v1/endpoints/{endpoint_id}/secrets/{}", args.secret_id),
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;
    print_secret_revoked(&response)?;
    Ok(())
}
