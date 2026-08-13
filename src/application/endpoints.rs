use reqwest::Method;
use reqwest::header::HeaderMap;
use serde::Serialize;

use crate::application::auth::{AuthMode, authenticated_request_json};
use crate::application::{AppContext, CliError};
use crate::cli::endpoints::{
    EndpointAction, EndpointCreateArgs, EndpointDeleteArgs, EndpointListArgs,
    EndpointSecretCreateArgs, EndpointSecretListArgs, EndpointSecretRevokeArgs,
    EndpointSecretsAction, EndpointsCommand,
};
use crate::infrastructure::terminal::confirm;
use crate::presentation::renderer::{TableColumn, Tone};
use crate::presentation::{
    OutputRenderer, TerminalCapabilities, print_json, print_lines, sanitize_terminal_text,
};
use crate::protocol::{
    EndpointCreateRequest, EndpointDeleteResponse, EndpointListResponse, EndpointMetadata,
    EndpointMode, EndpointResponse, EndpointSecretCreateResponse, EndpointSecretListResponse,
    EndpointSecretMetadata, EndpointSecretRevokeResponse, EndpointSecretStatus,
};

/// Runs an `endpoints` subcommand.
pub async fn run(context: &AppContext, command: EndpointsCommand) -> Result<(), CliError> {
    match command.action {
        EndpointAction::Create(args) => create(context, args).await,
        EndpointAction::List(args) => list(context, args).await,
        EndpointAction::Show(args) => show(context, &args.endpoint_id, args.json).await,
        EndpointAction::Delete(args) => delete(context, args).await,
        EndpointAction::Secrets(command) => match command.action {
            EndpointSecretsAction::Create(args) => create_secret(context, args).await,
            EndpointSecretsAction::List(args) => list_secrets(context, args).await,
            EndpointSecretsAction::Revoke(args) => revoke_secret(context, args).await,
        },
    }
}

async fn create(context: &AppContext, args: EndpointCreateArgs) -> Result<(), CliError> {
    if args.private && args.temporary {
        return Err(CliError::Local(
            "Choose either --private or --temporary, not both.".into(),
        ));
    }
    let mode = if args.temporary {
        EndpointMode::Temporary
    } else {
        EndpointMode::Private
    };
    let body = json_body(&EndpointCreateRequest {
        mode,
        name: args.name,
    })?;

    // Temporary endpoint creation is intentionally anonymous. Private endpoint
    // creation uses the same refresh-aware authenticated request path as the
    // remaining owner operations.
    let response: EndpointResponse = if mode == EndpointMode::Temporary {
        context
            .api
            .request_json(Method::POST, "/v1/endpoints", None, Some(body))
            .await
            .map_err(CliError::from_api_client)?
    } else {
        authenticated_request_json(
            context,
            Method::POST,
            "/v1/endpoints",
            HeaderMap::new(),
            Some(body),
            AuthMode::Required,
        )
        .await?
    };

    if args.set_default
        && persist_default_endpoint(context, &response.endpoint.id)
            .await
            .is_err()
    {
        print_endpoint_created(&response.endpoint, args.json)?;
        return Err(default_endpoint_partial_failure(&response.endpoint.id));
    }

    print_endpoint_created(&response.endpoint, args.json)?;
    if args.set_default && !args.json {
        print_lines([format!("Default endpoint: {}", response.endpoint.id)])?;
    }
    Ok(())
}

async fn list(context: &AppContext, args: EndpointListArgs) -> Result<(), CliError> {
    let response: EndpointListResponse = authenticated_request_json(
        context,
        Method::GET,
        "/v1/endpoints",
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;

    if args.json {
        print_json(&response)
    } else {
        print_lines(render_endpoint_list(
            &response.endpoints,
            TerminalCapabilities::detect(),
        ))
    }
}

async fn show(context: &AppContext, endpoint_id: &str, json: bool) -> Result<(), CliError> {
    let response: EndpointResponse = authenticated_request_json(
        context,
        Method::GET,
        &format!("/v1/endpoints/{endpoint_id}"),
        HeaderMap::new(),
        None,
        AuthMode::PublicRead,
    )
    .await?;

    if json {
        print_json(&response)
    } else {
        print_lines(render_endpoint_detail(
            &response.endpoint,
            TerminalCapabilities::detect(),
        ))
    }
}

async fn delete(context: &AppContext, args: EndpointDeleteArgs) -> Result<(), CliError> {
    if !args.yes
        && !confirm(&format!(
            "Delete endpoint {} and all events?",
            sanitize_terminal_text(&args.endpoint_id)
        ))?
    {
        return Err(CliError::Local("Endpoint deletion cancelled.".into()));
    }

    let response: EndpointDeleteResponse = authenticated_request_json(
        context,
        Method::DELETE,
        &format!("/v1/endpoints/{}", args.endpoint_id),
        HeaderMap::new(),
        None,
        AuthMode::Required,
    )
    .await?;
    print_lines(render_endpoint_deleted(
        &response,
        TerminalCapabilities::detect(),
    ))
}

async fn create_secret(
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

    if args.json {
        print_json(&response)
    } else {
        print_lines(render_secret_created(
            &response,
            TerminalCapabilities::detect(),
        ))
    }
}

async fn list_secrets(context: &AppContext, args: EndpointSecretListArgs) -> Result<(), CliError> {
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

    if args.json {
        print_json(&response)
    } else {
        print_lines(render_secret_list(
            &response.endpoint_secrets,
            TerminalCapabilities::detect(),
        ))
    }
}

async fn revoke_secret(
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
    print_lines(render_secret_revoked(
        &response,
        TerminalCapabilities::detect(),
    ))
}

async fn persist_default_endpoint(context: &AppContext, endpoint_id: &str) -> Result<(), CliError> {
    let _guard = context
        .credential_lock
        .acquire()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    let mut config = context
        .config
        .read()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    config.default_endpoint = Some(endpoint_id.to_owned());
    context
        .config
        .write(&config)
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))
}

fn default_endpoint_partial_failure(endpoint_id: &str) -> CliError {
    CliError::Local(format!(
        "Endpoint {endpoint_id} was created, but it could not be saved as the default endpoint.\nUse --endpoint {endpoint_id} or BARESTASH_ENDPOINT={endpoint_id} until local config is writable."
    ))
}

fn print_endpoint_created(endpoint: &EndpointMetadata, json: bool) -> Result<(), CliError> {
    if json {
        print_json(&EndpointResponse {
            endpoint: endpoint.clone(),
        })
    } else {
        print_lines(render_endpoint_created(
            endpoint,
            TerminalCapabilities::detect(),
        ))
    }
}

fn render_endpoint_created(
    endpoint: &EndpointMetadata,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    let event_limit = endpoint
        .event_limit
        .map_or_else(|| "unlimited".to_owned(), |limit| limit.to_string());
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.success(&format!("Endpoint created  {}", endpoint.id)),
            String::new(),
            renderer.section("Webhook URL"),
            endpoint.ingest_url.clone(),
            String::new(),
        ];
        lines.extend(renderer.details([
            ("Mode", endpoint_mode(endpoint.mode).to_owned()),
            ("Expires", endpoint.expires_at.clone()),
            (
                "Events",
                format!("{} / {event_limit}", endpoint.event_count),
            ),
        ]));
        lines.extend([
            String::new(),
            renderer.decorate("Append a path suffix when required:", Tone::Muted, false),
            format!("{}/github/push", endpoint.ingest_url),
        ]);
        return lines;
    }

    vec![
        format!("Created endpoint: {}", endpoint.id),
        String::new(),
        "Webhook URL:".into(),
        endpoint.ingest_url.clone(),
        String::new(),
        "Append a path suffix when the webhook provider requires it:".into(),
        format!("{}/github/push", endpoint.ingest_url),
        String::new(),
        format!("Mode: {}", endpoint_mode(endpoint.mode)),
        format!("Expires: {}", endpoint.expires_at),
        format!("Events: {} / {event_limit}", endpoint.event_count),
    ]
}

fn render_endpoint_list(
    endpoints: &[EndpointMetadata],
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let rows = endpoints
            .iter()
            .map(|endpoint| {
                vec![
                    endpoint.id.clone(),
                    endpoint.name.clone().unwrap_or_else(|| "-".into()),
                    endpoint_mode(endpoint.mode).into(),
                    format!(
                        "{}/{}",
                        endpoint.event_count,
                        endpoint
                            .event_limit
                            .map_or_else(|| "-".into(), |limit| limit.to_string())
                    ),
                    endpoint.expires_at.clone(),
                ]
            })
            .collect::<Vec<_>>();
        let mut lines = vec![
            renderer.heading("Endpoints", Some(&format!("{} total", endpoints.len()))),
            String::new(),
        ];
        lines.extend(renderer.table(
            &[
                TableColumn::new("ID", 12),
                TableColumn::new("NAME", 8).flexible(),
                TableColumn::new("MODE", 9),
                TableColumn::new("EVENTS", 8),
                TableColumn::new("EXPIRES", 10).flexible(),
            ],
            &rows,
        ));
        return lines;
    }

    let mut lines = vec!["ID          NAME          MODE        EVENTS      EXPIRES".into()];
    lines.extend(endpoints.iter().map(|endpoint| {
        format!(
            "{}  {}  {}  {}/{}  {}",
            endpoint.id,
            endpoint.name.as_deref().unwrap_or("-"),
            endpoint_mode(endpoint.mode),
            endpoint.event_count,
            endpoint
                .event_limit
                .map_or_else(|| "-".into(), |limit| limit.to_string()),
            endpoint.expires_at
        )
    }));
    lines
}

fn render_endpoint_detail(
    endpoint: &EndpointMetadata,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    let event_limit = endpoint
        .event_limit
        .map_or_else(|| "unlimited".to_owned(), |limit| limit.to_string());
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.heading("Endpoint", Some(&endpoint.id)),
            String::new(),
        ];
        lines.extend(renderer.details([
            ("Name", endpoint.name.clone().unwrap_or_else(|| "-".into())),
            ("Webhook URL", endpoint.ingest_url.clone()),
            ("Mode", endpoint_mode(endpoint.mode).into()),
            ("Expires", endpoint.expires_at.clone()),
            (
                "Events",
                format!("{} / {event_limit}", endpoint.event_count),
            ),
            (
                "Public read",
                if endpoint.public_read { "yes" } else { "no" }.into(),
            ),
            ("Created", endpoint.created_at.clone()),
        ]));
        return lines;
    }

    vec![
        format!("Endpoint: {}", endpoint.id),
        format!("Name: {}", endpoint.name.as_deref().unwrap_or("-")),
        format!("Webhook URL: {}", endpoint.ingest_url),
        format!("Mode: {}", endpoint_mode(endpoint.mode)),
        format!("Expires: {}", endpoint.expires_at),
        format!("Events: {} / {event_limit}", endpoint.event_count),
        format!(
            "Public read: {}",
            if endpoint.public_read {
                "yes (no authentication required)"
            } else {
                "no"
            }
        ),
        format!("Created: {}", endpoint.created_at),
    ]
}

fn render_endpoint_deleted(
    response: &EndpointDeleteResponse,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let mut lines = vec![
            renderer.success(&format!("Endpoint deleted  {}", response.endpoint.id)),
            String::new(),
        ];
        lines.extend(renderer.details([
            ("Deleted events", response.deleted_events.to_string()),
            (
                "Deleted body objects",
                response.deleted_body_objects.to_string(),
            ),
        ]));
        return lines;
    }
    vec![
        format!("Deleted endpoint: {}", response.endpoint.id),
        format!("Deleted events: {}", response.deleted_events),
        format!("Deleted body objects: {}", response.deleted_body_objects),
    ]
}

fn render_secret_created(
    response: &EndpointSecretCreateResponse,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        return vec![
            renderer.success(&format!(
                "Endpoint secret created  {}",
                response.endpoint_secret.id
            )),
            String::new(),
            renderer.section("Secret (shown once)"),
            response.secret.clone(),
            String::new(),
            renderer.decorate(
                "Save this secret now. It will not be shown again.",
                Tone::Warning,
                true,
            ),
            String::new(),
            renderer.section("Webhook header"),
            format!("  x-barestash-secret: {}", response.secret),
        ];
    }
    vec![
        format!("Created secret: {}", response.endpoint_secret.id),
        String::new(),
        "Secret (shown once):".into(),
        response.secret.clone(),
        String::new(),
        "Save this secret now. It will not be shown again.".into(),
        String::new(),
        "Configure your webhook provider to send:".into(),
        format!("  x-barestash-secret: {}", response.secret),
    ]
}

fn render_secret_list(
    secrets: &[EndpointSecretMetadata],
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        let rows = secrets
            .iter()
            .map(|secret| {
                vec![
                    secret.id.clone(),
                    endpoint_secret_status(secret.status).into(),
                    secret.created_at.clone(),
                    secret
                        .last_used_at
                        .clone()
                        .unwrap_or_else(|| "never".into()),
                ]
            })
            .collect::<Vec<_>>();
        let mut lines = vec![
            renderer.heading(
                "Endpoint secrets",
                Some(&format!("{} total", secrets.len())),
            ),
            String::new(),
        ];
        lines.extend(renderer.table(
            &[
                TableColumn::new("ID", 12),
                TableColumn::new("STATUS", 8),
                TableColumn::new("CREATED", 10).flexible(),
                TableColumn::new("LAST USED", 10).flexible(),
            ],
            &rows,
        ));
        return lines;
    }

    let mut lines = vec!["ID          STATUS   CREATED               LAST_USED".into()];
    lines.extend(secrets.iter().map(|secret| {
        format!(
            "{}  {}  {}  {}",
            secret.id,
            endpoint_secret_status(secret.status),
            secret.created_at,
            secret.last_used_at.as_deref().unwrap_or("never")
        )
    }));
    lines
}

fn render_secret_revoked(
    response: &EndpointSecretRevokeResponse,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    if capabilities.interactive {
        let renderer = OutputRenderer::new(capabilities);
        vec![renderer.success(&format!(
            "Endpoint secret revoked  {}",
            response.endpoint_secret.id
        ))]
    } else {
        vec![format!("Revoked secret: {}", response.endpoint_secret.id)]
    }
}

const fn endpoint_mode(mode: EndpointMode) -> &'static str {
    match mode {
        EndpointMode::Private => "private",
        EndpointMode::Temporary => "temporary",
    }
}

const fn endpoint_secret_status(status: EndpointSecretStatus) -> &'static str {
    match status {
        EndpointSecretStatus::Active => "active",
        EndpointSecretStatus::Revoked => "revoked",
    }
}

fn json_body(value: &impl Serialize) -> Result<serde_json::Value, CliError> {
    serde_json::to_value(value).map_err(|error| CliError::Infrastructure(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::EndpointStatus;

    fn plain() -> TerminalCapabilities {
        TerminalCapabilities {
            interactive: false,
            color: false,
            unicode: false,
            width: 80,
            height: 24,
        }
    }

    fn endpoint() -> EndpointMetadata {
        EndpointMetadata {
            id: "ep_test".into(),
            name: Some("stripe-test".into()),
            mode: EndpointMode::Temporary,
            status: EndpointStatus::Active,
            public_read: true,
            event_count: 3,
            event_limit: Some(100),
            expires_at: "2026-07-06T12:00:00.000Z".into(),
            created_at: "2026-07-05T12:00:00.000Z".into(),
            updated_at: "2026-07-05T12:00:00.000Z".into(),
            ingest_url: "https://ingest.example.com/ep_test".into(),
        }
    }

    #[test]
    fn created_output_preserves_webhook_instructions() {
        let lines = render_endpoint_created(&endpoint(), plain());
        assert_eq!(lines[0], "Created endpoint: ep_test");
        assert!(lines.contains(&"https://ingest.example.com/ep_test/github/push".into()));
        assert!(lines.contains(&"Mode: temporary".into()));
        assert!(lines.contains(&"Events: 3 / 100".into()));
    }

    #[test]
    fn endpoint_detail_explains_public_read_in_plain_output() {
        let lines = render_endpoint_detail(&endpoint(), plain());
        assert!(lines.contains(&"Public read: yes (no authentication required)".into()));
    }

    #[test]
    fn secret_list_does_not_accept_or_render_a_secret_value() {
        let secret = EndpointSecretMetadata {
            id: "sec_test".into(),
            endpoint_id: "ep_test".into(),
            status: EndpointSecretStatus::Active,
            created_at: "2026-07-05T12:00:00.000Z".into(),
            last_used_at: None,
            revoked_at: None,
        };
        assert_eq!(
            render_secret_list(&[secret], plain()),
            vec![
                "ID          STATUS   CREATED               LAST_USED",
                "sec_test  active  2026-07-05T12:00:00.000Z  never"
            ]
        );
    }

    #[test]
    fn default_persistence_failure_reports_recovery_command() {
        let message = default_endpoint_partial_failure("ep_created").to_string();
        assert!(message.contains("Endpoint ep_created was created"));
        assert!(message.contains("--endpoint ep_created"));
        assert!(message.contains("BARESTASH_ENDPOINT=ep_created"));
    }

    #[test]
    fn endpoint_request_omits_absent_name() {
        let body = json_body(&EndpointCreateRequest {
            mode: EndpointMode::Private,
            name: None,
        })
        .unwrap_or_default();
        assert_eq!(body, serde_json::json!({"mode": "private"}));
    }
}
