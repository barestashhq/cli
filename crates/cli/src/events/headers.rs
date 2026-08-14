use barestash_protocol::HeaderMap;

pub const REDACTED_HEADER_VALUE: &str = "[REDACTED]";

const REMOVED_HEADER_NAMES: [&str; 2] = ["x-barestash-secret", "x-barestash-bootstrap-token"];

const REDACTED_HEADER_NAMES: [&str; 12] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
    "stripe-signature",
    "x-hub-signature",
    "x-hub-signature-256",
    "x-slack-signature",
    "x-shopify-hmac-sha256",
];

/// Lowercases header names, removes Barestash credentials, and redacts other
/// known authentication/signature headers.
pub fn redact_headers_for_display(headers: &HeaderMap) -> HeaderMap {
    let mut display_headers = HeaderMap::new();

    for (raw_name, value) in headers {
        let name = raw_name.to_ascii_lowercase();

        if REMOVED_HEADER_NAMES.contains(&name.as_str()) {
            continue;
        }

        let display_value = if REDACTED_HEADER_NAMES.contains(&name.as_str()) {
            REDACTED_HEADER_VALUE.to_owned()
        } else {
            value.clone()
        };
        display_headers.insert(name, display_value);
    }

    display_headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_sensitive_headers_and_removes_barestash_credentials() {
        let headers = HeaderMap::from([
            ("Authorization".to_owned(), "Bearer raw-token".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Stripe-Signature".to_owned(), "t=raw,v1=raw".to_owned()),
            (
                "X-Barestash-Bootstrap-Token".to_owned(),
                "bootstrap-secret".to_owned(),
            ),
            (
                "X-Barestash-Secret".to_owned(),
                "endpoint-secret".to_owned(),
            ),
            ("X-Custom".to_owned(), "safe for display".to_owned()),
        ]);

        assert_eq!(
            redact_headers_for_display(&headers),
            HeaderMap::from([
                ("authorization".to_owned(), REDACTED_HEADER_VALUE.to_owned()),
                ("content-type".to_owned(), "application/json".to_owned()),
                (
                    "stripe-signature".to_owned(),
                    REDACTED_HEADER_VALUE.to_owned()
                ),
                ("x-custom".to_owned(), "safe for display".to_owned()),
            ])
        );
    }
}
