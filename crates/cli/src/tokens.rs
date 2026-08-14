mod args;
mod create;
mod list;
mod revoke;
mod view;

use crate::{AppContext, CliError};

pub(crate) use args::*;
pub(crate) use view::{
    print_created as print_token_created, print_diagnostic as print_token_diagnostic,
    print_list as print_token_list, print_revoked as print_token_revoked,
};

/// Runs a `tokens` subcommand.
pub async fn run(context: &AppContext, command: TokensCommand) -> Result<(), CliError> {
    match command.action {
        TokenAction::Create(args) => create::execute(context, args).await,
        TokenAction::List(args) => list::execute(context, args).await,
        TokenAction::Revoke(args) => revoke::execute(context, args).await,
    }
}
