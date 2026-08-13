use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match barestash::application::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            barestash::presentation::print_cli_error(&error);
            ExitCode::from(1)
        }
    }
}
