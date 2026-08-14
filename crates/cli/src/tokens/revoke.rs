use reqwest::Method;
use reqwest::header::HeaderMap;

use barestash_local_state::StoredCredential;
use barestash_protocol::{
    BearerTokenType, PersonalAccessTokenRevokeResponse, parse_bearer_token_string,
    token_id_from_bearer_token_string,
};

use super::{TokenRevokeArgs, print_token_diagnostic, print_token_revoked};
use crate::auth::{AuthMode, authenticated_request_json};
use crate::output::sanitize_terminal_text;
use crate::platform::terminal::confirm;
use crate::{AppContext, CliError};

pub(super) async fn execute(context: &AppContext, args: TokenRevokeArgs) -> Result<(), CliError> {
    if current_personal_access_token_id(context).await?.as_deref() == Some(args.token_id.as_str()) {
        print_token_diagnostic("Warning: this token is currently used by the CLI.")?;
    }
    if !args.yes
        && !confirm(&format!(
            "Revoke token {}?",
            sanitize_terminal_text(&args.token_id)
        ))?
    {
        return Err(CliError::Local("Token revocation cancelled.".into()));
    }

    let response: PersonalAccessTokenRevokeResponse = authenticated_request_json(
        context,
        Method::DELETE,
        &format!("/v1/tokens/{}", args.token_id),
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;
    print_token_revoked(&response)?;
    Ok(())
}

async fn current_personal_access_token_id(
    context: &AppContext,
) -> Result<Option<String>, CliError> {
    if let Some(token) = context.environment_token() {
        return Ok(personal_access_token_id(token));
    }
    let token = match context.stored_credential().await? {
        Some(StoredCredential::PersonalAccessToken { token }) => Some(token),
        Some(StoredCredential::CliSession { .. }) | None => None,
    };
    Ok(token.and_then(|token| personal_access_token_id(&token)))
}

fn personal_access_token_id(token: &str) -> Option<String> {
    let parsed = parse_bearer_token_string(token)?;
    if parsed.token_type == BearerTokenType::Pat {
        token_id_from_bearer_token_string(token)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_access_token_detection_rejects_session_tokens() {
        let suffix = "A".repeat(barestash_protocol::TOKEN_ID_SUFFIX_LENGTH);
        let secret = "B".repeat(barestash_protocol::BEARER_TOKEN_SECRET_LENGTH);
        assert_eq!(
            personal_access_token_id(&format!("bst_pat_{suffix}_{secret}")),
            Some(format!("tok_{suffix}"))
        );
        assert_eq!(
            personal_access_token_id(&format!("bst_access_{suffix}_{secret}")),
            None
        );
    }
}
