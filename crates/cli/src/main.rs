use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    barestash::run().await
}
