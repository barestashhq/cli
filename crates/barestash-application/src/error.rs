use barestash_client::ApiClientError;
use barestash_protocol::RestErrorResponse;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Local(String),
    #[error("{0}")]
    Api(#[from] RestErrorResponse),
    #[error("Failed to reach Barestash API.\n{0}")]
    Connectivity(String),
    #[error("{0}")]
    Infrastructure(String),
    #[error("diagnostic already reported")]
    AlreadyReported,
}

impl CliError {
    /// Converts the infrastructure error without exposing request internals or
    /// credentials in user-facing diagnostics.
    pub fn from_api_client(error: ApiClientError) -> Self {
        match error {
            ApiClientError::InvalidUrl(error) => Self::Local(error.to_string()),
            ApiClientError::InvalidLastEventId(_) => Self::Local(error.to_string()),
            ApiClientError::Api { error, .. } => Self::Api(error),
            other => Self::Connectivity(other.to_string()),
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(value: std::io::Error) -> Self {
        Self::Infrastructure(value.to_string())
    }
}

impl From<ApiClientError> for CliError {
    fn from(value: ApiClientError) -> Self {
        Self::from_api_client(value)
    }
}

impl From<barestash_presentation::PresentationError> for CliError {
    fn from(value: barestash_presentation::PresentationError) -> Self {
        Self::Infrastructure(value.to_string())
    }
}
