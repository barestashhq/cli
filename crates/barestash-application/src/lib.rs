mod auth;
mod command;
mod context;
mod endpoints;
mod error;
mod events;
mod output;
mod tokens;

pub use command::*;
pub use error::CliError;

use context::AppContext;

/// Executes an already parsed CLI command.
///
/// Argument parsing, root help, and version output belong to the `barestash`
/// facade. Keeping this entrypoint free of clap lets the application crate be
/// cached independently of command-line parser changes.
pub async fn execute(options: ExecutionOptions, command: AppCommand) -> Result<(), CliError> {
    let context = AppContext::from_environment(options.allow_insecure_api_url)?;
    dispatch(&context, command, &options.client_version).await
}

async fn dispatch(
    context: &AppContext,
    command: AppCommand,
    client_version: &str,
) -> Result<(), CliError> {
    match command {
        AppCommand::Auth(command) => auth::run(context, command, client_version).await,
        AppCommand::Endpoints(command) => endpoints::run(context, command).await,
        AppCommand::Events(command) => events::run(context, command).await,
        AppCommand::Tokens(command) => tokens::run(context, command).await,
    }
}
