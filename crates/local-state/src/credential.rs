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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stored_credentials_round_trip() {
        let credential = StoredCredential::CliSession {
            session_id: "cls_test".to_owned(),
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            access_token_expires_at: "2026-07-12T13:00:00.000Z".to_owned(),
            refresh_token_expires_at: "2026-10-12T13:00:00.000Z".to_owned(),
            scopes: vec!["events:read".to_owned()],
        };
        let serialized = serde_json::to_string(&credential)
            .unwrap_or_else(|error| panic!("credential should serialize: {error}"));

        let parsed = serde_json::from_str::<StoredCredential>(&serialized)
            .unwrap_or_else(|error| panic!("credential should deserialize: {error}"));
        assert_eq!(parsed, credential);
    }
}
