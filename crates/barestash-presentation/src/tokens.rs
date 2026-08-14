use std::io::{self, Write};

use barestash_protocol::{
    AuthorizationScope, PersonalAccessTokenCreateResponse, PersonalAccessTokenListResponse,
    PersonalAccessTokenMetadata, PersonalAccessTokenRevokeResponse, PersonalAccessTokenStatus,
};

use crate::output::{print_json, print_lines};
use crate::{
    OutputRenderer, PresentationError, TableColumn, TerminalCapabilities, Tone,
    sanitize_terminal_text,
};

pub fn print_created(
    response: &PersonalAccessTokenCreateResponse,
    json: bool,
) -> Result<(), PresentationError> {
    if json {
        print_json(response)
    } else {
        print_lines(render_created(response, TerminalCapabilities::detect()))
    }
}

pub fn print_list(
    response: &PersonalAccessTokenListResponse,
    json: bool,
) -> Result<(), PresentationError> {
    if json {
        print_json(response)
    } else {
        print_lines(render_list(
            &response.tokens,
            TerminalCapabilities::detect(),
        ))
    }
}

pub fn print_revoked(
    response: &PersonalAccessTokenRevokeResponse,
) -> Result<(), PresentationError> {
    print_lines(render_revoked(response, TerminalCapabilities::detect()))
}

pub fn print_diagnostic(message: &str) -> Result<(), PresentationError> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "{}", sanitize_terminal_text(message))?;
    stderr.flush()?;
    Ok(())
}

fn render_created(
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
                Tone::Warning,
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

fn render_list(
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

fn render_revoked(
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

const fn token_status(status: PersonalAccessTokenStatus) -> &'static str {
    match status {
        PersonalAccessTokenStatus::Active => "active",
        PersonalAccessTokenStatus::Revoked => "revoked",
        PersonalAccessTokenStatus::Expired => "expired",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let lines = render_created(&response, plain());
        assert_eq!(
            lines
                .iter()
                .filter(|line| *line == "bst_pat_secret")
                .count(),
            1
        );
    }
}
