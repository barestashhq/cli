use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match barestash::application::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error.print();
            ExitCode::from(1)
        }
    }
}
