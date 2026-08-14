use reqwest::Method;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use uuid::Uuid;

use barestash_presentation::{print_token_created, print_token_diagnostic};
use barestash_protocol::{
    AUTHORIZATION_SCOPES, AccountCredential, AccountResponse, AuthorizationScope,
    PersonalAccessTokenCreateRequest, PersonalAccessTokenCreateResponse,
};

use crate::auth::{AuthMode, auth_headers, authenticated_request_json};
use crate::command::{TokenCreateArgs, TokenPreset, TokenScope};
use crate::{AppContext, CliError};

const READ_ONLY_SCOPES: [AuthorizationScope; 3] = [
    AuthorizationScope::EndpointsRead,
    AuthorizationScope::EventsRead,
    AuthorizationScope::McpUse,
];

const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");

pub(super) async fn execute(context: &AppContext, args: TokenCreateArgs) -> Result<(), CliError> {
    if args.preset.is_some() && !args.scopes.is_empty() {
        return Err(CliError::Local(
            "Use either --preset or --scope, not both.".into(),
        ));
    }
    if args.no_expiration && args.expires_in.is_some() {
        return Err(CliError::Local(
            "Use either --no-expiration or --expires-in, not both.".into(),
        ));
    }
    let request = resolve_token_create_request(&args);
    if !args.json {
        print_token_diagnostic(&format!(
            "Scopes: {}",
            request
                .scopes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ))?;
        if request.expires_in == Some(None) {
            print_token_diagnostic("Warning: this token will not expire automatically.")?;
        }
    }

    // The account check is deliberately only performed when a local
    // credential exists. Anonymous requests are sent to the token endpoint so
    // the API remains the authority for the not-authenticated error.
    if auth_headers(context).await?.contains_key(AUTHORIZATION) {
        let account: AccountResponse = authenticated_request_json(
            context,
            Method::GET,
            "/v1/account",
            HeaderMap::new(),
            None,
            AuthMode::Required,
        )
        .await?;
        let allowed_scopes = credential_scopes(&account.credential);
        if let Some(scope) = request
            .scopes
            .iter()
            .find(|scope| !allowed_scopes.contains(scope))
        {
            return Err(CliError::Local(format!(
                "Requested scope {scope} is broader than the current credential allows."
            )));
        }
    }

    let key = Uuid::new_v4().to_string();
    let headers = idempotency_headers(&key)?;
    let response: PersonalAccessTokenCreateResponse = authenticated_request_json(
        context,
        Method::POST,
        "/v1/tokens",
        headers,
        Some(json_body(&request)?),
        AuthMode::Required,
    )
    .await?;

    print_token_created(&response, args.json)?;
    Ok(())
}

/// Resolves flags into the exact three-state API expiration contract and the
/// canonical, de-duplicated scope order.
#[must_use]
fn resolve_token_create_request(args: &TokenCreateArgs) -> PersonalAccessTokenCreateRequest {
    let scopes = if !args.scopes.is_empty() {
        args.scopes
            .iter()
            .copied()
            .map(scope_from_cli)
            .fold(Vec::new(), |mut scopes, scope| {
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
                scopes
            })
    } else if args.preset == Some(TokenPreset::ReadOnly) {
        READ_ONLY_SCOPES.to_vec()
    } else {
        AUTHORIZATION_SCOPES.to_vec()
    };
    let expires_in = if args.no_expiration {
        Some(None)
    } else {
        args.expires_in.map(|duration| Some(duration.as_seconds()))
    };

    PersonalAccessTokenCreateRequest {
        name: args.name.clone(),
        scopes,
        expires_in,
    }
}

fn credential_scopes(credential: &AccountCredential) -> &[AuthorizationScope] {
    match credential {
        AccountCredential::CliAccessToken { scopes, .. }
        | AccountCredential::PersonalAccessToken { scopes, .. } => scopes,
    }
}

const fn scope_from_cli(scope: TokenScope) -> AuthorizationScope {
    match scope {
        TokenScope::EndpointsRead => AuthorizationScope::EndpointsRead,
        TokenScope::EndpointsWrite => AuthorizationScope::EndpointsWrite,
        TokenScope::EventsRead => AuthorizationScope::EventsRead,
        TokenScope::TokensRead => AuthorizationScope::TokensRead,
        TokenScope::TokensWrite => AuthorizationScope::TokensWrite,
        TokenScope::McpUse => AuthorizationScope::McpUse,
    }
}

fn idempotency_headers(key: &str) -> Result<HeaderMap, CliError> {
    let value =
        HeaderValue::from_str(key).map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let mut headers = HeaderMap::new();
    headers.insert(IDEMPOTENCY_KEY, value);
    Ok(headers)
}

fn json_body(value: &impl Serialize) -> Result<serde_json::Value, CliError> {
    serde_json::to_value(value).map_err(|error| CliError::Infrastructure(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::TokenExpiration;

    fn args() -> TokenCreateArgs {
        TokenCreateArgs::default()
    }

    #[test]
    fn defaults_to_full_access_and_server_expiration() {
        let request = resolve_token_create_request(&args());
        assert_eq!(request.scopes, AUTHORIZATION_SCOPES);
        assert_eq!(request.expires_in, None);
        assert_eq!(
            json_body(&request).unwrap_or_default(),
            serde_json::json!({
                "scopes": [
                    "endpoints:read", "endpoints:write", "events:read",
                    "tokens:read", "tokens:write", "mcp:use"
                ]
            })
        );
    }

    #[test]
    fn read_only_preset_has_protocol_order() {
        let mut options = args();
        options.preset = Some(TokenPreset::ReadOnly);
        assert_eq!(
            resolve_token_create_request(&options).scopes,
            READ_ONLY_SCOPES
        );
    }

    #[test]
    fn explicit_scopes_are_deduplicated_in_input_order() {
        let mut options = args();
        options.scopes = vec![
            TokenScope::EventsRead,
            TokenScope::EndpointsRead,
            TokenScope::EventsRead,
        ];
        assert_eq!(
            resolve_token_create_request(&options).scopes,
            [
                AuthorizationScope::EventsRead,
                AuthorizationScope::EndpointsRead
            ]
        );
    }

    #[test]
    fn expiration_distinguishes_omitted_null_and_seconds() {
        let mut options = args();
        options.no_expiration = true;
        assert_eq!(
            resolve_token_create_request(&options).expires_in,
            Some(None)
        );

        options.no_expiration = false;
        options.expires_in = Some(
            "30d"
                .parse::<TokenExpiration>()
                .unwrap_or_else(|error| panic!("test duration should parse: {error}")),
        );
        assert_eq!(
            resolve_token_create_request(&options).expires_in,
            Some(Some(2_592_000))
        );
    }

    #[test]
    fn idempotency_key_is_forwarded_as_an_http_header() {
        let headers = idempotency_headers("logical-create").unwrap_or_default();
        assert_eq!(
            headers
                .get("idempotency-key")
                .and_then(|value| value.to_str().ok()),
            Some("logical-create")
        );
    }
}
