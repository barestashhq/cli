mod args;
mod create;
mod delete;
mod list;
mod secrets;
mod show;
mod view;

use crate::{AppContext, CliError};

pub(crate) use args::*;
pub(crate) use view::{
    print_created as print_endpoint_created, print_deleted as print_endpoint_deleted,
    print_detail as print_endpoint_detail, print_list as print_endpoint_list, print_secret_created,
    print_secret_list, print_secret_revoked,
};

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
