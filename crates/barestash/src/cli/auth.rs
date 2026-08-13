use clap::{Args, Subcommand};

/// `barestash auth` arguments.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub action: AuthAction,
}

/// Authentication actions.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AuthAction {
    /// Authenticate the CLI.
    Login(AuthLoginArgs),

    /// Show authentication status.
    Status(AuthStatusArgs),

    /// Remove local authentication credentials.
    Logout(AuthLogoutArgs),
}

/// Arguments for `auth login`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct AuthLoginArgs {
    /// Read a Personal Access Token from stdin.
    #[arg(long)]
    pub with_token: bool,

    /// Store credentials in a user-only plaintext file.
    #[arg(long)]
    pub insecure_storage: bool,
}

/// Arguments for `auth status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct AuthStatusArgs {
    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `auth logout`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct AuthLogoutArgs {
    /// Revoke the stored remote credential before clearing local state.
    #[arg(long)]
    pub revoke: bool,
}
