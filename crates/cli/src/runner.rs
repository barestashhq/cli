use std::process::ExitCode;

use clap::{CommandFactory as _, Parser as _};

use crate::cli::{Cli, unknown_command};
use crate::{AppContext, CliError, auth, endpoints, events, output, tokens};

pub async fn run() -> ExitCode {
    match try_run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_cli_error(&error);
            ExitCode::from(1)
        }
    }
}

async fn try_run() -> Result<(), CliError> {
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
                .map_err(|error| CliError::Infrastructure(error.to_string()))?;
            return Ok(());
        }
        Err(error) => {
            error
                .print()
                .map_err(|error| CliError::Infrastructure(error.to_string()))?;
            return Err(CliError::AlreadyReported);
        }
    };

    if cli.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let Some(command) = cli.command else {
        Cli::command()
            .print_help()
            .map_err(|error| CliError::Infrastructure(error.to_string()))?;
        println!();
        return Ok(());
    };

    let context = AppContext::from_environment(cli.allow_insecure_api_url)?;
    match command {
        crate::cli::ResourceCommand::Auth(command) => {
            auth::run(&context, command, env!("CARGO_PKG_VERSION")).await
        }
        crate::cli::ResourceCommand::Endpoints(command) => endpoints::run(&context, command).await,
        crate::cli::ResourceCommand::Events(command) => events::run(&context, command).await,
        crate::cli::ResourceCommand::Tokens(command) => tokens::run(&context, command).await,
    }
}

fn print_cli_error(error: &CliError) {
    match error {
        CliError::AlreadyReported => {}
        CliError::Api(response) => output::print_api_error(response, None),
        other => output::print_error_text(&other.to_string()),
    }
}
