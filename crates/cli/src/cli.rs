//! Command-line argument definitions.
//!
//! This module intentionally contains no command execution logic.  Keeping the
//! parser separate makes the command surface testable without constructing API,
//! credential, or terminal dependencies.

#[cfg(test)]
use std::ffi::OsString;

use clap::{Parser, Subcommand};

/// Parsed `barestash` invocation.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "barestash",
    about = "Headless request stash CLI",
    disable_version_flag = true,
    override_help = "Usage: barestash {resource} {action}\n\nResources: auth, endpoints, events, tokens\n\nRun `barestash --help` to show this message.\n"
)]
pub struct Cli {
    /// Print the CLI version.
    #[arg(short = 'V', long, global = true)]
    pub version: bool,

    /// Allow a private or link-local Barestash API URL.
    #[arg(long, global = true)]
    pub allow_insecure_api_url: bool,

    /// Resource command to execute.
    #[command(subcommand)]
    pub command: Option<ResourceCommand>,
}

/// Top-level resource commands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum ResourceCommand {
    /// Manage authentication.
    Auth(crate::auth::AuthCommand),

    /// Manage webhook endpoints.
    Endpoints(crate::endpoints::EndpointsCommand),

    /// Read captured events.
    Events(crate::events::EventsCommand),

    /// Manage API tokens.
    Tokens(crate::tokens::TokensCommand),
}

/// Parse an invocation without exiting the process.
///
/// Production entrypoints can decide how to route clap diagnostics and map
/// their [`clap::error::ErrorKind`] to the CLI exit-code contract.
///
/// # Errors
///
/// Returns a clap diagnostic for help, version, syntax, and value-validation
/// outcomes. Callers should render that diagnostic on clap's selected stream.
#[cfg(test)]
pub fn parse_from<I, T>(arguments: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(arguments)
}

/// Return the reference CLI's custom diagnostic target when the resource or
/// `events` action is not recognized. Global insecure-URL flags are removed
/// before command recognition, matching the original argument preprocessing.
#[must_use]
pub fn unknown_command(arguments: &[String]) -> Option<String> {
    let filtered = arguments
        .iter()
        .filter(|argument| argument.as_str() != "--allow-insecure-api-url")
        .collect::<Vec<_>>();
    let first = filtered.first()?.as_str();

    if !matches!(
        first,
        "auth" | "endpoints" | "events" | "tokens" | "--help" | "-h" | "--version" | "-V"
    ) {
        return Some(arguments.join(" "));
    }

    if first != "events" {
        return None;
    }

    let action = filtered.get(1).map(|argument| argument.as_str());
    let help_target = filtered.get(2).map(|argument| argument.as_str());
    let known_action = matches!(
        action,
        None | Some("list" | "latest" | "show" | "tail" | "stream" | "--help" | "-h")
    );
    let known_help = action == Some("help")
        && filtered.len() <= 3
        && matches!(
            help_target,
            None | Some("list" | "latest" | "show" | "tail" | "stream")
        );

    (!known_action && !known_help).then(|| {
        filtered
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    })
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, error::ErrorKind};

    use super::*;
    use crate::auth::AuthAction;
    use crate::endpoints::{EndpointAction, EndpointSecretsAction};
    use crate::events::EventAction;
    use crate::tokens::{TokenAction, TokenPreset, TokenScope};

    #[test]
    fn accepts_an_empty_invocation_for_successful_root_help() {
        let parsed = parse_from(["barestash"]).expect("root invocation should parse");

        assert_eq!(parsed.command, None);
        assert!(!parsed.version);
        assert!(!parsed.allow_insecure_api_url);
    }

    #[test]
    fn exposes_help_and_version() {
        let help = parse_from(["barestash", "--help"]).expect_err("help short-circuits");
        assert_eq!(help.kind(), ErrorKind::DisplayHelp);
        let rendered_help = help.to_string();
        assert!(rendered_help.contains("Usage: barestash"));
        assert!(rendered_help.contains("auth"));
        assert!(rendered_help.contains("endpoints"));
        assert!(rendered_help.contains("events"));
        assert!(rendered_help.contains("tokens"));
        assert!(rendered_help.contains("Usage: barestash {resource} {action}"));
        assert!(rendered_help.contains("Resources: auth, endpoints, events, tokens"));

        for flag in ["--version", "-V"] {
            let version = parse_from(["barestash", flag]).expect("version flag should parse");
            assert!(version.version);
            assert_eq!(version.command, None);
        }

        let mut command = Cli::command();
        command.build();
        assert!(command.find_subcommand("events").is_some());
    }

    #[test]
    fn preserves_custom_unknown_command_diagnostics() {
        assert_eq!(
            unknown_command(&["unknown".to_owned()]),
            Some("unknown".to_owned())
        );
        assert_eq!(
            unknown_command(&["unknown".to_owned(), "--allow-insecure-api-url".to_owned(),]),
            Some("unknown --allow-insecure-api-url".to_owned())
        );
        assert_eq!(
            unknown_command(&[
                "events".to_owned(),
                "help".to_owned(),
                "tail".to_owned(),
                "stream".to_owned(),
            ]),
            Some("events help tail stream".to_owned())
        );
        assert_eq!(
            unknown_command(&["events".to_owned(), "help".to_owned(), "tail".to_owned(),]),
            None
        );
    }

    #[test]
    fn exposes_help_for_resources_and_every_action() {
        for arguments in [
            vec!["auth", "--help"],
            vec!["auth", "login", "--help"],
            vec!["auth", "status", "--help"],
            vec!["auth", "logout", "--help"],
            vec!["endpoints", "--help"],
            vec!["endpoints", "create", "--help"],
            vec!["endpoints", "list", "--help"],
            vec!["endpoints", "show", "--help"],
            vec!["endpoints", "delete", "--help"],
            vec!["endpoints", "secrets", "--help"],
            vec!["endpoints", "secrets", "create", "--help"],
            vec!["endpoints", "secrets", "list", "--help"],
            vec!["endpoints", "secrets", "revoke", "--help"],
            vec!["events", "--help"],
            vec!["events", "list", "--help"],
            vec!["events", "latest", "--help"],
            vec!["events", "show", "--help"],
            vec!["events", "tail", "--help"],
            vec!["events", "stream", "--help"],
            vec!["tokens", "--help"],
            vec!["tokens", "create", "--help"],
            vec!["tokens", "list", "--help"],
            vec!["tokens", "revoke", "--help"],
        ] {
            let help = parse_tail(&arguments).expect_err("help should short-circuit");
            assert_eq!(
                help.kind(),
                ErrorKind::DisplayHelp,
                "unexpected result for {arguments:?}"
            );
        }

        let help = parse_tail(&["events", "help", "tail"])
            .expect_err("help subcommand should short-circuit");
        assert_eq!(help.kind(), ErrorKind::DisplayHelp);
        assert!(help.to_string().contains("--poll-interval"));
    }

    #[test]
    fn parses_every_auth_action() {
        let cases = [
            vec!["auth", "login"],
            vec!["auth", "login", "--with-token", "--insecure-storage"],
            vec!["auth", "status", "--json"],
            vec!["auth", "logout", "--revoke"],
        ];

        for arguments in cases {
            let parsed = parse_tail(&arguments).expect("auth action should parse");
            let Some(ResourceCommand::Auth(command)) = parsed.command else {
                panic!("expected auth command");
            };
            assert!(matches!(
                command.action,
                AuthAction::Login(_) | AuthAction::Status(_) | AuthAction::Logout(_)
            ));
        }
    }

    #[test]
    fn parses_every_endpoint_action() {
        let cases = [
            vec!["endpoints", "create", "--private", "--name", "github-dev"],
            vec![
                "endpoints",
                "create",
                "--temporary",
                "--set-default",
                "--json",
            ],
            vec!["endpoints", "list", "--json"],
            vec!["endpoints", "show", "ep_example", "--json"],
            vec!["endpoints", "delete", "ep_example", "--yes"],
            vec![
                "endpoints",
                "secrets",
                "create",
                "--endpoint",
                "ep_example",
                "--json",
            ],
            vec![
                "endpoints",
                "secrets",
                "list",
                "--endpoint",
                "ep_example",
                "--json",
            ],
            vec![
                "endpoints",
                "secrets",
                "revoke",
                "sec_example",
                "--endpoint",
                "ep_example",
                "--yes",
            ],
        ];

        for arguments in cases {
            let parsed = parse_tail(&arguments).expect("endpoint action should parse");
            let Some(ResourceCommand::Endpoints(command)) = parsed.command else {
                panic!("expected endpoints command");
            };
            match command.action {
                EndpointAction::Create(_)
                | EndpointAction::List(_)
                | EndpointAction::Show(_)
                | EndpointAction::Delete(_) => {}
                EndpointAction::Secrets(secrets) => assert!(matches!(
                    secrets.action,
                    EndpointSecretsAction::Create(_)
                        | EndpointSecretsAction::List(_)
                        | EndpointSecretsAction::Revoke(_)
                )),
            }
        }
    }

    #[test]
    fn parses_every_event_action() {
        let cases = [
            vec![
                "events",
                "list",
                "--endpoint",
                "ep_example",
                "--limit",
                "20",
                "--json",
            ],
            vec!["events", "latest", "--endpoint", "ep_example", "--json"],
            vec!["events", "show", "evt_example", "--json"],
            vec![
                "events",
                "tail",
                "--endpoint",
                "ep_example",
                "--last",
                "10",
                "--headers",
                "--body",
                "--poll-interval",
                "500ms",
            ],
            vec!["events", "tail", "--endpoint", "ep_example", "--view"],
            vec!["events", "stream", "--endpoint", "ep_example"],
        ];

        for arguments in cases {
            let parsed = parse_tail(&arguments).expect("event action should parse");
            let Some(ResourceCommand::Events(command)) = parsed.command else {
                panic!("expected events command");
            };
            assert!(matches!(
                command.action,
                EventAction::List(_)
                    | EventAction::Latest(_)
                    | EventAction::Show(_)
                    | EventAction::Tail(_)
                    | EventAction::Stream(_)
            ));
        }
    }

    #[test]
    fn parses_every_token_action_and_value_enum() {
        let parsed = parse_tail(&[
            "tokens",
            "create",
            "--name",
            "ci-github",
            "--scope",
            "endpoints:read",
            "--scope",
            "events:read",
            "--expires-in",
            "90d",
            "--json",
        ])
        .expect("token create should parse");
        let Some(ResourceCommand::Tokens(command)) = parsed.command else {
            panic!("expected tokens command");
        };
        let TokenAction::Create(create) = command.action else {
            panic!("expected token create");
        };
        assert_eq!(
            create.scopes,
            vec![TokenScope::EndpointsRead, TokenScope::EventsRead]
        );
        assert_eq!(
            create.expires_in.expect("expiration").as_seconds(),
            7_776_000
        );

        let parsed = parse_tail(&[
            "tokens",
            "create",
            "--preset",
            "read-only",
            "--no-expiration",
        ])
        .expect("preset should parse");
        let Some(ResourceCommand::Tokens(command)) = parsed.command else {
            panic!("expected tokens command");
        };
        let TokenAction::Create(create) = command.action else {
            panic!("expected token create");
        };
        assert_eq!(create.preset, Some(TokenPreset::ReadOnly));

        for arguments in [
            vec!["tokens", "list", "--all", "--json"],
            vec!["tokens", "revoke", "tok_example", "--yes"],
        ] {
            let parsed = parse_tail(&arguments).expect("token action should parse");
            let Some(ResourceCommand::Tokens(command)) = parsed.command else {
                panic!("expected tokens command");
            };
            assert!(matches!(
                command.action,
                TokenAction::List(_) | TokenAction::Revoke(_)
            ));
        }
    }

    #[test]
    fn accepts_the_global_security_flag_at_any_position() {
        for arguments in [
            vec!["--allow-insecure-api-url", "auth", "status"],
            vec!["auth", "--allow-insecure-api-url", "status"],
            vec!["auth", "status", "--allow-insecure-api-url"],
        ] {
            let parsed = parse_tail(&arguments).expect("global flag should parse anywhere");
            assert!(parsed.allow_insecure_api_url);
        }
    }

    #[test]
    fn leaves_behavioral_conflicts_and_event_values_for_command_validation() {
        for arguments in [
            vec!["endpoints", "create", "--private", "--temporary"],
            vec!["events", "tail", "--last", "-1"],
            vec!["events", "tail", "--poll-interval", "2"],
            vec!["events", "tail", "--view", "--headers"],
            vec!["events", "tail", "--view", "--body"],
            vec![
                "tokens",
                "create",
                "--scope",
                "events:read",
                "--preset",
                "full-access",
            ],
            vec!["tokens", "create", "--expires-in", "90d", "--no-expiration"],
        ] {
            assert!(
                parse_tail(&arguments).is_ok(),
                "expected command-level validation for {arguments:?}"
            );
        }

        for arguments in [
            vec!["tokens", "create", "--scope", "not:a-scope"],
            vec!["tokens", "create", "--expires-in", "0d"],
        ] {
            assert!(
                parse_tail(&arguments).is_err(),
                "expected parser rejection for {arguments:?}"
            );
        }

        // Resource IDs and list limits remain server-validated for behavioral
        // compatibility with the reference CLI.
        assert!(parse_tail(&["events", "show", "opaque-event-id"]).is_ok());
        assert!(parse_tail(&["events", "list", "--endpoint", "opaque", "--limit", "0"]).is_ok());
    }

    fn parse_tail(arguments: &[&str]) -> Result<Cli, clap::Error> {
        parse_from(std::iter::once("barestash").chain(arguments.iter().copied()))
    }
}
