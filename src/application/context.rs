use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::application::CliError;
use crate::domain::StoredCredential;
use crate::infrastructure::api::{ApiClient, ApiUrlPolicy};
use crate::infrastructure::config::FileConfigStore;
use crate::infrastructure::credentials::CredentialStore;
use crate::infrastructure::lock::FileLock;

pub const DEFAULT_API_URL: &str = "http://localhost:8787";

pub struct AppContext {
    pub env: HashMap<String, String>,
    pub api: ApiClient,
    pub config: FileConfigStore,
    pub credentials: Arc<CredentialStore>,
    pub credential_lock: FileLock,
}

impl AppContext {
    pub fn from_environment(allow_insecure_flag: bool) -> Result<Self, CliError> {
        let env: HashMap<String, String> = env::vars().collect();
        let home = home_directory(&env)
            .or_else(|| env.get("BARESTASH_CONFIG_FILE").map(|_| PathBuf::new()))
            .ok_or_else(|| {
                CliError::Infrastructure("Unable to determine the home directory.".into())
            })?;
        let config = FileConfigStore::from_environment(&env, env::consts::OS, &home);
        let config_directory = config
            .path()
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        let allow_insecure = allow_insecure_flag
            || matches!(
                env.get("BARESTASH_ALLOW_INSECURE_API_URL")
                    .map(String::as_str),
                Some("1" | "true")
            );
        let api_url = env
            .get("BARESTASH_API_URL")
            .map(String::as_str)
            .unwrap_or(DEFAULT_API_URL);
        let api = ApiClient::new_deferred(api_url, ApiUrlPolicy { allow_insecure })
            .map_err(|error| CliError::Infrastructure(error.to_string()))?
            .with_host_diagnostic(true);

        Ok(Self {
            env,
            api,
            config,
            credentials: Arc::new(CredentialStore::system(
                config_directory.join("credentials.json"),
            )),
            credential_lock: FileLock::new(config_directory.join("credentials.lock")),
        })
    }

    pub async fn selected_endpoint(&self, explicit: Option<&str>) -> Result<String, CliError> {
        if let Some(value) = explicit {
            return Ok(value.to_owned());
        }
        if let Some(value) = self.env.get("BARESTASH_ENDPOINT") {
            return Ok(value.clone());
        }
        let _guard = self
            .credential_lock
            .acquire()
            .await
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        self.config
            .read()
            .await
            .map_err(|error| CliError::Infrastructure(error.to_string()))?
            .default_endpoint
            .ok_or_else(no_endpoint_selected)
    }

    pub async fn stored_credential(&self) -> Result<Option<StoredCredential>, CliError> {
        let stored = self
            .credentials
            .read()
            .await
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        if stored.is_some() {
            return Ok(stored);
        }
        let config = self
            .config
            .read()
            .await
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        Ok(config
            .token
            .map(|token| StoredCredential::PersonalAccessToken { token }))
    }

    pub fn environment_token(&self) -> Option<&str> {
        self.env.get("BARESTASH_TOKEN").and_then(|value| {
            if value.is_empty() {
                None
            } else {
                Some(value.as_str())
            }
        })
    }

    pub fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

fn home_directory(env: &HashMap<String, String>) -> Option<PathBuf> {
    #[cfg(windows)]
    let value = env.get("USERPROFILE").or_else(|| env.get("HOME"));
    #[cfg(not(windows))]
    let value = env.get("HOME");

    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(home::home_dir)
}

fn no_endpoint_selected() -> CliError {
    CliError::Local(
        "No endpoint selected.\n\nRun:\n  barestash endpoints create --set-default\n\nOr specify:\n  --endpoint ep_abc123"
            .into(),
    )
}
