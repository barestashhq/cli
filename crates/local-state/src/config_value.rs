use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_endpoint: Option<String>,
}

pub(crate) fn parse_config(text: Option<&str>) -> CliConfig {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_config_is_empty_and_valid_string_fields_survive() {
        for invalid in [None, Some(""), Some("{"), Some("null"), Some("\"text\"")] {
            assert_eq!(parse_config(invalid), CliConfig::default());
        }
        assert_eq!(
            parse_config(Some("token = 42\ndefault_endpoint = \"ep_test\"")),
            CliConfig {
                token: None,
                default_endpoint: Some("ep_test".to_owned()),
            }
        );
    }
}
