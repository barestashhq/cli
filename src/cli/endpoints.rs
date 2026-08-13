use clap::{Args, Subcommand};

/// `barestash endpoints` arguments.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EndpointsCommand {
    #[command(subcommand)]
    pub action: EndpointAction,
}

/// Endpoint actions.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum EndpointAction {
    /// Create an endpoint.
    Create(EndpointCreateArgs),

    /// List endpoints.
    List(EndpointListArgs),

    /// Show endpoint details.
    Show(EndpointShowArgs),

    /// Delete an endpoint.
    Delete(EndpointDeleteArgs),

    /// Manage endpoint ingest secrets.
    Secrets(EndpointSecretsCommand),
}

/// Arguments for `endpoints create`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct EndpointCreateArgs {
    /// Create a private endpoint (the default mode).
    #[arg(long = "private")]
    pub private: bool,

    /// Create a temporary public-by-URL endpoint.
    #[arg(long)]
    pub temporary: bool,

    /// Assign a human-readable name.
    #[arg(long, value_name = "name")]
    pub name: Option<String>,

    /// Set the created endpoint as the CLI default.
    #[arg(long)]
    pub set_default: bool,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `endpoints list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct EndpointListArgs {
    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `endpoints show`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EndpointShowArgs {
    /// Endpoint ID to show.
    #[arg(value_name = "endpoint-id")]
    pub endpoint_id: String,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `endpoints delete`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EndpointDeleteArgs {
    /// Endpoint ID to delete.
    #[arg(value_name = "endpoint-id")]
    pub endpoint_id: String,

    /// Delete without prompting.
    #[arg(long)]
    pub yes: bool,
}

/// `barestash endpoints secrets` arguments.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EndpointSecretsCommand {
    #[command(subcommand)]
    pub action: EndpointSecretsAction,
}

/// Endpoint-secret actions.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum EndpointSecretsAction {
    /// Create an endpoint ingest secret.
    Create(EndpointSecretCreateArgs),

    /// List endpoint ingest secrets.
    List(EndpointSecretListArgs),

    /// Revoke an endpoint ingest secret.
    Revoke(EndpointSecretRevokeArgs),
}

/// Arguments for `endpoints secrets create`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct EndpointSecretCreateArgs {
    /// Target endpoint; uses endpoint resolution when omitted.
    #[arg(long, value_name = "endpoint-id")]
    pub endpoint: Option<String>,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `endpoints secrets list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct EndpointSecretListArgs {
    /// Target endpoint; uses endpoint resolution when omitted.
    #[arg(long, value_name = "endpoint-id")]
    pub endpoint: Option<String>,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `endpoints secrets revoke`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EndpointSecretRevokeArgs {
    /// Secret ID to revoke.
    #[arg(value_name = "secret-id")]
    pub secret_id: String,

    /// Target endpoint; uses endpoint resolution when omitted.
    #[arg(long, value_name = "endpoint-id")]
    pub endpoint: Option<String>,

    /// Revoke without prompting.
    #[arg(long)]
    pub yes: bool,
}
