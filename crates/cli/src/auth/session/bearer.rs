use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

use crate::CliError;

pub(in crate::auth) fn authorization_headers(token: Option<String>) -> Result<HeaderMap, CliError> {
    let mut headers = HeaderMap::new();
    if let Some(token) = token {
        insert_bearer(&mut headers, &token)?;
    }
    Ok(headers)
}

pub(super) fn insert_bearer(headers: &mut HeaderMap, token: &str) -> Result<(), CliError> {
    let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
        CliError::Local("The authentication token is not valid for an HTTP header.".into())
    })?;
    headers.insert(AUTHORIZATION, value);
    Ok(())
}

pub(super) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_header_does_not_expose_invalid_token_in_errors() {
        let error = authorization_headers(Some("secret\nvalue".into()))
            .expect_err("invalid header should fail")
            .to_string();
        assert!(!error.contains("secret"));
    }
}
