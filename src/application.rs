pub mod auth;
pub mod endpoints;
pub mod events;
pub mod tokens;

mod context;
mod error;

pub use context::AppContext;
pub use error::CliError;

use clap::Parser;

use crate::cli::{Cli, unknown_command};

pub async fn run() -> Result<(), CliError> {
    let arguments = std::env::args_os()
        .skip(1)
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if let Some(command) = unknown_command(&arguments) {
        eprintln!("Unknown command: {command}");
        eprintln!("Run `barestash --help` for usage.");
        return Err(CliError::AlreadyReported);
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            error
                .print()
                .map_err(|print_error| CliError::Infrastructure(print_error.to_string()))?;
            return Ok(());
        }
        Err(error) => {
            error
                .print()
                .map_err(|print_error| CliError::Infrastructure(print_error.to_string()))?;
            return Err(CliError::AlreadyReported);
        }
    };
    if cli.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let Some(command) = cli.command else {
        use clap::CommandFactory as _;
        Cli::command()
            .print_help()
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        println!();
        return Ok(());
    };
    let context = AppContext::from_environment(cli.allow_insecure_api_url)?;
    dispatch(&context, command).await
}

async fn dispatch(
    context: &AppContext,
    command: crate::cli::ResourceCommand,
) -> Result<(), CliError> {
    use crate::cli::ResourceCommand;

    match command {
        ResourceCommand::Auth(command) => auth::run(context, command).await,
        ResourceCommand::Endpoints(command) => endpoints::run(context, command).await,
        ResourceCommand::Events(command) => events::run(context, command).await,
        ResourceCommand::Tokens(command) => tokens::run(context, command).await,
    }
}
