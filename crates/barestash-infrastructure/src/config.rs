use std::collections::HashMap;
#[cfg(unix)]
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;
use thiserror::Error;

use barestash_domain::{CliConfig, parse_config};

#[derive(Debug, Error)]
pub enum ConfigStoreError {
    #[error("failed to serialize the Barestash config file")]
    Serialize(#[source] toml::ser::Error),
    #[error("failed to create the Barestash config directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("failed to create a temporary Barestash config file")]
    CreateTemporary(#[source] std::io::Error),
    #[error("failed to write a temporary Barestash config file")]
    WriteTemporary(#[source] std::io::Error),
    #[error("failed to restrict Barestash config file permissions")]
    Permissions(#[source] std::io::Error),
    #[error("failed to replace the Barestash config file atomically")]
    Replace(#[source] std::io::Error),
    #[error("failed to delete the Barestash config file")]
    Delete(#[source] std::io::Error),
    #[error("the Barestash config task failed")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Clone, Debug)]
pub struct FileConfigStore {
    path: PathBuf,
}

impl FileConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn from_environment(
        env: &HashMap<String, String>,
        platform: &str,
        home_directory: &Path,
    ) -> Self {
        Self::new(resolve_config_path(env, platform, home_directory))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn read(&self) -> Result<CliConfig, ConfigStoreError> {
        let path = self.path.clone();
        Ok(tokio::task::spawn_blocking(move || read_config(&path)).await?)
    }

    pub async fn write(&self, config: &CliConfig) -> Result<(), ConfigStoreError> {
        let path = self.path.clone();
        let mut serialized = toml::to_string_pretty(config).map_err(ConfigStoreError::Serialize)?;
        if !serialized.ends_with('\n') {
            serialized.push('\n');
        }
        tokio::task::spawn_blocking(move || secure_atomic_write(&path, serialized.as_bytes()))
            .await?
    }

    pub async fn delete(&self) -> Result<(), ConfigStoreError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConfigStoreError::Delete(error)),
        })
        .await?
    }
}

pub fn resolve_config_path(
    env: &HashMap<String, String>,
    platform: &str,
    home_directory: &Path,
) -> PathBuf {
    if let Some(path) = env.get("BARESTASH_CONFIG_FILE") {
        return PathBuf::from(path);
    }
    if let Some(xdg) = env.get("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(xdg).join("barestash").join("config.toml");
    }
    match platform {
        "darwin" | "macos" => home_directory
            .join("Library")
            .join("Application Support")
            .join("barestash")
            .join("config.toml"),
        "win32" | "windows" => env
            .get("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_directory.join("AppData").join("Roaming"))
            .join("barestash")
            .join("config.toml"),
        _ => home_directory
            .join(".config")
            .join("barestash")
            .join("config.toml"),
    }
}

fn read_config(path: &Path) -> CliConfig {
    // Local config is optional. Absent, unreadable, malformed, and non-table
    // TOML behaves as an empty config; valid string fields still survive an
    // unrelated field with an incompatible type.
    let text = std::fs::read_to_string(path).ok();
    parse_config(text.as_deref())
}

pub(crate) fn secure_atomic_write(path: &Path, contents: &[u8]) -> Result<(), ConfigStoreError> {
    let parent = non_empty_parent(path).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(ConfigStoreError::CreateDirectory)?;

    // NamedTempFile creates the file in the destination directory and its
    // `persist` implementation performs an atomic replacement on Unix and
    // MoveFileExW(MOVEFILE_REPLACE_EXISTING) on Windows.
    let mut temporary = NamedTempFile::new_in(parent).map_err(ConfigStoreError::CreateTemporary)?;
    // Restrict the temporary before it contains a token. This is redundant
    // with tempfile's 0600 Unix default, but keeps the security invariant
    // explicit and applies an equivalent user-only ACL on Windows.
    enforce_user_only_permissions(temporary.path())?;
    temporary
        .write_all(contents)
        .map_err(ConfigStoreError::WriteTemporary)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(ConfigStoreError::WriteTemporary)?;
    temporary
        .persist(path)
        .map_err(|error| ConfigStoreError::Replace(error.error))?;
    sync_parent_directory(path);
    Ok(())
}

fn non_empty_parent(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

fn enforce_user_only_permissions(path: &Path) -> Result<(), ConfigStoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(ConfigStoreError::Permissions)?;
    }

    #[cfg(windows)]
    {
        let username = std::env::var("USERNAME").map_err(|_| {
            ConfigStoreError::Permissions(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Unable to determine the Windows user for credential ACLs.",
            ))
        })?;
        if username.is_empty() {
            return Err(ConfigStoreError::Permissions(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Unable to determine the Windows user for credential ACLs.",
            )));
        }
        let output = std::process::Command::new("icacls.exe")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &format!("{username}:F")])
            .output()
            .map_err(ConfigStoreError::Permissions)?;
        if !output.status.success() {
            return Err(ConfigStoreError::Permissions(std::io::Error::other(
                "icacls.exe could not apply a user-only ACL",
            )));
        }
    }

    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) {
    if let Some(parent) = non_empty_parent(path)
        && let Ok(directory) = File::open(parent)
    {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_compatible_platform_paths() {
        let home = Path::new("/Users/tester");
        let mut env = HashMap::new();
        env.insert("XDG_CONFIG_HOME".into(), "/xdg".into());
        assert_eq!(
            resolve_config_path(&env, "macos", home),
            PathBuf::from("/xdg/barestash/config.toml")
        );
        env.insert(
            "BARESTASH_CONFIG_FILE".into(),
            "/override/config.conf".into(),
        );
        assert_eq!(
            resolve_config_path(&env, "windows", home),
            PathBuf::from("/override/config.conf")
        );

        assert_eq!(
            resolve_config_path(&HashMap::new(), "macos", home),
            PathBuf::from("/Users/tester/Library/Application Support/barestash/config.toml")
        );
        assert_eq!(
            resolve_config_path(&HashMap::new(), "linux", Path::new("/home/tester")),
            PathBuf::from("/home/tester/.config/barestash/config.toml")
        );
    }

    #[tokio::test]
    async fn absent_and_malformed_files_read_as_empty() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("nested/config.toml");
        let store = FileConfigStore::new(&path);
        assert_eq!(
            store.read().await.expect("absent config"),
            CliConfig::default()
        );

        std::fs::create_dir_all(path.parent().expect("config parent")).expect("config parent");
        std::fs::write(&path, "default_endpoint = [").expect("malformed config");
        assert_eq!(
            store.read().await.expect("malformed config"),
            CliConfig::default()
        );

        std::fs::write(&path, "token = 42\ndefault_endpoint = \"ep_compatible\"\n")
            .expect("partially compatible config");
        assert_eq!(
            store.read().await.expect("partially compatible config"),
            CliConfig {
                token: None,
                default_endpoint: Some("ep_compatible".into()),
            }
        );
    }

    #[tokio::test]
    async fn writes_toml_atomically_with_a_trailing_newline() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("nested/config.toml");
        let store = FileConfigStore::new(&path);
        let config = CliConfig {
            token: Some("test-token".into()),
            default_endpoint: Some("ep_test".into()),
        };
        store.write(&config).await.expect("write config");

        let text = std::fs::read_to_string(&path).expect("read config");
        assert!(text.ends_with('\n'));
        assert!(text.contains("token = \"test-token\""));
        assert!(text.contains("default_endpoint = \"ep_test\""));
        assert!(!text.trim_start().starts_with('{'));
        assert_eq!(store.read().await.expect("round trip"), config);
        let temporary_files = std::fs::read_dir(path.parent().expect("config parent"))
            .expect("config directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(temporary_files, 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writes_with_user_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.toml");
        let store = FileConfigStore::new(&path);
        store
            .write(&CliConfig::default())
            .await
            .expect("write config");
        assert_eq!(
            std::fs::metadata(path)
                .expect("config metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.toml");
        let store = FileConfigStore::new(&path);
        store
            .write(&CliConfig::default())
            .await
            .expect("write config");
        store.delete().await.expect("first delete");
        store.delete().await.expect("second delete");
        assert!(!path.exists());
    }
}
