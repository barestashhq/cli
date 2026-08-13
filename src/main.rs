use std::process::ExitCode;

use barestash::{cli, domain, infrastructure, protocol};

mod application;
mod error;
mod presentation;

#[tokio::main]
async fn main() -> ExitCode {
    match application::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            presentation::print_cli_error(&error);
            ExitCode::from(1)
        }
    }
}
