use std::process::ExitCode;

use barestash_application::{AppCommand, CliError, ExecutionOptions};
use clap::{CommandFactory as _, Parser as _};

use crate::cli::{Cli, unknown_command};

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

    barestash_application::execute(
        ExecutionOptions {
            allow_insecure_api_url: cli.allow_insecure_api_url,
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        AppCommand::from(command),
    )
    .await
}

fn print_cli_error(error: &CliError) {
    match error {
        CliError::AlreadyReported => {}
        CliError::Api(response) => barestash_presentation::print_api_error(response, None),
        other => barestash_presentation::print_error_text(&other.to_string()),
    }
}
