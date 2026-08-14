use serde_json::{Value, json};

use barestash_protocol::{AccountCredential, AccountResponse};

use crate::{OutputRenderer, PresentationError, TerminalCapabilities, print_json, print_lines};

/// Authentication result displayed after a successful login.
#[derive(Clone, Copy, Debug)]
pub struct AuthLoginView<'a> {
    pub principal: &'a AccountResponse,
    pub session_expires_at: Option<&'a str>,
}

/// Current authentication state displayed by `auth status`.
#[derive(Clone, Copy, Debug)]
pub struct AuthStatusView<'a> {
    pub principal: Option<&'a AccountResponse>,
    pub default_endpoint: Option<&'a str>,
}

/// Prints a successful authentication result.
///
/// # Errors
///
/// Returns an output error when stdout cannot be written.
pub fn print_auth_login(view: AuthLoginView<'_>) -> Result<(), PresentationError> {
    print_lines(login_lines(view, TerminalCapabilities::detect()))
}

/// Prints the current authentication state in the requested format.
///
/// # Errors
///
/// Returns an output or serialization error when stdout cannot be written.
pub fn print_auth_status(
    view: AuthStatusView<'_>,
    json_output: bool,
) -> Result<(), PresentationError> {
    if json_output {
        return print_json(&status_json(view));
    }
    print_lines(status_lines(view, TerminalCapabilities::detect()))
}

/// Prints confirmation that the local authentication credential was removed.
///
/// # Errors
///
/// Returns an output error when stdout cannot be written.
pub fn print_logged_out() -> Result<(), PresentationError> {
    let capabilities = TerminalCapabilities::detect();
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        print_lines([renderer.success("Logged out")])
    } else {
        print_lines(["Logged out.".to_owned()])
    }
}

fn login_lines(view: AuthLoginView<'_>, capabilities: TerminalCapabilities) -> Vec<String> {
    let identity = view
        .principal
        .account
        .primary_email
        .as_deref()
        .unwrap_or(&view.principal.account.id);
    let id = match &view.principal.credential {
        AccountCredential::CliAccessToken { session_id, .. } => session_id,
        AccountCredential::PersonalAccessToken { id, .. } => id,
    };
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.success(&format!("Authenticated as {identity}")),
            String::new(),
        ];
        lines.extend(renderer.details([
            ("Credential", id.clone()),
            (
                "Session expires",
                view.session_expires_at.unwrap_or("never").to_owned(),
            ),
        ]));
        lines
    } else {
        let mut lines = vec![format!("Authenticated as {identity} ({id})")];
        if let Some(expires_at) = view.session_expires_at {
            lines.push(format!("Session expires: {expires_at}"));
        }
        lines
    }
}

fn status_json(view: AuthStatusView<'_>) -> Value {
    if let Some(principal) = view.principal {
        json!({
            "authenticated": true,
            "account": principal.account,
            "credential": principal.credential,
            "default_endpoint": view.default_endpoint,
        })
    } else {
        json!({
            "authenticated": false,
            "account": Value::Null,
            "credential": Value::Null,
            "default_endpoint": view.default_endpoint,
        })
    }
}

fn status_lines(view: AuthStatusView<'_>, capabilities: TerminalCapabilities) -> Vec<String> {
    let Some(principal) = view.principal else {
        if capabilities.interactive {
            let renderer = OutputRenderer::new(capabilities);
            return vec![renderer.heading("Authentication", Some("not authenticated"))];
        }
        return vec!["Not authenticated.".to_owned()];
    };

    let identity = principal
        .account
        .primary_email
        .as_deref()
        .unwrap_or(&principal.account.id);
    let (kind, id, scopes, expires) = match &principal.credential {
        AccountCredential::CliAccessToken {
            id,
            scopes,
            expires_at,
            ..
        } => ("cli_access_token", id, scopes, Some(expires_at.as_str())),
        AccountCredential::PersonalAccessToken {
            id,
            scopes,
            expires_at,
        } => ("personal_access_token", id, scopes, expires_at.as_deref()),
    };
    let scope_text = scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.heading("Authentication", Some(identity)),
            String::new(),
        ];
        lines.extend(renderer.details([
            ("Credential", format!("{kind} ({id})")),
            ("Scopes", scope_text),
            ("Expires", expires.unwrap_or("never").to_owned()),
            (
                "Default endpoint",
                view.default_endpoint.unwrap_or("none").to_owned(),
            ),
        ]));
        lines
    } else {
        vec![
            format!("Authenticated as {identity}"),
            format!("Credential: {kind} ({id})"),
            format!("Scopes: {scope_text}"),
            format!("Expires: {}", expires.unwrap_or("never")),
            format!(
                "Default endpoint: {}",
                view.default_endpoint.unwrap_or("none")
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use barestash_protocol::{
        AccountCredential, AccountMetadata, AccountResponse, AuthorizationScope,
    };

    use super::*;

    fn capabilities(interactive: bool) -> TerminalCapabilities {
        TerminalCapabilities {
            interactive,
            color: false,
            unicode: false,
            width: 80,
            height: 24,
        }
    }

    fn principal() -> AccountResponse {
        AccountResponse {
            account: AccountMetadata {
                id: "acc_test".into(),
                primary_email: Some("user@example.com".into()),
            },
            credential: AccountCredential::CliAccessToken {
                id: "atk_test".into(),
                session_id: "cls_test".into(),
                scopes: vec![AuthorizationScope::EventsRead],
                expires_at: "2026-08-13T01:00:00.000Z".into(),
            },
        }
    }

    #[test]
    fn login_lines_preserve_interactive_and_plain_contracts() {
        let principal = principal();
        let view = AuthLoginView {
            principal: &principal,
            session_expires_at: Some("2026-11-13T00:00:00.000Z"),
        };
        assert_eq!(
            login_lines(view, capabilities(false)),
            [
                "Authenticated as user@example.com (cls_test)",
                "Session expires: 2026-11-13T00:00:00.000Z",
            ]
        );
        assert_eq!(
            login_lines(view, capabilities(true)),
            [
                "OK Authenticated as user@example.com",
                "",
                "  Credential       cls_test",
                "  Session expires  2026-11-13T00:00:00.000Z",
            ]
        );
    }

    #[test]
    fn unauthenticated_status_preserves_json_and_human_contracts() {
        let view = AuthStatusView {
            principal: None,
            default_endpoint: Some("ep_default"),
        };
        assert_eq!(
            status_json(view),
            json!({
                "authenticated": false,
                "account": Value::Null,
                "credential": Value::Null,
                "default_endpoint": "ep_default",
            })
        );
        assert_eq!(
            status_lines(view, capabilities(false)),
            ["Not authenticated."]
        );
        assert_eq!(
            status_lines(view, capabilities(true)),
            ["AUTHENTICATION  not authenticated"]
        );
    }

    #[test]
    fn authenticated_status_preserves_human_contract() {
        let principal = principal();
        let view = AuthStatusView {
            principal: Some(&principal),
            default_endpoint: None,
        };
        assert_eq!(
            status_lines(view, capabilities(false)),
            [
                "Authenticated as user@example.com",
                "Credential: cli_access_token (atk_test)",
                "Scopes: events:read",
                "Expires: 2026-08-13T01:00:00.000Z",
                "Default endpoint: none",
            ]
        );
    }
}
