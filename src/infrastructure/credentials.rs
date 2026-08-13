use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use thiserror::Error;

use crate::domain::StoredCredential;

use super::config::{ConfigStoreError, secure_atomic_write};

pub const CREDENTIAL_SERVICE: &str = "barestash";
pub const CREDENTIAL_ACCOUNT: &str = "default";

#[derive(Debug, Error)]
#[error("operating-system credential store error: {message}")]
pub struct KeyringBackendError {
    message: String,
}

impl KeyringBackendError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait KeyringBackend: Send + Sync + 'static {
    fn get_password(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<String>, KeyringBackendError>;
    fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeyringBackendError>;
    fn delete_password(&self, service: &str, account: &str) -> Result<bool, KeyringBackendError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemKeyring;

impl SystemKeyring {
    fn entry(service: &str, account: &str) -> Result<keyring::Entry, KeyringBackendError> {
        keyring::Entry::new(service, account)
            .map_err(|error| KeyringBackendError::new(error.to_string()))
    }
}

impl KeyringBackend for SystemKeyring {
    fn get_password(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<String>, KeyringBackendError> {
        match Self::entry(service, account)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => legacy_keytar_password(service, account),
            Err(error) => Err(KeyringBackendError::new(error.to_string())),
        }
    }

    fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeyringBackendError> {
        Self::entry(service, account)?
            .set_password(password)
            .map_err(|error| KeyringBackendError::new(error.to_string()))?;
        // A successful write migrates away from keytar's Linux-specific
        // `account` attribute so a later delete cannot resurrect the old item.
        let _ = delete_legacy_keytar_password(service, account);
        Ok(())
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<bool, KeyringBackendError> {
        let deleted_current = match Self::entry(service, account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(KeyringBackendError::new(error.to_string())),
        }?;
        // Propagate legacy cleanup failures even when the current entry was
        // deleted. The caller will then persist the authoritative logged-out
        // marker, preventing the legacy credential from becoming visible on a
        // subsequent read.
        let deleted_legacy = delete_legacy_keytar_password(service, account)?;
        Ok(deleted_current || deleted_legacy)
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn legacy_keytar_password(
    service: &str,
    account: &str,
) -> Result<Option<String>, KeyringBackendError> {
    use std::collections::HashMap;

    use secret_service::{EncryptionType, blocking::SecretService};

    let secret_service = SecretService::connect(EncryptionType::Dh)
        .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    let mut result = secret_service
        .search_items(HashMap::from([("service", service), ("account", account)]))
        .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    if !result.locked.is_empty() {
        secret_service
            .unlock_all(&result.locked.iter().collect::<Vec<_>>())
            .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    }
    let Some(item) = result.unlocked.pop().or_else(|| result.locked.pop()) else {
        return Ok(None);
    };
    let secret = item
        .get_secret()
        .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    String::from_utf8(secret)
        .map(Some)
        .map_err(|error| KeyringBackendError::new(error.to_string()))
}

#[cfg(windows)]
fn legacy_keytar_password(
    service: &str,
    account: &str,
) -> Result<Option<String>, KeyringBackendError> {
    match legacy_keytar_windows_entry(service, account)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(error) => Err(KeyringBackendError::new(error.to_string())),
    }
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
fn legacy_keytar_password(
    _service: &str,
    _account: &str,
) -> Result<Option<String>, KeyringBackendError> {
    Ok(None)
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn delete_legacy_keytar_password(
    service: &str,
    account: &str,
) -> Result<bool, KeyringBackendError> {
    use std::collections::HashMap;

    use secret_service::{EncryptionType, blocking::SecretService};

    let secret_service = SecretService::connect(EncryptionType::Dh)
        .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    let result = secret_service
        .search_items(HashMap::from([("service", service), ("account", account)]))
        .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    let mut items = result.unlocked;
    if !result.locked.is_empty() {
        secret_service
            .unlock_all(&result.locked.iter().collect::<Vec<_>>())
            .map_err(|error| KeyringBackendError::new(error.to_string()))?;
        items.extend(result.locked);
    }
    let deleted = !items.is_empty();
    for item in items {
        item.delete()
            .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    }
    Ok(deleted)
}

#[cfg(windows)]
fn delete_legacy_keytar_password(
    service: &str,
    account: &str,
) -> Result<bool, KeyringBackendError> {
    match legacy_keytar_windows_entry(service, account)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring_core::Error::NoEntry) => Ok(false),
        Err(error) => Err(KeyringBackendError::new(error.to_string())),
    }
}

#[cfg(windows)]
fn legacy_keytar_windows_entry(
    service: &str,
    account: &str,
) -> Result<keyring_core::Entry, KeyringBackendError> {
    use std::collections::HashMap;

    use keyring_core::api::CredentialStoreApi;

    // keytar used `service/account` as the Windows Credential Manager target,
    // while keyring's native backend defaults to `account.service`.
    let target = format!("{service}/{account}");
    let modifiers = HashMap::from([("target", target.as_str())]);
    let store = windows_native_keyring_store::Store::new()
        .map_err(|error| KeyringBackendError::new(error.to_string()))?;
    store
        .build(service, account, Some(&modifiers))
        .map_err(|error| KeyringBackendError::new(error.to_string()))
}

#[cfg(not(any(
    windows,
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
fn delete_legacy_keytar_password(
    _service: &str,
    _account: &str,
) -> Result<bool, KeyringBackendError> {
    Ok(false)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CredentialWriteResult {
    Keyring,
    Plaintext { path: PathBuf, fallback: bool },
}

impl CredentialWriteResult {
    pub fn is_plaintext(&self) -> bool {
        matches!(self, Self::Plaintext { .. })
    }
}

#[derive(Debug, Error)]
pub enum CredentialStoreError {
    #[error("failed to read the plaintext credential file")]
    ReadPlaintext(#[source] std::io::Error),
    #[error("failed to serialize the stored credential")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write the plaintext credential file securely")]
    WritePlaintext(#[source] ConfigStoreError),
    #[error("failed to delete the plaintext credential file")]
    DeletePlaintext(#[source] std::io::Error),
    #[error("the credential store task failed")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Clone)]
pub struct CredentialStore {
    keyring: Arc<dyn KeyringBackend>,
    plaintext_path: PathBuf,
}

impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialStore")
            .field("plaintext_path", &self.plaintext_path)
            .finish_non_exhaustive()
    }
}

impl CredentialStore {
    pub fn new(keyring: Arc<dyn KeyringBackend>, plaintext_path: impl Into<PathBuf>) -> Self {
        Self {
            keyring,
            plaintext_path: plaintext_path.into(),
        }
    }

    pub fn system(plaintext_path: impl Into<PathBuf>) -> Self {
        // Integration tests execute the real binary, where a conventional
        // mock cannot be injected through `AppContext`. Keep this hook both
        // conspicuously test-named and absent from optimized release builds so
        // production users can never disable the OS credential store through
        // ambient environment state.
        #[cfg(debug_assertions)]
        if std::env::var_os("BARESTASH_TEST_KEYRING_UNAVAILABLE").is_some() {
            return Self::new(Arc::new(UnavailableTestKeyring), plaintext_path);
        }
        Self::new(Arc::new(SystemKeyring), plaintext_path)
    }

    pub fn plaintext_path(&self) -> &Path {
        &self.plaintext_path
    }

    pub async fn read(&self) -> Result<Option<StoredCredential>, CredentialStoreError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.read_blocking()).await?
    }

    pub async fn write(
        &self,
        credential: &StoredCredential,
        insecure: bool,
    ) -> Result<CredentialWriteResult, CredentialStoreError> {
        let serialized =
            serde_json::to_string(credential).map_err(CredentialStoreError::Serialize)?;
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.write_serialized(&serialized, insecure)).await?
    }

    pub async fn replace(
        &self,
        credential: &StoredCredential,
    ) -> Result<CredentialWriteResult, CredentialStoreError> {
        let serialized =
            serde_json::to_string(credential).map_err(CredentialStoreError::Serialize)?;
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.replace_serialized(&serialized)).await?
    }

    pub async fn delete(&self) -> Result<(), CredentialStoreError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.delete_blocking()).await?
    }

    fn read_blocking(&self) -> Result<Option<StoredCredential>, CredentialStoreError> {
        let plaintext = read_optional_file(&self.plaintext_path)?;
        if plaintext.as_deref().is_some_and(is_logged_out_marker) {
            return Ok(None);
        }
        if let Some(credential) = plaintext.as_deref().and_then(parse_credential) {
            return Ok(Some(credential));
        }

        // Read failures from an optional OS keyring are treated as unavailable;
        // writes surface this through an explicit plaintext-fallback result.
        let keyring_value = self
            .keyring
            .get_password(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT)
            .ok()
            .flatten();
        Ok(keyring_value.as_deref().and_then(parse_credential))
    }

    fn write_serialized(
        &self,
        serialized: &str,
        insecure: bool,
    ) -> Result<CredentialWriteResult, CredentialStoreError> {
        if insecure {
            self.write_plaintext(serialized)?;
            let _ = self
                .keyring
                .delete_password(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT);
            return Ok(CredentialWriteResult::Plaintext {
                path: self.plaintext_path.clone(),
                fallback: false,
            });
        }

        let plaintext = read_optional_file(&self.plaintext_path)?;
        let plaintext_is_authoritative = plaintext
            .as_deref()
            .is_some_and(|value| parse_credential(value).is_some() || is_logged_out_marker(value));
        if plaintext_is_authoritative {
            self.write_plaintext(serialized)?;
        }

        if self
            .keyring
            .set_password(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT, serialized)
            .is_err()
        {
            if !plaintext_is_authoritative {
                self.write_plaintext(serialized)?;
            }
            return Ok(CredentialWriteResult::Plaintext {
                path: self.plaintext_path.clone(),
                fallback: true,
            });
        }

        if plaintext_is_authoritative && remove_optional_file(&self.plaintext_path).is_err() {
            return Ok(CredentialWriteResult::Plaintext {
                path: self.plaintext_path.clone(),
                fallback: true,
            });
        }
        Ok(CredentialWriteResult::Keyring)
    }

    fn replace_serialized(
        &self,
        serialized: &str,
    ) -> Result<CredentialWriteResult, CredentialStoreError> {
        let plaintext = read_optional_file(&self.plaintext_path)?;
        let plaintext_is_authoritative = plaintext
            .as_deref()
            .is_some_and(|value| parse_credential(value).is_some() || is_logged_out_marker(value));
        if plaintext_is_authoritative {
            self.write_plaintext(serialized)?;
            return Ok(CredentialWriteResult::Plaintext {
                path: self.plaintext_path.clone(),
                fallback: false,
            });
        }

        if self
            .keyring
            .set_password(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT, serialized)
            .is_ok()
        {
            return Ok(CredentialWriteResult::Keyring);
        }

        self.write_plaintext(serialized)?;
        Ok(CredentialWriteResult::Plaintext {
            path: self.plaintext_path.clone(),
            fallback: true,
        })
    }

    fn delete_blocking(&self) -> Result<(), CredentialStoreError> {
        if self
            .keyring
            .delete_password(CREDENTIAL_SERVICE, CREDENTIAL_ACCOUNT)
            .is_err()
        {
            return self.write_logout_marker();
        }

        if remove_optional_file(&self.plaintext_path).is_err() {
            // Mask a stale plaintext secret even when removal is unavailable.
            self.write_logout_marker()?;
        }
        Ok(())
    }

    fn write_plaintext(&self, serialized: &str) -> Result<(), CredentialStoreError> {
        let mut contents = String::with_capacity(serialized.len() + 1);
        contents.push_str(serialized);
        contents.push('\n');
        secure_atomic_write(&self.plaintext_path, contents.as_bytes())
            .map_err(CredentialStoreError::WritePlaintext)
    }

    fn write_logout_marker(&self) -> Result<(), CredentialStoreError> {
        self.write_plaintext(&json!({ "version": 1, "state": "logged_out" }).to_string())
    }
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug)]
struct UnavailableTestKeyring;

#[cfg(debug_assertions)]
impl KeyringBackend for UnavailableTestKeyring {
    fn get_password(
        &self,
        _service: &str,
        _account: &str,
    ) -> Result<Option<String>, KeyringBackendError> {
        Err(KeyringBackendError::new("unavailable in integration test"))
    }

    fn set_password(
        &self,
        _service: &str,
        _account: &str,
        _password: &str,
    ) -> Result<(), KeyringBackendError> {
        Err(KeyringBackendError::new("unavailable in integration test"))
    }

    fn delete_password(&self, _service: &str, _account: &str) -> Result<bool, KeyringBackendError> {
        Err(KeyringBackendError::new("unavailable in integration test"))
    }
}

fn parse_credential(value: &str) -> Option<StoredCredential> {
    serde_json::from_str(value).ok()
}

fn is_logged_out_marker(value: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(value) else {
        return false;
    };
    value.get("version").and_then(serde_json::Value::as_u64) == Some(1)
        && value.get("state").and_then(serde_json::Value::as_str) == Some("logged_out")
}

fn read_optional_file(path: &Path) -> Result<Option<String>, CredentialStoreError> {
    match std::fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CredentialStoreError::ReadPlaintext(error)),
    }
}

fn remove_optional_file(path: &Path) -> Result<(), std::io::Error> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct MemoryKeyring {
        value: Mutex<Option<String>>,
        fail_get: AtomicBool,
        fail_set: AtomicBool,
        fail_delete: AtomicBool,
        get_calls: AtomicUsize,
    }

    impl KeyringBackend for MemoryKeyring {
        fn get_password(
            &self,
            service: &str,
            account: &str,
        ) -> Result<Option<String>, KeyringBackendError> {
            assert_eq!(service, CREDENTIAL_SERVICE);
            assert_eq!(account, CREDENTIAL_ACCOUNT);
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_get.load(Ordering::SeqCst) {
                return Err(KeyringBackendError::new("unavailable"));
            }
            Ok(self.value.lock().expect("keyring lock").clone())
        }

        fn set_password(
            &self,
            service: &str,
            account: &str,
            password: &str,
        ) -> Result<(), KeyringBackendError> {
            assert_eq!(service, CREDENTIAL_SERVICE);
            assert_eq!(account, CREDENTIAL_ACCOUNT);
            if self.fail_set.load(Ordering::SeqCst) {
                return Err(KeyringBackendError::new("unavailable"));
            }
            *self.value.lock().expect("keyring lock") = Some(password.to_owned());
            Ok(())
        }

        fn delete_password(
            &self,
            service: &str,
            account: &str,
        ) -> Result<bool, KeyringBackendError> {
            assert_eq!(service, CREDENTIAL_SERVICE);
            assert_eq!(account, CREDENTIAL_ACCOUNT);
            if self.fail_delete.load(Ordering::SeqCst) {
                return Err(KeyringBackendError::new("unavailable"));
            }
            Ok(self.value.lock().expect("keyring lock").take().is_some())
        }
    }

    fn pat(token: &str) -> StoredCredential {
        StoredCredential::PersonalAccessToken {
            token: token.to_owned(),
        }
    }

    #[tokio::test]
    async fn uses_the_keyring_by_default() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keyring = Arc::new(MemoryKeyring::default());
        let store = CredentialStore::new(keyring, directory.path().join("credentials.json"));
        assert_eq!(
            store.write(&pat("secret"), false).await.expect("write"),
            CredentialWriteResult::Keyring
        );
        assert_eq!(store.read().await.expect("read"), Some(pat("secret")));
    }

    #[tokio::test]
    async fn keyring_failure_falls_back_to_plaintext() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keyring = Arc::new(MemoryKeyring::default());
        keyring.fail_set.store(true, Ordering::SeqCst);
        let path = directory.path().join("credentials.json");
        let store = CredentialStore::new(keyring, &path);
        assert_eq!(
            store.write(&pat("secret"), false).await.expect("write"),
            CredentialWriteResult::Plaintext {
                path: path.clone(),
                fallback: true,
            }
        );
        assert_eq!(store.read().await.expect("read"), Some(pat("secret")));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path)
                    .expect("credential metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn valid_plaintext_is_authoritative_over_a_stale_keyring() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keyring = Arc::new(MemoryKeyring::default());
        *keyring.value.lock().expect("keyring lock") =
            Some(serde_json::to_string(&pat("stale")).expect("serialize"));
        let path = directory.path().join("credentials.json");
        secure_atomic_write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&pat("current")).expect("serialize")
            )
            .as_bytes(),
        )
        .expect("plaintext credential");
        let store = CredentialStore::new(Arc::clone(&keyring) as Arc<dyn KeyringBackend>, &path);

        assert_eq!(store.read().await.expect("read"), Some(pat("current")));
        assert_eq!(keyring.get_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn malformed_plaintext_falls_through_to_the_keyring() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keyring = Arc::new(MemoryKeyring::default());
        *keyring.value.lock().expect("keyring lock") =
            Some(serde_json::to_string(&pat("keyring")).expect("serialize"));
        let path = directory.path().join("credentials.json");
        std::fs::write(&path, "{").expect("invalid plaintext");
        let store = CredentialStore::new(Arc::clone(&keyring) as Arc<dyn KeyringBackend>, &path);

        assert_eq!(store.read().await.expect("read"), Some(pat("keyring")));
        assert_eq!(keyring.get_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn explicit_plaintext_storage_masks_the_keyring() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keyring = Arc::new(MemoryKeyring::default());
        *keyring.value.lock().expect("keyring lock") =
            Some(serde_json::to_string(&pat("stale")).expect("serialize"));
        let path = directory.path().join("credentials.json");
        let store = CredentialStore::new(keyring, &path);
        assert_eq!(
            store.write(&pat("plaintext"), true).await.expect("write"),
            CredentialWriteResult::Plaintext {
                path,
                fallback: false,
            }
        );
        assert_eq!(store.read().await.expect("read"), Some(pat("plaintext")));
    }

    #[tokio::test]
    async fn logout_marker_masks_a_keyring_that_cannot_be_deleted() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keyring = Arc::new(MemoryKeyring::default());
        *keyring.value.lock().expect("keyring lock") =
            Some(serde_json::to_string(&pat("stale")).expect("serialize"));
        keyring.fail_delete.store(true, Ordering::SeqCst);
        let path = directory.path().join("credentials.json");
        let store = CredentialStore::new(Arc::clone(&keyring) as Arc<dyn KeyringBackend>, &path);

        store.delete().await.expect("delete");
        let text = std::fs::read_to_string(path).expect("logout marker");
        assert!(!text.contains("stale"));
        assert!(is_logged_out_marker(&text));
        assert_eq!(store.read().await.expect("read"), None);
        assert_eq!(keyring.get_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn refresh_replacement_preserves_plaintext_backend() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let keyring = Arc::new(MemoryKeyring::default());
        let path = directory.path().join("credentials.json");
        let store = CredentialStore::new(keyring, &path);
        store.write(&pat("old"), true).await.expect("initial write");
        assert_eq!(
            store.replace(&pat("rotated")).await.expect("replace"),
            CredentialWriteResult::Plaintext {
                path,
                fallback: false,
            }
        );
        assert_eq!(store.read().await.expect("read"), Some(pat("rotated")));
    }
}
