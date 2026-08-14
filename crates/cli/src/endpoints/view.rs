use barestash_protocol::{
    EndpointDeleteResponse, EndpointListResponse, EndpointMetadata, EndpointMode, EndpointResponse,
    EndpointSecretCreateResponse, EndpointSecretListResponse, EndpointSecretMetadata,
    EndpointSecretRevokeResponse, EndpointSecretStatus,
};

use crate::output::{
    OutputRenderer, PresentationError, TableColumn, TerminalCapabilities, Tone, print_json,
    print_lines,
};

pub fn print_created(
    response: &EndpointResponse,
    json: bool,
    default_saved: bool,
) -> Result<(), PresentationError> {
    if json {
        return print_json(response);
    }

    let mut lines = render_created(&response.endpoint, TerminalCapabilities::detect());
    if default_saved {
        lines.push(format!("Default endpoint: {}", response.endpoint.id));
    }
    print_lines(lines)
}

pub fn print_list(response: &EndpointListResponse, json: bool) -> Result<(), PresentationError> {
    if json {
        print_json(response)
    } else {
        print_lines(render_list(
            &response.endpoints,
            TerminalCapabilities::detect(),
        ))
    }
}

pub fn print_detail(response: &EndpointResponse, json: bool) -> Result<(), PresentationError> {
    if json {
        print_json(response)
    } else {
        print_lines(render_detail(
            &response.endpoint,
            TerminalCapabilities::detect(),
        ))
    }
}

pub fn print_deleted(response: &EndpointDeleteResponse) -> Result<(), PresentationError> {
    print_lines(render_deleted(response, TerminalCapabilities::detect()))
}

pub fn print_secret_created(
    response: &EndpointSecretCreateResponse,
    json: bool,
) -> Result<(), PresentationError> {
    if json {
        print_json(response)
    } else {
        print_lines(render_secret_created(
            response,
            TerminalCapabilities::detect(),
        ))
    }
}

pub fn print_secret_list(
    response: &EndpointSecretListResponse,
    json: bool,
) -> Result<(), PresentationError> {
    if json {
        print_json(response)
    } else {
        print_lines(render_secret_list(
            &response.endpoint_secrets,
            TerminalCapabilities::detect(),
        ))
    }
}

pub fn print_secret_revoked(
    response: &EndpointSecretRevokeResponse,
) -> Result<(), PresentationError> {
    print_lines(render_secret_revoked(
        response,
        TerminalCapabilities::detect(),
    ))
}

fn render_created(endpoint: &EndpointMetadata, capabilities: TerminalCapabilities) -> Vec<String> {
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

fn render_list(endpoints: &[EndpointMetadata], capabilities: TerminalCapabilities) -> Vec<String> {
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

fn render_detail(endpoint: &EndpointMetadata, capabilities: TerminalCapabilities) -> Vec<String> {
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

fn render_deleted(
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

#[cfg(test)]
mod tests {
    use super::*;
    use barestash_protocol::EndpointStatus;

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
        let lines = render_created(&endpoint(), plain());
        assert_eq!(lines[0], "Created endpoint: ep_test");
        assert!(lines.contains(&"https://ingest.example.com/ep_test/github/push".into()));
        assert!(lines.contains(&"Mode: temporary".into()));
        assert!(lines.contains(&"Events: 3 / 100".into()));
    }

    #[test]
    fn endpoint_detail_explains_public_read_in_plain_output() {
        let lines = render_detail(&endpoint(), plain());
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
}
