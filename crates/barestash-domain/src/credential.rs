use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoredCredential {
    PersonalAccessToken {
        token: String,
    },
    CliSession {
        session_id: String,
        access_token: String,
        refresh_token: String,
        access_token_expires_at: String,
        refresh_token_expires_at: String,
        // Stored sessions accept future server scopes as opaque strings. The
        // reference credential parser required strings, but intentionally did
        // not reject scopes introduced after this CLI version.
        scopes: Vec<String>,
    },
}

pub fn parse_stored_credential(value: Option<&str>) -> Option<StoredCredential> {
    value.and_then(|value| serde_json::from_str(value).ok())
}

pub fn serialize_stored_credential(
    credential: &StoredCredential,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(credential)
}

#[cfg(test)]
mod tests {
    use super::*;
    use barestash_protocol::AuthorizationScope;

    #[test]
    fn stored_credentials_round_trip_and_invalid_storage_is_empty() {
        let credential = StoredCredential::CliSession {
            session_id: "cls_test".to_owned(),
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            access_token_expires_at: "2026-07-12T13:00:00.000Z".to_owned(),
            refresh_token_expires_at: "2026-10-12T13:00:00.000Z".to_owned(),
            scopes: vec![AuthorizationScope::EventsRead.to_string()],
        };
        let serialized = serialize_stored_credential(&credential)
            .unwrap_or_else(|error| panic!("credential should serialize: {error}"));

        assert_eq!(parse_stored_credential(Some(&serialized)), Some(credential));
        assert_eq!(parse_stored_credential(Some("not-json")), None);
        assert_eq!(parse_stored_credential(None), None);
    }
}
