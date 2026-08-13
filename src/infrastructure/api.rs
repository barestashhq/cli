use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use reqwest::header::{CONTENT_LENGTH, HeaderMap, LOCATION, TRANSFER_ENCODING};
use reqwest::redirect::Policy;
use reqwest::{Method, Request, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use thiserror::Error;
use url::{Host, Url};

use crate::protocol::{RestErrorCode, RestErrorDetail, RestErrorResponse};

pub const DEFAULT_API_URL: &str = "http://localhost:8787";
pub const DEFAULT_MAX_REDIRECTS: usize = 5;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApiUrlPolicy {
    pub allow_insecure: bool,
}

#[derive(Debug, Error)]
pub enum ApiUrlError {
    #[error("BARESTASH_API_URL is not a valid URL.")]
    InvalidUrl(#[source] url::ParseError),
    #[error("BARESTASH_API_URL must use the http: or https: scheme.")]
    UnsupportedScheme,
    #[error("BARESTASH_API_URL must not include embedded credentials.")]
    EmbeddedCredentials,
    #[error(
        "BARESTASH_API_URL points to a private or link-local address. Use --allow-insecure-api-url to override."
    )]
    PrivateOrLinkLocal,
    #[error("Redirect target points to a private or link-local address.")]
    RedirectPrivateOrLinkLocal,
}

#[derive(Debug, Error)]
pub enum ApiClientError {
    #[error(transparent)]
    InvalidUrl(#[from] ApiUrlError),
    #[error("failed to initialize the Barestash API client")]
    BuildClient(#[source] reqwest::Error),
    #[error("failed to resolve a Barestash API path")]
    ResolvePath(#[source] url::ParseError),
    #[error("failed to resolve the Barestash API host")]
    ResolveHost(#[source] std::io::Error),
    #[error("failed to build a Barestash API request")]
    BuildRequest(#[source] reqwest::Error),
    #[error("failed to reach the Barestash API")]
    Request(#[source] reqwest::Error),
    #[error("failed to read the Barestash API response")]
    ReadResponse(#[source] reqwest::Error),
    #[error("Barestash API request failed with HTTP status {status}")]
    Api {
        status: StatusCode,
        error: RestErrorResponse,
        retry_after: Option<u64>,
    },
    #[error("Barestash API redirect limit exceeded.")]
    RedirectLimitExceeded,
    #[error("Barestash API redirect is missing a Location header.")]
    MissingRedirectLocation,
    #[error("Barestash API redirect has an invalid Location header.")]
    InvalidRedirectLocation,
    #[error("Barestash API redirect to a different origin is not allowed.")]
    CrossOriginRedirect,
    #[error("Barestash API request body cannot be replayed across a redirect.")]
    UnreplayableRedirectBody,
}

/// Validates the configured API URL without making a network request.
pub fn validate_api_base_url(raw_url: &str, policy: ApiUrlPolicy) -> Result<Url, ApiUrlError> {
    let url = Url::parse(raw_url).map_err(ApiUrlError::InvalidUrl)?;
    validate_url(&url, policy, false)?;
    Ok(url)
}

/// Applies the API URL policy to every redirect destination.
pub fn validate_redirect_target(url: &Url, policy: ApiUrlPolicy) -> Result<(), ApiUrlError> {
    validate_url(url, policy, true)
}

fn validate_url(url: &Url, policy: ApiUrlPolicy, redirect: bool) -> Result<(), ApiUrlError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiUrlError::UnsupportedScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ApiUrlError::EmbeddedCredentials);
    }
    if !policy.allow_insecure && host_is_private_or_link_local(url.host()) {
        return Err(if redirect {
            ApiUrlError::RedirectPrivateOrLinkLocal
        } else {
            ApiUrlError::PrivateOrLinkLocal
        });
    }
    Ok(())
}

fn host_is_private_or_link_local(host: Option<Host<&str>>) -> bool {
    match host {
        Some(Host::Ipv4(address)) => ipv4_is_private_or_link_local(address),
        Some(Host::Ipv6(address)) => ipv6_is_private_or_link_local(address),
        Some(Host::Domain(domain)) => domain
            .trim_end_matches('.')
            .eq_ignore_ascii_case("metadata.google.internal"),
        None => false,
    }
}

fn ipv4_is_private_or_link_local(address: Ipv4Addr) -> bool {
    // Loopback is intentionally supported for local development.
    if address.is_loopback() {
        return false;
    }
    // `Ipv4Addr::is_unspecified` only covers 0.0.0.0. The reference security
    // policy rejects the complete "this network" 0.0.0.0/8 range.
    address.octets()[0] == 0 || address.is_private() || address.is_link_local()
}

fn ipv6_is_private_or_link_local(address: Ipv6Addr) -> bool {
    // Loopback is intentionally supported for local development.
    if address.is_loopback() {
        return false;
    }
    if let Some(mapped) = address.to_ipv4_mapped() {
        return ipv4_is_private_or_link_local(mapped);
    }
    let first = address.segments()[0];
    address.is_unspecified()
        || address.is_unique_local()
        || address.is_unicast_link_local()
        // Deprecated IPv6 site-local range fec0::/10.
        || (first & 0xffc0) == 0xfec0
}

#[derive(Clone, Debug)]
pub struct ApiClient {
    client: reqwest::Client,
    raw_base_url: String,
    policy: ApiUrlPolicy,
    max_redirects: usize,
    log_host: bool,
    host_logged: Arc<AtomicBool>,
    resolved_client: Arc<tokio::sync::OnceCell<reqwest::Client>>,
}

impl ApiClient {
    pub fn new(raw_base_url: &str, policy: ApiUrlPolicy) -> Result<Self, ApiClientError> {
        validate_api_base_url(raw_base_url, policy)?;
        Self::new_deferred(raw_base_url, policy)
    }

    /// Builds the transport while deferring URL parsing and policy checks to
    /// the first request. This lets local-only commands, help, and version work
    /// even when an unrelated API environment value is invalid.
    pub fn new_deferred(raw_base_url: &str, policy: ApiUrlPolicy) -> Result<Self, ApiClientError> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .map_err(ApiClientError::BuildClient)?;
        Ok(Self {
            client,
            raw_base_url: raw_base_url.to_owned(),
            policy,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            log_host: false,
            host_logged: Arc::new(AtomicBool::new(false)),
            resolved_client: Arc::new(tokio::sync::OnceCell::new()),
        })
    }

    pub fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    /// Enables the one-time production diagnostic without affecting injected
    /// clients used by unit tests.
    #[must_use]
    pub fn with_host_diagnostic(mut self, enabled: bool) -> Self {
        self.log_host = enabled;
        self
    }

    pub fn base_url(&self) -> Result<Url, ApiClientError> {
        validate_api_base_url(&self.raw_base_url, self.policy).map_err(ApiClientError::from)
    }

    pub fn url(&self, path: &str) -> Result<Url, ApiClientError> {
        self.base_url()?
            .join(path)
            .map_err(ApiClientError::ResolvePath)
    }

    /// Builds and executes a request while retaining manual redirect control.
    pub async fn send<F>(
        &self,
        method: Method,
        path: &str,
        configure: F,
    ) -> Result<Response, ApiClientError>
    where
        F: FnOnce(RequestBuilder) -> RequestBuilder,
    {
        let url = self.url(path)?;
        if self.log_host && !self.host_logged.swap(true, Ordering::AcqRel) {
            eprintln!("Barestash API host: {}", format_api_host(&url));
        }
        let request = configure(self.client.request(method, url))
            .build()
            .map_err(ApiClientError::BuildRequest)?;
        self.execute(request).await
    }

    /// Sends a JSON API request and distinguishes typed REST errors from
    /// connectivity failures. `body` is owned so callers may pass `json!(...)`
    /// directly without extending a borrow across `.await`.
    pub async fn request_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        headers: Option<HeaderMap>,
        body: Option<serde_json::Value>,
    ) -> Result<T, ApiClientError> {
        let response = self
            .send(method, path, |mut builder| {
                if let Some(headers) = headers {
                    builder = builder.headers(headers);
                }
                if let Some(body) = body {
                    builder = builder.json(&body);
                }
                builder
            })
            .await?;
        Self::decode_json(response).await
    }

    pub async fn decode_json<T: DeserializeOwned>(response: Response) -> Result<T, ApiClientError> {
        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        let bytes = response
            .bytes()
            .await
            .map_err(ApiClientError::ReadResponse)?;

        if !status.is_success() {
            let error = serde_json::from_slice::<RestErrorResponse>(&bytes)
                .unwrap_or_else(|_| invalid_json_response());
            return Err(ApiClientError::Api {
                status,
                error,
                retry_after,
            });
        }

        serde_json::from_slice(&bytes).map_err(|_| ApiClientError::Api {
            status,
            error: invalid_json_response(),
            retry_after,
        })
    }

    /// Executes an already-built request with the same redirect protections.
    pub async fn execute(&self, mut request: Request) -> Result<Response, ApiClientError> {
        let mut redirect_count = 0usize;

        loop {
            validate_redirect_target(request.url(), self.policy)?;
            let source_url = request.url().clone();
            let source_method = request.method().clone();
            let source_headers = request.headers().clone();
            let replayable_request = request.try_clone();
            let client = self.transport_client(&source_url).await?;
            let response = client
                .execute(request)
                .await
                .map_err(ApiClientError::Request)?;

            if !is_redirect_status(response.status()) {
                return Ok(response);
            }
            if redirect_count >= self.max_redirects {
                return Err(ApiClientError::RedirectLimitExceeded);
            }

            let location = response
                .headers()
                .get(LOCATION)
                .ok_or(ApiClientError::MissingRedirectLocation)?;
            if location.as_bytes().is_empty() {
                return Err(ApiClientError::MissingRedirectLocation);
            }
            let location = location
                .to_str()
                .map_err(|_| ApiClientError::InvalidRedirectLocation)?;
            let redirect_url = source_url
                .join(location)
                .map_err(|_| ApiClientError::InvalidRedirectLocation)?;

            validate_redirect_target(&redirect_url, self.policy)?;
            if source_url.origin() != redirect_url.origin() {
                return Err(ApiClientError::CrossOriginRedirect);
            }

            request = redirect_followup_request(
                replayable_request,
                source_method,
                source_headers,
                response.status(),
                redirect_url,
            )?;
            redirect_count += 1;
        }
    }

    async fn transport_client(&self, url: &Url) -> Result<reqwest::Client, ApiClientError> {
        if self.policy.allow_insecure || !matches!(url.host(), Some(Host::Domain(_))) {
            return Ok(self.client.clone());
        }
        self.resolved_client
            .get_or_try_init(|| async {
                let host = url.host_str().unwrap_or_default();
                let normalized_host = host.trim_end_matches('.');
                let port = url.port_or_known_default().ok_or_else(|| {
                    ApiClientError::ResolveHost(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "API URL has no resolvable port",
                    ))
                })?;
                let addresses = tokio::net::lookup_host((host, port))
                    .await
                    .map_err(ApiClientError::ResolveHost)?
                    .collect::<Vec<_>>();
                if addresses.is_empty() {
                    return Err(ApiClientError::ResolveHost(std::io::Error::new(
                        std::io::ErrorKind::AddrNotAvailable,
                        "API host resolved to no addresses",
                    )));
                }
                if !resolved_addresses_are_allowed(normalized_host, &addresses) {
                    return Err(ApiClientError::InvalidUrl(ApiUrlError::PrivateOrLinkLocal));
                }
                reqwest::Client::builder()
                    .redirect(Policy::none())
                    .resolve_to_addrs(host, &addresses)
                    .build()
                    .map_err(ApiClientError::BuildClient)
            })
            .await
            .cloned()
    }
}

fn resolved_addresses_are_allowed(host: &str, addresses: &[SocketAddr]) -> bool {
    let localhost = host.eq_ignore_ascii_case("localhost");
    addresses.iter().all(|address| {
        if localhost {
            return resolved_ip_is_loopback(address.ip());
        }
        !resolved_ip_is_private_or_link_local(address.ip())
    })
}

fn resolved_ip_is_loopback(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_loopback(),
        IpAddr::V6(address) => {
            address.is_loopback()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| mapped.is_loopback())
        }
    }
}

fn resolved_ip_is_private_or_link_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_loopback() || ipv4_is_private_or_link_local(address),
        IpAddr::V6(address) => {
            resolved_ip_is_loopback(IpAddr::V6(address)) || ipv6_is_private_or_link_local(address)
        }
    }
}

fn redirect_followup_request(
    replayable_request: Option<Request>,
    source_method: Method,
    source_headers: HeaderMap,
    status: StatusCode,
    redirect_url: Url,
) -> Result<Request, ApiClientError> {
    let rewrite_to_get = status == StatusCode::SEE_OTHER
        && source_method != Method::GET
        && source_method != Method::HEAD
        || matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND)
            && source_method == Method::POST;

    if rewrite_to_get {
        let mut request = Request::new(Method::GET, redirect_url);
        let mut headers = source_headers;
        headers.remove(CONTENT_LENGTH);
        headers.remove(TRANSFER_ENCODING);
        *request.headers_mut() = headers;
        return Ok(request);
    }

    let mut request = replayable_request.ok_or(ApiClientError::UnreplayableRedirectBody)?;
    *request.url_mut() = redirect_url;
    Ok(request)
}

fn is_redirect_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

fn invalid_json_response() -> RestErrorResponse {
    RestErrorResponse {
        error: RestErrorDetail {
            code: RestErrorCode::InternalError,
            message: "Barestash API returned a response that was not valid JSON.".into(),
        },
    }
}

pub fn format_api_host(url: &Url) -> &str {
    url.host_str().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[test]
    fn validates_local_and_public_api_urls() {
        let policy = ApiUrlPolicy::default();
        assert!(validate_api_base_url("http://localhost:8787", policy).is_ok());
        assert!(validate_api_base_url("http://127.0.0.1:8787", policy).is_ok());
        assert!(validate_api_base_url("http://[::1]:8787", policy).is_ok());
        assert!(validate_api_base_url("https://api.example.com", policy).is_ok());
    }

    #[test]
    fn rejects_the_complete_ipv4_this_network_range() {
        let policy = ApiUrlPolicy::default();
        assert!(matches!(
            validate_api_base_url("http://0.1.2.3", policy),
            Err(ApiUrlError::PrivateOrLinkLocal)
        ));
    }

    #[test]
    fn rejects_unsafe_base_urls() {
        let policy = ApiUrlPolicy::default();
        assert!(matches!(
            validate_api_base_url("file:///etc/passwd", policy),
            Err(ApiUrlError::UnsupportedScheme)
        ));
        assert!(matches!(
            validate_api_base_url("https://token:secret@api.example.com", policy),
            Err(ApiUrlError::EmbeddedCredentials)
        ));
        for raw in [
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://192.168.0.1/",
            "http://169.254.169.254/",
            "http://[fe90::1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://metadata.google.internal./",
        ] {
            assert!(matches!(
                validate_api_base_url(raw, policy),
                Err(ApiUrlError::PrivateOrLinkLocal)
            ));
        }
    }

    #[test]
    fn explicit_policy_allows_private_addresses() {
        let policy = ApiUrlPolicy {
            allow_insecure: true,
        };
        assert!(validate_api_base_url("http://192.168.0.1:8787", policy).is_ok());
    }

    #[test]
    fn dns_results_cannot_turn_a_public_hostname_into_a_private_destination() {
        let private = [SocketAddr::from(([10, 0, 0, 1], 443))];
        let loopback = [SocketAddr::from(([127, 0, 0, 1], 8787))];
        let mapped_loopback = [SocketAddr::new(
            "::ffff:127.0.0.1"
                .parse()
                .unwrap_or(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            8787,
        )];
        let public = [SocketAddr::from(([93, 184, 216, 34], 443))];
        assert!(!resolved_addresses_are_allowed("api.example", &private));
        assert!(!resolved_addresses_are_allowed("api.example", &loopback));
        assert!(!resolved_addresses_are_allowed(
            "api.example",
            &mapped_loopback
        ));
        assert!(resolved_addresses_are_allowed("localhost", &loopback));
        assert!(resolved_addresses_are_allowed(
            "localhost",
            &mapped_loopback
        ));
        assert!(resolved_addresses_are_allowed("api.example", &public));
    }

    #[test]
    fn rewrites_post_to_get_for_302_without_dropping_authorization() {
        let url = Url::parse("https://api.example.com/v1/tokens").expect("source URL");
        let mut source = Request::new(Method::POST, url);
        source
            .headers_mut()
            .insert(AUTHORIZATION, "Bearer secret".parse().expect("header"));
        source
            .headers_mut()
            .insert(CONTENT_TYPE, "application/json".parse().expect("header"));
        *source.body_mut() = Some(reqwest::Body::from("{}"));
        let headers = source.headers().clone();
        let next = redirect_followup_request(
            source.try_clone(),
            Method::POST,
            headers,
            StatusCode::FOUND,
            Url::parse("https://api.example.com/v2/tokens").expect("redirect URL"),
        )
        .expect("follow-up request");

        assert_eq!(next.method(), Method::GET);
        assert!(next.body().is_none());
        assert_eq!(
            next.headers().get(AUTHORIZATION).expect("authorization"),
            "Bearer secret"
        );
    }

    #[test]
    fn preserves_methods_and_bodies_for_307_and_delete_for_302() {
        let destination = Url::parse("https://api.example.com/next").expect("redirect URL");
        for (method, status) in [
            (Method::POST, StatusCode::TEMPORARY_REDIRECT),
            (Method::DELETE, StatusCode::FOUND),
        ] {
            let mut source = Request::new(
                method.clone(),
                Url::parse("https://api.example.com/start").expect("source URL"),
            );
            *source.body_mut() = Some(reqwest::Body::from("request body"));
            let next = redirect_followup_request(
                source.try_clone(),
                method.clone(),
                HeaderMap::new(),
                status,
                destination.clone(),
            )
            .expect("follow-up request");
            assert_eq!(next.method(), method);
            assert!(next.body().is_some());
        }
    }

    #[test]
    fn origin_comparison_includes_scheme_and_port() {
        let source = Url::parse("https://api.example.com/start").expect("source URL");
        let same = Url::parse("https://api.example.com/next").expect("same-origin URL");
        let other_port = Url::parse("https://api.example.com:8443/next").expect("other-port URL");
        let other_scheme = Url::parse("http://api.example.com/next").expect("other-scheme URL");
        assert_eq!(source.origin(), same.origin());
        assert_ne!(source.origin(), other_port.origin());
        assert_ne!(source.origin(), other_scheme.origin());
    }

    #[test]
    fn redirect_private_address_uses_redirect_specific_error() {
        let target =
            Url::parse("http://169.254.169.254/latest/meta-data/").expect("redirect target");
        assert!(matches!(
            validate_redirect_target(&target, ApiUrlPolicy::default()),
            Err(ApiUrlError::RedirectPrivateOrLinkLocal)
        ));
    }

    #[test]
    fn recognizes_only_the_five_redirect_statuses() {
        for status in [301, 302, 303, 307, 308] {
            assert!(is_redirect_status(
                StatusCode::from_u16(status).expect("status")
            ));
        }
        assert!(!is_redirect_status(StatusCode::MULTIPLE_CHOICES));
        assert!(!is_redirect_status(StatusCode::NOT_MODIFIED));
    }

    #[test]
    fn accepts_only_numeric_retry_after_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "10800".parse().expect("header"),
        );
        assert_eq!(parse_retry_after(&headers), Some(10_800));
        headers.insert(reqwest::header::RETRY_AFTER, "-1".parse().expect("header"));
        assert_eq!(parse_retry_after(&headers), None);
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2015 07:28:00 GMT".parse().expect("header"),
        );
        assert_eq!(parse_retry_after(&headers), None);
    }

    #[test]
    fn ip_address_helper_preserves_loopback_exception() {
        assert!(!ipv4_is_private_or_link_local(Ipv4Addr::LOCALHOST));
        assert!(!ipv6_is_private_or_link_local(Ipv6Addr::LOCALHOST));
        assert!(ipv4_is_private_or_link_local(Ipv4Addr::new(0, 0, 0, 0)));
        assert!(ipv6_is_private_or_link_local(Ipv6Addr::UNSPECIFIED));
        assert!(host_is_private_or_link_local(Some(Host::Ipv4(
            Ipv4Addr::new(10, 0, 0, 1)
        ))));
    }

    #[tokio::test]
    async fn refuses_cross_origin_redirect_before_forwarding_authorization() {
        let source = MockServer::start().await;
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/stolen", destination.uri())),
            )
            .mount(&source)
            .await;

        let client = ApiClient::new(&source.uri(), ApiUrlPolicy::default()).expect("API client");
        let error = client
            .send(Method::GET, "/start", |request| {
                request.header(AUTHORIZATION, "Bearer secret")
            })
            .await
            .expect_err("cross-origin redirect");

        assert!(matches!(error, ApiClientError::CrossOriginRedirect));
        assert!(
            destination
                .received_requests()
                .await
                .expect("destination requests")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn follows_same_origin_redirect_with_fetch_method_semantics() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/start"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/finish"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/finish"))
            .and(header("authorization", "Bearer secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
            .mount(&server)
            .await;

        let client = ApiClient::new(&server.uri(), ApiUrlPolicy::default()).expect("API client");
        let response = client
            .send(Method::POST, "/start", |request| {
                request
                    .header(AUTHORIZATION, "Bearer secret")
                    .json(&json!({ "name": "ci" }))
            })
            .await
            .expect("redirected response");

        assert_eq!(response.status(), StatusCode::OK);
        let requests = server.received_requests().await.expect("received requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].method.as_str(), "GET");
        assert!(requests[1].body.is_empty());
    }

    #[tokio::test]
    async fn rejects_redirect_without_location() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(ResponseTemplate::new(302))
            .mount(&server)
            .await;
        let client = ApiClient::new(&server.uri(), ApiUrlPolicy::default()).expect("API client");

        assert!(matches!(
            client.send(Method::GET, "/start", |request| request).await,
            Err(ApiClientError::MissingRedirectLocation)
        ));
    }
}
