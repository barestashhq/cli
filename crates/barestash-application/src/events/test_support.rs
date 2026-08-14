use std::collections::HashMap;
use std::sync::Arc;

use barestash_client::{ApiClient, ApiUrlPolicy};
use barestash_infrastructure::config::FileConfigStore;
use barestash_infrastructure::credentials::CredentialStore;
use barestash_infrastructure::lock::FileLock;
use barestash_protocol::{EventBodyMetadata, EventMetadata, HeaderMap, QueryParameters};
use tempfile::TempDir;
use wiremock::MockServer;

use crate::AppContext;

pub(super) fn api(server: &MockServer) -> ApiClient {
    ApiClient::new(
        &server.uri(),
        ApiUrlPolicy {
            allow_insecure: true,
        },
    )
    .unwrap_or_else(|error| panic!("mock API URL is valid: {error}"))
}

pub(super) fn context(server: &MockServer) -> (AppContext, TempDir) {
    let directory = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("temporary directory is available: {error}"));
    let config_path = directory.path().join("config.toml");
    let context = AppContext {
        env: HashMap::from([("BARESTASH_TOKEN".to_owned(), "test-token".to_owned())]),
        api: api(server),
        api_host_logged: std::sync::atomic::AtomicBool::new(true),
        config: FileConfigStore::new(&config_path),
        credentials: Arc::new(CredentialStore::system(
            directory.path().join("credentials.json"),
        )),
        credential_lock: FileLock::new(directory.path().join("credentials.lock")),
    };
    (context, directory)
}

pub(super) fn event(id: &str) -> EventMetadata {
    EventMetadata {
        id: id.to_owned(),
        endpoint_id: "ep_test".to_owned(),
        received_at: "2026-07-05T12:04:32.000Z".to_owned(),
        method: "POST".to_owned(),
        request_path: "/webhook".to_owned(),
        query: QueryParameters::new(),
        headers: HeaderMap::from([("content-type".to_owned(), "application/json".to_owned())]),
        body: EventBodyMetadata {
            size: 2,
            sha256: "hash".to_owned(),
            available: true,
            url: None,
        },
    }
}
