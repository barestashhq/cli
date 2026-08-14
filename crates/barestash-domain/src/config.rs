use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_endpoint: Option<String>,
}

pub fn resolve_config_path(
    env: &BTreeMap<String, String>,
    platform_name: &str,
    home_directory: &str,
) -> String {
    if let Some(config_file) = env.get("BARESTASH_CONFIG_FILE") {
        return config_file.clone();
    }

    if let Some(xdg_config_home) = env.get("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return format!("{xdg_config_home}/barestash/config.toml");
    }

    match platform_name {
        "darwin" => format!("{home_directory}/Library/Application Support/barestash/config.toml"),
        "win32" => {
            let app_data = env
                .get("APPDATA")
                .cloned()
                .unwrap_or_else(|| format!("{home_directory}/AppData/Roaming"));
            format!("{app_data}/barestash/config.toml")
        }
        _ => format!("{home_directory}/.config/barestash/config.toml"),
    }
}

pub fn parse_config(text: Option<&str>) -> CliConfig {
    let Some(text) = text.filter(|value| !value.trim().is_empty()) else {
        return CliConfig::default();
    };
    let Ok(toml::Value::Table(table)) = toml::from_str::<toml::Value>(text) else {
        return CliConfig::default();
    };

    CliConfig {
        token: table
            .get("token")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
        default_endpoint: table
            .get("default_endpoint")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
    }
}

pub fn serialize_config(config: &CliConfig) -> String {
    let mut serialized = toml::to_string_pretty(config).unwrap_or_default();
    if !serialized.ends_with('\n') {
        serialized.push('\n');
    }
    serialized
}

pub fn select_endpoint_id(
    endpoint_flag: Option<&str>,
    environment_endpoint: Option<&str>,
    configured_endpoint: Option<&str>,
) -> Option<String> {
    endpoint_flag
        .or(environment_endpoint)
        .or(configured_endpoint)
        .map(str::to_owned)
}

pub fn selected_endpoint_id(
    endpoint_flag: Option<&str>,
    env: &BTreeMap<String, String>,
    config: &CliConfig,
) -> Option<String> {
    select_endpoint_id(
        endpoint_flag,
        env.get("BARESTASH_ENDPOINT").map(String::as_str),
        config.default_endpoint.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_toml_config_paths_with_environment_precedence() {
        let env = BTreeMap::from([
            (
                "BARESTASH_CONFIG_FILE".to_owned(),
                "/override/barestash.conf".to_owned(),
            ),
            ("XDG_CONFIG_HOME".to_owned(), "/xdg".to_owned()),
            ("APPDATA".to_owned(), "C:/AppData".to_owned()),
        ]);
        assert_eq!(
            resolve_config_path(&env, "win32", "/home/tester"),
            "/override/barestash.conf"
        );
        assert_eq!(
            resolve_config_path(
                &BTreeMap::from([("XDG_CONFIG_HOME".to_owned(), "/xdg".to_owned())]),
                "darwin",
                "/Users/tester"
            ),
            "/xdg/barestash/config.toml"
        );
        assert_eq!(
            resolve_config_path(&BTreeMap::new(), "darwin", "/Users/tester"),
            "/Users/tester/Library/Application Support/barestash/config.toml"
        );
        assert_eq!(
            resolve_config_path(&BTreeMap::new(), "linux", "/home/tester"),
            "/home/tester/.config/barestash/config.toml"
        );
        assert_eq!(
            resolve_config_path(&BTreeMap::new(), "win32", "C:/Users/tester"),
            "C:/Users/tester/AppData/Roaming/barestash/config.toml"
        );
    }

    #[test]
    fn parses_and_serializes_config_without_exposing_invalid_values() {
        for invalid in [None, Some(""), Some("{"), Some("null"), Some("\"text\"")] {
            assert_eq!(parse_config(invalid), CliConfig::default());
        }

        let config = CliConfig {
            token: Some("test-token".to_owned()),
            default_endpoint: Some("ep_test".to_owned()),
        };
        let serialized = serialize_config(&config);
        assert!(serialized.ends_with('\n'));
        assert!(serialized.contains("token = \"test-token\""));
        assert!(serialized.contains("default_endpoint = \"ep_test\""));
        assert!(!serialized.trim_start().starts_with('{'));
        assert_eq!(parse_config(Some(&serialized)), config);
    }

    #[test]
    fn endpoint_selection_prefers_flag_then_environment_then_config() {
        assert_eq!(
            select_endpoint_id(Some("ep_flag"), Some("ep_env"), Some("ep_config")),
            Some("ep_flag".to_owned())
        );
        assert_eq!(
            select_endpoint_id(None, Some("ep_env"), Some("ep_config")),
            Some("ep_env".to_owned())
        );
        assert_eq!(
            select_endpoint_id(None, None, Some("ep_config")),
            Some("ep_config".to_owned())
        );
        assert_eq!(select_endpoint_id(None, None, None), None);
        assert_eq!(
            select_endpoint_id(Some(""), Some("ep_env"), Some("ep_config")),
            Some(String::new())
        );
    }
}
