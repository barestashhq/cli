use crate::{AppContext, CliError};

pub(in crate::auth) async fn clear_stored_credential(context: &AppContext) -> Result<(), CliError> {
    clear_legacy_config_token(context).await?;
    context
        .credentials
        .delete()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))
}

pub(in crate::auth) async fn clear_legacy_config_token(
    context: &AppContext,
) -> Result<(), CliError> {
    let mut config = context
        .config
        .read()
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    if config.token.take().is_none() {
        return Ok(());
    }
    context
        .config
        .write(&config)
        .await
        .map_err(|error| CliError::Infrastructure(error.to_string()))
}
