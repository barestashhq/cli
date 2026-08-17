use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wiremock::MockServer;

use barestash_client::{ApiClient, ApiUrlPolicy};
use barestash_local_state::StoredCredential;
use barestash_local_state::config::FileConfigStore;
use barestash_local_state::credentials::{CredentialStore, KeyringBackend, KeyringBackendError};
use barestash_local_state::lock::FileLock;
use barestash_protocol::AuthorizationScope;

use crate::AppContext;

pub(in crate::auth) fn session(session_id: &str, access_token: &str) -> StoredCredential {
    StoredCredential::CliSession {
        session_id: session_id.into(),
        access_token: access_token.into(),
        refresh_token: "refresh".into(),
        access_token_expires_at: "2026-08-13T01:00:00.000Z".into(),
        refresh_token_expires_at: "2026-11-13T00:00:00.000Z".into(),
        scopes: vec![AuthorizationScope::EventsRead.to_string()],
    }
}

#[derive(Default)]
struct TestKeyring {
    value: Mutex<Option<String>>,
}

impl KeyringBackend for TestKeyring {
    fn get_password(
        &self,
        _service: &str,
        _account: &str,
    ) -> Result<Option<String>, KeyringBackendError> {
        self.value
            .lock()
            .map(|value| value.clone())
            .map_err(|error| KeyringBackendError::new(error.to_string()))
    }

    fn set_password(
        &self,
        _service: &str,
        _account: &str,
        password: &str,
    ) -> Result<(), KeyringBackendError> {
        *self
            .value
            .lock()
            .map_err(|error| KeyringBackendError::new(error.to_string()))? =
            Some(password.to_owned());
        Ok(())
    }

    fn delete_password(&self, _service: &str, _account: &str) -> Result<bool, KeyringBackendError> {
        Ok(self
            .value
            .lock()
            .map_err(|error| KeyringBackendError::new(error.to_string()))?
            .take()
            .is_some())
    }
}

pub(in crate::auth) fn test_context(
    server: &MockServer,
    credential: Option<&StoredCredential>,
    environment_token: Option<&str>,
) -> (tempfile::TempDir, AppContext) {
    let directory = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
    let keyring = Arc::new(TestKeyring::default());
    if let Some(credential) = credential {
        *keyring
            .value
            .lock()
            .unwrap_or_else(|error| panic!("keyring lock: {error}")) = Some(
            serde_json::to_string(credential)
                .unwrap_or_else(|error| panic!("credential serialization: {error}")),
        );
    }
    let credentials = CredentialStore::new(
        Arc::clone(&keyring) as Arc<dyn KeyringBackend>,
        directory.path().join("credentials.json"),
    );
    let mut env = HashMap::new();
    if let Some(token) = environment_token {
        env.insert("BARESTASH_TOKEN".into(), token.into());
    }
    let context = AppContext {
        env,
        api: ApiClient::new(&server.uri(), ApiUrlPolicy::default())
            .unwrap_or_else(|error| panic!("API client: {error}")),
        api_host_logged: std::sync::atomic::AtomicBool::new(true),
        config: FileConfigStore::new(directory.path().join("config.toml")),
        credentials: Arc::new(credentials),
        credential_lock: FileLock::new(directory.path().join("credentials.lock")),
    };
    (directory, context)
}
