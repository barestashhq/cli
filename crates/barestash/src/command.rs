use barestash_application as application;

use crate::cli;

impl From<cli::ResourceCommand> for application::AppCommand {
    fn from(command: cli::ResourceCommand) -> Self {
        match command {
            cli::ResourceCommand::Auth(command) => Self::Auth(command.into()),
            cli::ResourceCommand::Endpoints(command) => Self::Endpoints(command.into()),
            cli::ResourceCommand::Events(command) => Self::Events(command.into()),
            cli::ResourceCommand::Tokens(command) => Self::Tokens(command.into()),
        }
    }
}

impl From<cli::auth::AuthCommand> for application::AuthCommand {
    fn from(command: cli::auth::AuthCommand) -> Self {
        Self {
            action: match command.action {
                cli::auth::AuthAction::Login(args) => {
                    application::AuthAction::Login(application::AuthLoginArgs {
                        with_token: args.with_token,
                        insecure_storage: args.insecure_storage,
                    })
                }
                cli::auth::AuthAction::Status(args) => {
                    application::AuthAction::Status(application::AuthStatusArgs { json: args.json })
                }
                cli::auth::AuthAction::Logout(args) => {
                    application::AuthAction::Logout(application::AuthLogoutArgs {
                        revoke: args.revoke,
                    })
                }
            },
        }
    }
}

impl From<cli::endpoints::EndpointsCommand> for application::EndpointsCommand {
    fn from(command: cli::endpoints::EndpointsCommand) -> Self {
        Self {
            action: match command.action {
                cli::endpoints::EndpointAction::Create(args) => {
                    application::EndpointAction::Create(application::EndpointCreateArgs {
                        private: args.private,
                        temporary: args.temporary,
                        name: args.name,
                        set_default: args.set_default,
                        json: args.json,
                    })
                }
                cli::endpoints::EndpointAction::List(args) => {
                    application::EndpointAction::List(application::EndpointListArgs {
                        json: args.json,
                    })
                }
                cli::endpoints::EndpointAction::Show(args) => {
                    application::EndpointAction::Show(application::EndpointShowArgs {
                        endpoint_id: args.endpoint_id,
                        json: args.json,
                    })
                }
                cli::endpoints::EndpointAction::Delete(args) => {
                    application::EndpointAction::Delete(application::EndpointDeleteArgs {
                        endpoint_id: args.endpoint_id,
                        yes: args.yes,
                    })
                }
                cli::endpoints::EndpointAction::Secrets(command) => {
                    application::EndpointAction::Secrets(application::EndpointSecretsCommand {
                        action: match command.action {
                            cli::endpoints::EndpointSecretsAction::Create(args) => {
                                application::EndpointSecretsAction::Create(
                                    application::EndpointSecretCreateArgs {
                                        endpoint: args.endpoint,
                                        json: args.json,
                                    },
                                )
                            }
                            cli::endpoints::EndpointSecretsAction::List(args) => {
                                application::EndpointSecretsAction::List(
                                    application::EndpointSecretListArgs {
                                        endpoint: args.endpoint,
                                        json: args.json,
                                    },
                                )
                            }
                            cli::endpoints::EndpointSecretsAction::Revoke(args) => {
                                application::EndpointSecretsAction::Revoke(
                                    application::EndpointSecretRevokeArgs {
                                        secret_id: args.secret_id,
                                        endpoint: args.endpoint,
                                        yes: args.yes,
                                    },
                                )
                            }
                        },
                    })
                }
            },
        }
    }
}

impl From<cli::events::EventsCommand> for application::EventsCommand {
    fn from(command: cli::events::EventsCommand) -> Self {
        Self {
            action: match command.action {
                cli::events::EventAction::List(args) => {
                    application::EventAction::List(application::EventListArgs {
                        endpoint: args.endpoint,
                        limit: args.limit,
                        json: args.json,
                    })
                }
                cli::events::EventAction::Latest(args) => {
                    application::EventAction::Latest(application::EventLatestArgs {
                        endpoint: args.endpoint,
                        json: args.json,
                    })
                }
                cli::events::EventAction::Show(args) => {
                    application::EventAction::Show(application::EventShowArgs {
                        event_id: args.event_id,
                        json: args.json,
                    })
                }
                cli::events::EventAction::Tail(args) => {
                    application::EventAction::Tail(application::EventTailArgs {
                        endpoint: args.endpoint,
                        last: args.last,
                        headers: args.headers,
                        body: args.body,
                        view: args.view,
                        poll_interval: args.poll_interval,
                    })
                }
                cli::events::EventAction::Stream(args) => {
                    application::EventAction::Stream(application::EventStreamArgs {
                        endpoint: args.endpoint,
                    })
                }
            },
        }
    }
}

impl From<cli::tokens::TokensCommand> for application::TokensCommand {
    fn from(command: cli::tokens::TokensCommand) -> Self {
        Self {
            action: match command.action {
                cli::tokens::TokenAction::Create(args) => {
                    application::TokenAction::Create(application::TokenCreateArgs {
                        name: args.name,
                        scopes: args.scopes.into_iter().map(Into::into).collect(),
                        preset: args.preset.map(Into::into),
                        expires_in: args.expires_in.map(|expiration| {
                            application::TokenExpiration::from_seconds(expiration.as_seconds())
                        }),
                        no_expiration: args.no_expiration,
                        json: args.json,
                    })
                }
                cli::tokens::TokenAction::List(args) => {
                    application::TokenAction::List(application::TokenListArgs {
                        all: args.all,
                        json: args.json,
                    })
                }
                cli::tokens::TokenAction::Revoke(args) => {
                    application::TokenAction::Revoke(application::TokenRevokeArgs {
                        token_id: args.token_id,
                        yes: args.yes,
                    })
                }
            },
        }
    }
}

impl From<cli::tokens::TokenScope> for application::TokenScope {
    fn from(scope: cli::tokens::TokenScope) -> Self {
        match scope {
            cli::tokens::TokenScope::EndpointsRead => Self::EndpointsRead,
            cli::tokens::TokenScope::EndpointsWrite => Self::EndpointsWrite,
            cli::tokens::TokenScope::EventsRead => Self::EventsRead,
            cli::tokens::TokenScope::TokensRead => Self::TokensRead,
            cli::tokens::TokenScope::TokensWrite => Self::TokensWrite,
            cli::tokens::TokenScope::McpUse => Self::McpUse,
        }
    }
}

impl From<cli::tokens::TokenPreset> for application::TokenPreset {
    fn from(preset: cli::tokens::TokenPreset) -> Self {
        match preset {
            cli::tokens::TokenPreset::ReadOnly => Self::ReadOnly,
            cli::tokens::TokenPreset::FullAccess => Self::FullAccess,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_auth_commands_without_losing_flags() {
        let command = cli::ResourceCommand::Auth(cli::auth::AuthCommand {
            action: cli::auth::AuthAction::Login(cli::auth::AuthLoginArgs {
                with_token: true,
                insecure_storage: true,
            }),
        });

        assert_eq!(
            application::AppCommand::from(command),
            application::AppCommand::Auth(application::AuthCommand {
                action: application::AuthAction::Login(application::AuthLoginArgs {
                    with_token: true,
                    insecure_storage: true,
                }),
            })
        );
    }

    #[test]
    fn converts_nested_endpoint_secret_commands() {
        let command = cli::ResourceCommand::Endpoints(cli::endpoints::EndpointsCommand {
            action: cli::endpoints::EndpointAction::Secrets(
                cli::endpoints::EndpointSecretsCommand {
                    action: cli::endpoints::EndpointSecretsAction::Revoke(
                        cli::endpoints::EndpointSecretRevokeArgs {
                            secret_id: "sec_example".into(),
                            endpoint: Some("ep_example".into()),
                            yes: true,
                        },
                    ),
                },
            ),
        });

        assert!(matches!(
            application::AppCommand::from(command),
            application::AppCommand::Endpoints(application::EndpointsCommand {
                action: application::EndpointAction::Secrets(
                    application::EndpointSecretsCommand {
                        action: application::EndpointSecretsAction::Revoke(
                            application::EndpointSecretRevokeArgs {
                                secret_id,
                                endpoint: Some(endpoint),
                                yes: true,
                            }
                        )
                    }
                )
            }) if secret_id == "sec_example" && endpoint == "ep_example"
        ));
    }

    #[test]
    fn converts_event_tail_options() {
        let command = cli::ResourceCommand::Events(cli::events::EventsCommand {
            action: cli::events::EventAction::Tail(cli::events::EventTailArgs {
                endpoint: Some("ep_example".into()),
                last: "3".into(),
                headers: true,
                body: true,
                view: true,
                poll_interval: "500ms".into(),
            }),
        });

        assert!(matches!(
            application::AppCommand::from(command),
            application::AppCommand::Events(application::EventsCommand {
                action: application::EventAction::Tail(application::EventTailArgs {
                    endpoint: Some(endpoint),
                    last,
                    headers: true,
                    body: true,
                    view: true,
                    poll_interval,
                })
            }) if endpoint == "ep_example" && last == "3" && poll_interval == "500ms"
        ));
    }

    #[test]
    fn converts_token_scope_preset_and_expiration() {
        let command = cli::ResourceCommand::Tokens(cli::tokens::TokensCommand {
            action: cli::tokens::TokenAction::Create(cli::tokens::TokenCreateArgs {
                name: Some("agent".into()),
                scopes: vec![cli::tokens::TokenScope::EventsRead],
                preset: Some(cli::tokens::TokenPreset::ReadOnly),
                expires_in: Some("30d".parse().expect("valid expiration")),
                no_expiration: false,
                json: true,
            }),
        });

        assert!(matches!(
            application::AppCommand::from(command),
            application::AppCommand::Tokens(application::TokensCommand {
                action: application::TokenAction::Create(application::TokenCreateArgs {
                    name: Some(name),
                    scopes,
                    preset: Some(application::TokenPreset::ReadOnly),
                    expires_in: Some(expiration),
                    no_expiration: false,
                    json: true,
                })
            }) if name == "agent"
                && scopes == vec![application::TokenScope::EventsRead]
                && expiration.as_seconds() == 2_592_000
        ));
    }
}
