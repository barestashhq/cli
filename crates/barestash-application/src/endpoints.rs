mod create;
mod delete;
mod list;
mod secrets;
mod show;

use crate::command::{EndpointAction, EndpointSecretsAction, EndpointsCommand};
use crate::{AppContext, CliError};

/// Runs an `endpoints` subcommand.
pub async fn run(context: &AppContext, command: EndpointsCommand) -> Result<(), CliError> {
    match command.action {
        EndpointAction::Create(args) => create::execute(context, args).await,
        EndpointAction::List(args) => list::execute(context, args).await,
        EndpointAction::Show(args) => show::execute(context, args).await,
        EndpointAction::Delete(args) => delete::execute(context, args).await,
        EndpointAction::Secrets(command) => match command.action {
            EndpointSecretsAction::Create(args) => secrets::create(context, args).await,
            EndpointSecretsAction::List(args) => secrets::list(context, args).await,
            EndpointSecretsAction::Revoke(args) => secrets::revoke(context, args).await,
        },
    }
}
