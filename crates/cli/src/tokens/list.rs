use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_protocol::PersonalAccessTokenListResponse;

use super::{TokenListArgs, print_token_list};
use crate::auth::{AuthMode, authenticated_request_json};
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
