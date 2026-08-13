use std::io::{self, Write};

use reqwest::Method;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use uuid::Uuid;

use crate::application::auth::{AuthMode, auth_headers, authenticated_request_json};
use crate::application::{AppContext, CliError};
use crate::cli::tokens::{
    TokenAction, TokenCreateArgs, TokenListArgs, TokenPreset, TokenRevokeArgs, TokenScope,
    TokensCommand,
};
use crate::domain::StoredCredential;
use crate::infrastructure::terminal::confirm;
use crate::presentation::renderer::TableColumn;
use crate::presentation::{
    OutputRenderer, TerminalCapabilities, print_json, print_lines, sanitize_terminal_text,
};
use barestash_protocol::{
    AUTHORIZATION_SCOPES, AccountCredential, AccountResponse, AuthorizationScope, BearerTokenType,
    PersonalAccessTokenCreateRequest, PersonalAccessTokenCreateResponse,
    PersonalAccessTokenListResponse, PersonalAccessTokenMetadata,
    PersonalAccessTokenRevokeResponse, PersonalAccessTokenStatus, parse_bearer_token_string,
    token_id_from_bearer_token_string,
};

const READ_ONLY_SCOPES: [AuthorizationScope; 3] = [
    AuthorizationScope::EndpointsRead,
    AuthorizationScope::EventsRead,
    AuthorizationScope::McpUse,
];

const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");

/// Runs a `tokens` subcommand.
pub async fn run(context: &AppContext, command: TokensCommand) -> Result<(), CliError> {
    match command.action {
        TokenAction::Create(args) => create(context, args).await,
        TokenAction::List(args) => list(context, args).await,
        TokenAction::Revoke(args) => revoke(context, args).await,
    }
}

async fn create(context: &AppContext, args: TokenCreateArgs) -> Result<(), CliError> {
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
        diagnostic(&format!(
            "Scopes: {}",
            request
                .scopes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ))?;
        if request.expires_in == Some(None) {
            diagnostic("Warning: this token will not expire automatically.")?;
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

    if args.json {
        print_json(&response)
    } else {
        print_lines(render_token_created(
            &response,
            TerminalCapabilities::detect(),
        ))
    }
}

async fn list(context: &AppContext, args: TokenListArgs) -> Result<(), CliError> {
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

    if args.json {
        print_json(&response)
    } else {
        print_lines(render_token_list(
            &response.tokens,
            TerminalCapabilities::detect(),
        ))
    }
}

async fn revoke(context: &AppContext, args: TokenRevokeArgs) -> Result<(), CliError> {
    if current_personal_access_token_id(context).await?.as_deref() == Some(args.token_id.as_str()) {
        diagnostic("Warning: this token is currently used by the CLI.")?;
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
    print_lines(render_token_revoked(
        &response,
        TerminalCapabilities::detect(),
    ))
}

/// Resolves flags into the exact three-state API expiration contract and the
/// canonical, de-duplicated scope order.
#[must_use]
pub fn resolve_token_create_request(args: &TokenCreateArgs) -> PersonalAccessTokenCreateRequest {
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

fn render_token_created(
    response: &PersonalAccessTokenCreateResponse,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        return vec![
            renderer.success(&format!("Token created  {}", response.id)),
            String::new(),
            renderer.section("Token (shown once)"),
            response.token.clone(),
            String::new(),
            renderer.decorate(
                "Save this token now. It will not be shown again.",
                crate::presentation::renderer::Tone::Warning,
                true,
            ),
            String::new(),
            renderer.section("Use it with"),
            "  export BARESTASH_TOKEN=...".into(),
            "  echo \"$BARESTASH_TOKEN\" | barestash auth login --with-token".into(),
        ];
    }
    vec![
        format!("Created token: {}", response.id),
        String::new(),
        "Token (shown once):".into(),
        response.token.clone(),
        String::new(),
        "Save this token now. It will not be shown again.".into(),
        String::new(),
        "Use it with:".into(),
        "  export BARESTASH_TOKEN=...".into(),
        "  echo \"$BARESTASH_TOKEN\" | barestash auth login --with-token".into(),
    ]
}

fn render_token_list(
    tokens: &[PersonalAccessTokenMetadata],
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let rows = tokens
            .iter()
            .map(|token| {
                vec![
                    token.id.clone(),
                    token.name.clone().unwrap_or_else(|| "-".into()),
                    joined_scopes(&token.scopes),
                    token.expires_at.clone().unwrap_or_else(|| "never".into()),
                    token.last_used_at.clone().unwrap_or_else(|| "never".into()),
                    token_status(token.status).into(),
                ]
            })
            .collect::<Vec<_>>();
        let mut lines = vec![
            renderer.heading("Tokens", Some(&format!("{} total", tokens.len()))),
            String::new(),
        ];
        lines.extend(renderer.table(
            &[
                TableColumn::new("ID", 12),
                TableColumn::new("NAME", 8).flexible(),
                TableColumn::new("SCOPES", 12).flexible(),
                TableColumn::new("EXPIRES", 10).flexible(),
                TableColumn::new("LAST USED", 10).flexible(),
                TableColumn::new("STATUS", 7),
            ],
            &rows,
        ));
        return lines;
    }

    let mut lines = vec![
        "ID          NAME         SCOPES                       EXPIRES                  LAST_USED             STATUS"
            .into(),
    ];
    lines.extend(tokens.iter().map(|token| {
        format!(
            "{}  {}  {}  {}  {}  {}",
            token.id,
            token.name.as_deref().unwrap_or("-"),
            joined_scopes(&token.scopes),
            token.expires_at.as_deref().unwrap_or("never"),
            token.last_used_at.as_deref().unwrap_or("never"),
            token_status(token.status)
        )
    }));
    lines
}

fn render_token_revoked(
    response: &PersonalAccessTokenRevokeResponse,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        vec![
            OutputRenderer::new(capabilities)
                .success(&format!("Token revoked  {}", response.token.id)),
        ]
    } else {
        vec![format!("Revoked token: {}", response.token.id)]
    }
}

fn joined_scopes(scopes: &[AuthorizationScope]) -> String {
    scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
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

const fn token_status(status: PersonalAccessTokenStatus) -> &'static str {
    match status {
        PersonalAccessTokenStatus::Active => "active",
        PersonalAccessTokenStatus::Revoked => "revoked",
        PersonalAccessTokenStatus::Expired => "expired",
    }
}

fn diagnostic(message: &str) -> Result<(), CliError> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "{message}")?;
    stderr.flush()?;
    Ok(())
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
    use crate::cli::tokens::TokenExpiration;

    fn args() -> TokenCreateArgs {
        TokenCreateArgs::default()
    }

    fn plain() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            unicode: false,
            width: 80,
            height: 24,
        }
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
    fn created_output_prints_token_secret_exactly_once() {
        let response = PersonalAccessTokenCreateResponse {
            id: "tok_created".into(),
            name: Some("ci".into()),
            status: PersonalAccessTokenStatus::Active,
            scopes: vec![AuthorizationScope::EventsRead],
            created_at: "2026-07-05T12:00:00.000Z".into(),
            expires_at: None,
            last_used_at: None,
            revoked_at: None,
            token: "bst_pat_secret".into(),
        };
        let lines = render_token_created(&response, plain());
        assert_eq!(
            lines
                .iter()
                .filter(|line| *line == "bst_pat_secret")
                .count(),
            1
        );
    }

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
