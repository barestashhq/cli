use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(unix)]
use std::{process::Stdio, time::Duration};

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use serde_json::{Value, json};
use tempfile::TempDir;
use wiremock::matchers::{
    body_json, body_partial_json, header, header_exists, method, path, query_param,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ENDPOINT_ID: &str = "ep_integration";

fn pat() -> String {
    format!("bst_pat_{}_{}", "A".repeat(24), "B".repeat(32))
}

fn old_access_token() -> String {
    format!("bst_access_{}_{}", "C".repeat(24), "D".repeat(32))
}

fn new_access_token() -> String {
    format!("bst_access_{}_{}", "E".repeat(24), "F".repeat(32))
}

fn old_refresh_token() -> String {
    format!("bst_refresh_{}_{}", "G".repeat(24), "H".repeat(32))
}

fn new_refresh_token() -> String {
    format!("bst_refresh_{}_{}", "I".repeat(24), "J".repeat(32))
}

fn token_id_from_pat() -> String {
    format!("tok_{}", "A".repeat(24))
}

fn command(server: &MockServer, directory: &TempDir) -> Command {
    let mut command = cargo_bin_cmd!("barestash");
    command
        .env("HOME", directory.path())
        .env(
            "BARESTASH_CONFIG_FILE",
            directory.path().join("config.toml"),
        )
        .env("BARESTASH_API_URL", server.uri())
        .env("BARESTASH_ALLOW_INSECURE_API_URL", "1")
        .env("BARESTASH_TEST_KEYRING_UNAVAILABLE", "1")
        .env("BARESTASH_TOKEN", "")
        .env("BARESTASH_ENDPOINT", "")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");
    command
}

fn parse_stdout(assertion: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assertion.get_output().stdout)
        .expect("successful --json output should be one JSON document")
}

fn account_for_pat(token_id: &str) -> Value {
    json!({
        "account": {
            "id": "acc_integration",
            "primary_email": "integration@example.com"
        },
        "credential": {
            "type": "personal_access_token",
            "id": token_id,
            "scopes": [
                "endpoints:read",
                "endpoints:write",
                "events:read",
                "tokens:read",
                "tokens:write",
                "mcp:use"
            ],
            "expires_at": null
        }
    })
}

fn account_for_cli(access_token_id: &str) -> Value {
    json!({
        "account": {
            "id": "acc_integration",
            "primary_email": "integration@example.com"
        },
        "credential": {
            "type": "cli_access_token",
            "id": access_token_id,
            "session_id": "cls_integration",
            "scopes": ["events:read"],
            "expires_at": "2027-01-01T00:00:00.000Z"
        }
    })
}

fn endpoint_response() -> Value {
    json!({
        "endpoint": {
            "id": ENDPOINT_ID,
            "name": "integration",
            "mode": "temporary",
            "status": "active",
            "public_read": true,
            "event_count": 0,
            "event_limit": 100,
            "expires_at": "2026-08-14T00:00:00.000Z",
            "created_at": "2026-08-13T00:00:00.000Z",
            "updated_at": "2026-08-13T00:00:00.000Z",
            "ingest_url": "https://example.test/i/ep_integration"
        }
    })
}

fn event_metadata() -> Value {
    json!({
        "id": "evt_integration",
        "endpoint_id": ENDPOINT_ID,
        "received_at": "2026-08-13T01:02:03.000Z",
        "method": "POST",
        "request_path": "/webhooks/github",
        "query": {"delivery": "42"},
        "headers": {
            "Authorization": "server-secret",
            "content-type": "application/json",
            "x-barestash-secret": "ingest-secret"
        },
        "body": {
            "size": 11,
            "sha256": "sha256-fixture",
            "available": true
        }
    })
}

fn event_detail() -> Value {
    json!({
        "id": "evt_integration",
        "endpoint_id": ENDPOINT_ID,
        "received_at": "2026-08-13T01:02:03.000Z",
        "request": {
            "method": "POST",
            "ingest_path": "/i/ep_integration/webhooks/github",
            "request_path": "/webhooks/github",
            "query": {"delivery": "42"},
            "headers": {
                "Authorization": "server-secret",
                "content-type": "application/json",
                "x-barestash-secret": "ingest-secret"
            },
            "body": {
                "size": 11,
                "sha256": "sha256-fixture",
                "available": true
            }
        }
    })
}

#[test]
fn help_and_version_do_not_validate_an_unrelated_api_url() {
    let mut help = cargo_bin_cmd!("barestash");
    help.env("BARESTASH_API_URL", "not a URL")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("barestash"))
        .stderr(predicate::str::is_empty());

    for flag in ["--version", "-V"] {
        let mut version = cargo_bin_cmd!("barestash");
        version
            .env("BARESTASH_API_URL", "not a URL")
            .arg(flag)
            .assert()
            .success()
            .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")))
            .stderr(predicate::str::is_empty());
    }
}

#[test]
fn root_help_and_unknown_commands_preserve_the_reference_contract() {
    cargo_bin_cmd!("barestash")
        .arg("--help")
        .env("BARESTASH_API_URL", "not a URL")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Usage: barestash {resource} {action}",
        ))
        .stdout(predicate::str::contains(
            "Resources: auth, endpoints, events, tokens",
        ))
        .stderr(predicate::str::is_empty());

    cargo_bin_cmd!("barestash")
        .arg("unknown")
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Unknown command: unknown"))
        .stderr(predicate::str::contains(
            "Run `barestash --help` for usage.",
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creates_a_temporary_endpoint_and_persists_the_default_as_toml() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let response = endpoint_response();

    Mock::given(method("POST"))
        .and(path("/v1/endpoints"))
        .and(body_json(json!({
            "mode": "temporary",
            "name": "integration"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let assertion = command(&server, &directory)
        .args([
            "endpoints",
            "create",
            "--temporary",
            "--name",
            "integration",
            "--set-default",
            "--json",
        ])
        .assert()
        .success();

    assert_eq!(parse_stdout(&assertion), response);
    let config_path = directory.path().join("config.toml");
    let config_text = fs::read_to_string(&config_path).expect("TOML config should be written");
    let config: toml::Value = toml::from_str(&config_text).expect("config should be valid TOML");
    assert_eq!(config["default_endpoint"].as_str(), Some(ENDPOINT_ID));
    assert!(!config_text.trim_start().starts_with('{'));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn semantic_flag_errors_keep_the_reference_messages_and_avoid_api_calls() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let cases = [
        (
            vec!["endpoints", "create", "--private", "--temporary"],
            "Choose either --private or --temporary, not both.",
        ),
        (
            vec![
                "tokens",
                "create",
                "--scope",
                "events:read",
                "--preset",
                "full-access",
            ],
            "Use either --preset or --scope, not both.",
        ),
        (
            vec!["tokens", "create", "--expires-in", "30d", "--no-expiration"],
            "Use either --no-expiration or --expires-in, not both.",
        ),
        (
            vec!["events", "tail", "--endpoint", ENDPOINT_ID, "--last", "-1"],
            "--last must be a non-negative integer.",
        ),
    ];

    for (arguments, message) in cases {
        command(&server, &directory)
            .args(arguments)
            .assert()
            .failure()
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::contains(message));
    }
    assert!(
        server
            .received_requests()
            .await
            .is_none_or(|requests| requests.is_empty())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endpoint_list_show_and_delete_preserve_the_http_and_output_contract() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();
    let endpoint = endpoint_response()["endpoint"].clone();
    let list_response = json!({"endpoints": [endpoint.clone()]});

    Mock::given(method("GET"))
        .and(path("/v1/endpoints"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}")))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"endpoint": endpoint.clone()})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}")))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "endpoint": endpoint,
            "deleted_events": 3,
            "deleted_body_objects": 2
        })))
        .expect(1)
        .mount(&server)
        .await;

    let listed = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args(["endpoints", "list", "--json"])
        .assert()
        .success();
    assert_eq!(parse_stdout(&listed), list_response);

    let shown = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args(["endpoints", "show", ENDPOINT_ID, "--json"])
        .assert()
        .success();
    assert_eq!(parse_stdout(&shown), endpoint_response());

    command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args(["endpoints", "delete", ENDPOINT_ID, "--yes"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains(format!("Deleted endpoint: {ENDPOINT_ID}"))
                .and(predicate::str::contains("Deleted events: 3"))
                .and(predicate::str::contains("Deleted body objects: 2")),
        );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endpoint_secret_create_list_and_revoke_preserve_one_time_secret_boundaries() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();
    let secret_id = "sec_integration";
    let secret_value = "bst_endpoint_secret_shown_once";
    let active = json!({
        "id": secret_id,
        "endpoint_id": ENDPOINT_ID,
        "status": "active",
        "created_at": "2026-08-13T00:00:00.000Z",
        "last_used_at": null,
        "revoked_at": null
    });
    let created = json!({
        "endpoint_secret": active.clone(),
        "secret": secret_value
    });
    let listed = json!({"endpoint_secrets": [active.clone()]});
    let mut revoked = active.clone();
    revoked["status"] = json!("revoked");
    revoked["revoked_at"] = json!("2026-08-13T02:00:00.000Z");

    Mock::given(method("POST"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}/secrets")))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(created.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}/secrets")))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!(
            "/v1/endpoints/{ENDPOINT_ID}/secrets/{secret_id}"
        )))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "endpoint_secret": revoked
        })))
        .expect(1)
        .mount(&server)
        .await;

    let create = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args([
            "endpoints",
            "secrets",
            "create",
            "--endpoint",
            ENDPOINT_ID,
            "--json",
        ])
        .assert()
        .success();
    assert_eq!(parse_stdout(&create), created);

    let list = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args([
            "endpoints",
            "secrets",
            "list",
            "--endpoint",
            ENDPOINT_ID,
            "--json",
        ])
        .assert()
        .success();
    assert_eq!(parse_stdout(&list), listed);
    assert!(!String::from_utf8_lossy(&list.get_output().stdout).contains(secret_value));

    command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args([
            "endpoints",
            "secrets",
            "revoke",
            secret_id,
            "--endpoint",
            ENDPOINT_ID,
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Revoked secret: {secret_id}"
        )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_list_redacts_sensitive_headers_in_json() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();

    Mock::given(method("GET"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}/events")))
        .and(query_param("limit", "2"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "events": [{
                "id": "evt_integration",
                "endpoint_id": ENDPOINT_ID,
                "received_at": "2026-08-13T01:02:03.000Z",
                "method": "POST",
                "request_path": "/webhooks/github",
                "query": {},
                "headers": {
                    "Authorization": "server-secret",
                    "x-barestash-secret": "ingest-secret",
                    "content-type": "application/json"
                },
                "body": {
                    "size": 2,
                    "sha256": "sha256-fixture",
                    "available": true
                }
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let assertion = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args([
            "events",
            "list",
            "--endpoint",
            ENDPOINT_ID,
            "--limit",
            "2",
            "--json",
        ])
        .assert()
        .success();
    let output = parse_stdout(&assertion);
    let headers = &output["events"][0]["headers"];

    assert_eq!(headers["authorization"], "[REDACTED]");
    assert_eq!(headers["content-type"], "application/json");
    assert!(headers.get("x-barestash-secret").is_none());
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout);
    assert!(!stdout.contains("server-secret"));
    assert!(!stdout.contains("ingest-secret"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_latest_and_show_fetch_detail_and_transform_the_body_as_json() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();
    let metadata = event_metadata();
    let detail = event_detail();

    Mock::given(method("GET"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}/events")))
        .and(query_param("limit", "1"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "events": [metadata]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/events/evt_integration"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/events/evt_integration/body"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(br#"{"ok":true}"#, "application/json"),
        )
        .expect(2)
        .mount(&server)
        .await;

    let latest = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args(["events", "latest", "--endpoint", ENDPOINT_ID, "--json"])
        .assert()
        .success();
    let latest = parse_stdout(&latest);
    assert_eq!(latest["event"]["id"], "evt_integration");
    assert_eq!(
        latest["event"]["request"]["headers"]["authorization"],
        "[REDACTED]"
    );
    assert!(
        latest["event"]["request"]["headers"]
            .get("x-barestash-secret")
            .is_none()
    );
    assert_eq!(latest["body"], json!({"ok": true}));

    let shown = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args(["events", "show", "evt_integration", "--json"])
        .assert()
        .success();
    let shown_stdout = String::from_utf8_lossy(&shown.get_output().stdout);
    let shown = parse_stdout(&shown);
    assert_eq!(shown["event"]["id"], "evt_integration");
    assert_eq!(shown["body"], json!({"ok": true}));
    assert!(!shown_stdout.contains("server-secret"));
    assert!(!shown_stdout.contains("ingest-secret"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_stream_emits_only_jsonl_and_ctrl_c_exits_successfully() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();
    let sse = concat!(
        "id: evt_stream_integration\n",
        "data: {\"id\":\"evt_stream_integration\",",
        "\"endpoint_id\":\"ep_integration\",",
        "\"received_at\":\"2026-08-13T01:02:03.000Z\",",
        "\"request\":{\"method\":\"POST\",\"path\":\"/stream\",",
        "\"query\":{},\"headers\":{\"Authorization\":\"server-secret\",",
        "\"content-type\":\"application/json\"},",
        "\"body_size\":11,\"body_sha256\":\"sha256-fixture\"},",
        "\"body\":{\"encoding\":\"base64\",",
        "\"data\":\"eyJvayI6dHJ1ZX0=\"}}\n\n"
    );

    Mock::given(method("GET"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}/events/stream")))
        .and(header("authorization", format!("Bearer {bearer}")))
        .and(header("accept", "text/event-stream"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let mut process = std::process::Command::new(assert_cmd::cargo::cargo_bin("barestash"));
    process
        .env("HOME", directory.path())
        .env(
            "BARESTASH_CONFIG_FILE",
            directory.path().join("config.toml"),
        )
        .env("BARESTASH_API_URL", server.uri())
        .env("BARESTASH_ALLOW_INSECURE_API_URL", "1")
        .env("BARESTASH_TEST_KEYRING_UNAVAILABLE", "1")
        .env("BARESTASH_TOKEN", &bearer)
        .env("BARESTASH_ENDPOINT", "")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["events", "stream", "--endpoint", ENDPOINT_ID])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = process.spawn().expect("spawn events stream");

    let mut observed_request = false;
    for _ in 0..50 {
        if server
            .received_requests()
            .await
            .is_some_and(|requests| !requests.is_empty())
        {
            observed_request = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(observed_request, "stream request was not observed");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let interrupt = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT to events stream");
    assert!(interrupt.success(), "kill -INT failed: {interrupt}");
    let output = child.wait_with_output().expect("wait for events stream");
    assert!(
        output.status.success(),
        "stream did not exit successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stream stdout should be UTF-8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "stream stdout must contain JSONL only");
    let payload: Value = serde_json::from_str(lines[0]).expect("stream line should be JSON");
    assert_eq!(payload["id"], "evt_stream_integration");
    assert_eq!(payload["body"], json!({"ok": true}));
    assert_eq!(payload["request"]["headers"]["authorization"], "[REDACTED]");
    assert!(!stdout.contains("server-secret"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Barestash API host:"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_tail_suppresses_existing_events_and_ctrl_c_exits_successfully() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();

    Mock::given(method("GET"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}/events")))
        .and(query_param("limit", "1"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "events": [event_metadata()]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/endpoints/{ENDPOINT_ID}/events")))
        .and(query_param("after", "evt_integration"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"events": []})))
        .mount(&server)
        .await;

    let mut process = std::process::Command::new(assert_cmd::cargo::cargo_bin("barestash"));
    process
        .env("HOME", directory.path())
        .env(
            "BARESTASH_CONFIG_FILE",
            directory.path().join("config.toml"),
        )
        .env("BARESTASH_API_URL", server.uri())
        .env("BARESTASH_ALLOW_INSECURE_API_URL", "1")
        .env("BARESTASH_TEST_KEYRING_UNAVAILABLE", "1")
        .env("BARESTASH_TOKEN", &bearer)
        .env("BARESTASH_ENDPOINT", "")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args([
            "events",
            "tail",
            "--endpoint",
            ENDPOINT_ID,
            "--poll-interval",
            "20ms",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = process.spawn().expect("spawn events tail");

    let mut observed_poll = false;
    for _ in 0..100 {
        if server.received_requests().await.is_some_and(|requests| {
            requests.iter().any(|request| {
                request
                    .url
                    .query()
                    .is_some_and(|query| query.contains("after="))
            })
        }) {
            observed_poll = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(observed_poll, "tail polling request was not observed");

    let interrupt = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT to events tail");
    assert!(interrupt.success(), "kill -INT failed: {interrupt}");
    let output = child.wait_with_output().expect("wait for events tail");
    assert!(
        output.status.success(),
        "tail did not exit successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("tail stdout should be UTF-8");
    assert!(stdout.contains(&format!("Watching endpoint: {ENDPOINT_ID}")));
    assert!(stdout.contains("RECEIVED"));
    assert!(
        !stdout.contains("/webhooks/github"),
        "--last 0 must not print the existing cursor event"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tokens_list_sends_the_environment_bearer_and_prints_json() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();
    let response = json!({
        "tokens": [{
            "id": token_id_from_pat(),
            "name": "integration",
            "status": "active",
            "scopes": ["tokens:read"],
            "created_at": "2026-08-13T00:00:00.000Z",
            "expires_at": null,
            "last_used_at": null,
            "revoked_at": null
        }]
    });

    Mock::given(method("GET"))
        .and(path("/v1/tokens"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let assertion = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args(["tokens", "list", "--json"])
        .assert()
        .success();

    assert_eq!(parse_stdout(&assertion), response);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_create_and_revoke_preserve_scopes_expiration_and_idempotency() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();
    let created_id = "tok_created_integration";
    let created_secret = "bst_pat_created_once";
    let metadata = json!({
        "id": created_id,
        "name": "automation",
        "status": "active",
        "scopes": ["endpoints:read", "events:read", "mcp:use"],
        "created_at": "2026-08-13T00:00:00.000Z",
        "expires_at": "2026-09-12T00:00:00.000Z",
        "last_used_at": null,
        "revoked_at": null
    });
    let mut created = metadata.clone();
    created["token"] = json!(created_secret);
    let mut revoked = metadata;
    revoked["status"] = json!("revoked");
    revoked["revoked_at"] = json!("2026-08-13T02:00:00.000Z");

    Mock::given(method("GET"))
        .and(path("/v1/account"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(account_for_pat(&token_id_from_pat())),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/tokens"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .and(header_exists("idempotency-key"))
        .and(body_json(json!({
            "name": "automation",
            "scopes": ["endpoints:read", "events:read", "mcp:use"],
            "expires_in": 2_592_000
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(created.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/tokens/{created_id}")))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": revoked})))
        .expect(1)
        .mount(&server)
        .await;

    let create = command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args([
            "tokens",
            "create",
            "--name",
            "automation",
            "--preset",
            "read-only",
            "--expires-in",
            "30d",
            "--json",
        ])
        .assert()
        .success();
    assert_eq!(parse_stdout(&create), created);
    assert!(!String::from_utf8_lossy(&create.get_output().stderr).contains(created_secret));

    command(&server, &directory)
        .env("BARESTASH_TOKEN", &bearer)
        .args(["tokens", "revoke", created_id, "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Revoked token: {created_id}"
        )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pat_login_status_and_revoke_logout_share_the_plaintext_credential() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let bearer = pat();
    let token_id = token_id_from_pat();

    Mock::given(method("GET"))
        .and(path("/v1/account"))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(account_for_pat(&token_id)))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/tokens/{token_id}")))
        .and(header("authorization", format!("Bearer {bearer}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let login = command(&server, &directory)
        .args(["auth", "login", "--with-token", "--insecure-storage"])
        .write_stdin(format!("{bearer}\n"))
        .assert()
        .success();
    let login_stdout = String::from_utf8_lossy(&login.get_output().stdout);
    let login_stderr = String::from_utf8_lossy(&login.get_output().stderr);
    assert!(login_stdout.contains("Authenticated as integration@example.com"));
    assert!(!login_stdout.contains(&bearer));
    assert!(!login_stderr.contains(&bearer));

    let credential_path = directory.path().join("credentials.json");
    let stored: Value = serde_json::from_str(
        &fs::read_to_string(&credential_path).expect("plaintext credential should exist"),
    )
    .expect("plaintext credential should be JSON");
    assert_eq!(stored["type"], "personal_access_token");
    assert_eq!(stored["token"], bearer);
    assert_user_only_permissions(&credential_path);

    let status = command(&server, &directory)
        .args(["auth", "status", "--json"])
        .assert()
        .success();
    let status = parse_stdout(&status);
    assert_eq!(status["authenticated"], true);
    assert_eq!(status["account"]["id"], "acc_integration");
    assert_eq!(status["credential"]["id"], token_id);

    command(&server, &directory)
        .args(["auth", "logout", "--revoke"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Logged out."));

    let marker: Value = serde_json::from_str(
        &fs::read_to_string(&credential_path).expect("logout marker should be authoritative"),
    )
    .expect("logout marker should be JSON");
    assert_eq!(marker, json!({"version": 1, "state": "logged_out"}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_session_logout_revoke_is_idempotent_and_leaves_a_logout_marker() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let access_token = old_access_token();
    let credential_path = directory.path().join("credentials.json");
    fs::write(
        &credential_path,
        serde_json::to_vec(&json!({
            "type": "cli_session",
            "session_id": "cls_integration",
            "access_token": access_token,
            "refresh_token": old_refresh_token(),
            "access_token_expires_at": "2099-01-01T00:00:00.000Z",
            "refresh_token_expires_at": "2099-06-01T00:00:00.000Z",
            "scopes": ["events:read"]
        }))
        .expect("serialize session credential"),
    )
    .expect("seed session credential");

    Mock::given(method("POST"))
        .and(path("/v1/auth/sessions/current/revoke"))
        .and(header("authorization", format!("Bearer {access_token}")))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {
                "code": "session_revoked",
                "message": "The session is already revoked."
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    command(&server, &directory)
        .args(["auth", "logout", "--revoke"])
        .assert()
        .success()
        .stdout(predicate::eq("Logged out.\n"));
    let marker: Value = serde_json::from_str(
        &fs::read_to_string(&credential_path).expect("logout marker should exist"),
    )
    .expect("logout marker should be JSON");
    assert_eq!(marker, json!({"version": 1, "state": "logged_out"}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_login_handles_pending_and_slow_down_before_persisting_the_session() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let access_token = new_access_token();
    let refresh_token = new_refresh_token();
    let poll_count = Arc::new(AtomicUsize::new(0));

    Mock::given(method("POST"))
        .and(path("/v1/auth/device/authorizations"))
        .and(body_partial_json(json!({
            "client_name": "barestash-cli",
            "client_version": env!("CARGO_PKG_VERSION"),
            "requested_scopes": [
                "endpoints:read",
                "endpoints:write",
                "events:read",
                "tokens:read",
                "tokens:write",
                "mcp:use"
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "device-code-integration",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://example.test/device",
            "verification_uri_complete": "https://example.test/device?code=ABCD-EFGH",
            "expires_in": 30,
            "interval": 0
        })))
        .expect(1)
        .mount(&server)
        .await;

    let responder_count = poll_count.clone();
    let issued_access = access_token.clone();
    let issued_refresh = refresh_token.clone();
    Mock::given(method("POST"))
        .and(path("/v1/auth/device/token"))
        .and(body_json(json!({"device_code": "device-code-integration"})))
        .respond_with(move |_request: &wiremock::Request| {
            match responder_count.fetch_add(1, Ordering::SeqCst) {
                0 => ResponseTemplate::new(400).set_body_json(json!({
                    "error": {
                        "code": "authorization_pending",
                        "message": "Authorization is pending."
                    }
                })),
                1 => ResponseTemplate::new(400).set_body_json(json!({
                    "error": {
                        "code": "slow_down",
                        "message": "Poll more slowly."
                    }
                })),
                _ => ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": issued_access,
                    "refresh_token": issued_refresh,
                    "token_type": "Bearer",
                    "expires_in": 3600,
                    "refresh_token_expires_in": 31_536_000,
                    "scopes": [
                        "endpoints:read",
                        "endpoints:write",
                        "events:read",
                        "tokens:read",
                        "tokens:write",
                        "mcp:use"
                    ]
                })),
            }
        })
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/account"))
        .and(header("authorization", format!("Bearer {access_token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(account_for_cli("atk_device")))
        .expect(1)
        .mount(&server)
        .await;

    let login = command(&server, &directory)
        // Prevent the integration test from launching a real browser while
        // preserving the production best-effort browser-open path.
        .env("PATH", directory.path())
        .args(["auth", "login", "--insecure-storage"])
        .assert()
        .success();
    assert_eq!(poll_count.load(Ordering::SeqCst), 3);
    let stdout = String::from_utf8_lossy(&login.get_output().stdout);
    let stderr = String::from_utf8_lossy(&login.get_output().stderr);
    assert!(stdout.contains("Authenticated as integration@example.com"));
    assert!(stderr.contains("https://example.test/device"));
    assert!(stderr.contains("ABCD-EFGH"));
    assert!(!stdout.contains(&access_token));
    assert!(!stderr.contains(&access_token));
    assert!(!stdout.contains(&refresh_token));
    assert!(!stderr.contains(&refresh_token));

    let credential_path = directory.path().join("credentials.json");
    let stored: Value = serde_json::from_str(
        &fs::read_to_string(&credential_path).expect("device credential should be stored"),
    )
    .expect("device credential should be JSON");
    assert_eq!(stored["type"], "cli_session");
    assert_eq!(stored["session_id"], "cls_integration");
    assert_eq!(stored["access_token"], access_token);
    assert_eq!(stored["refresh_token"], refresh_token);
    assert_user_only_permissions(&credential_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_cli_session_is_rotated_before_account_status_request() {
    let server = MockServer::start().await;
    let directory = TempDir::new().expect("temporary directory");
    let old_access = old_access_token();
    let old_refresh = old_refresh_token();
    let fresh_access = new_access_token();
    let fresh_refresh = new_refresh_token();
    let credential_path = directory.path().join("credentials.json");
    fs::write(
        &credential_path,
        serde_json::to_vec(&json!({
            "type": "cli_session",
            "session_id": "cls_integration",
            "access_token": old_access,
            "refresh_token": old_refresh,
            "access_token_expires_at": "2020-01-01T00:00:00.000Z",
            "refresh_token_expires_at": "2027-01-01T00:00:00.000Z",
            "scopes": ["events:read"]
        }))
        .expect("serialize credential"),
    )
    .expect("seed plaintext credential");

    Mock::given(method("POST"))
        .and(path("/v1/auth/token/refresh"))
        .and(body_json(json!({
            "grant_type": "refresh_token",
            "refresh_token": old_refresh
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": fresh_access,
            "refresh_token": fresh_refresh,
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token_expires_in": 31_536_000
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/account"))
        .and(header("authorization", format!("Bearer {fresh_access}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(account_for_cli("atk_integration")))
        .expect(1)
        .mount(&server)
        .await;

    let assertion = command(&server, &directory)
        .args(["auth", "status", "--json"])
        .assert()
        .success();
    let output = parse_stdout(&assertion);
    assert_eq!(output["authenticated"], true);
    assert_eq!(output["credential"]["session_id"], "cls_integration");

    let rotated: Value = serde_json::from_str(
        &fs::read_to_string(&credential_path).expect("rotated credential should remain plaintext"),
    )
    .expect("rotated credential should be JSON");
    assert_eq!(rotated["access_token"], fresh_access);
    assert_eq!(rotated["refresh_token"], fresh_refresh);
    assert_ne!(rotated["access_token"], old_access);
    assert_ne!(rotated["refresh_token"], old_refresh);
}

#[cfg(unix)]
fn assert_user_only_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let mode = fs::metadata(path)
        .expect("credential metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[cfg(not(unix))]
fn assert_user_only_permissions(_path: &Path) {
    // Windows ACL enforcement is covered by the platform implementation and
    // cannot be represented as a Unix mode bit assertion here.
}
