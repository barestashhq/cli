use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_presentation::print_token_list;
use barestash_protocol::PersonalAccessTokenListResponse;

use crate::auth::{AuthMode, authenticated_request_json};
use crate::command::TokenListArgs;
use crate::{AppContext, CliError};

pub(super) async fn execute(context: &AppContext, args: TokenListArgs) -> Result<(), CliError> {
    let path = if args.all {
        "/v1/tokens?all=true"
    } else {
        "/v1/tokens"
    };
    let response: PersonalAccessTokenListResponse = authenticated_request_json(
        context,
        Method::GET,
        path,
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;

    print_token_list(&response, args.json)?;
    Ok(())
}
