mod create;
mod list;
mod revoke;

use crate::command::{TokenAction, TokensCommand};
use crate::{AppContext, CliError};

/// Runs a `tokens` subcommand.
pub async fn run(context: &AppContext, command: TokensCommand) -> Result<(), CliError> {
    match command.action {
        TokenAction::Create(args) => create::execute(context, args).await,
        TokenAction::List(args) => list::execute(context, args).await,
        TokenAction::Revoke(args) => revoke::execute(context, args).await,
    }
}
