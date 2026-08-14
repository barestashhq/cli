mod command;
mod fetch;
mod stream;
mod tail;

#[cfg(test)]
mod test_support;

pub(crate) use command::run;

use barestash_client::ApiClientError;

use crate::CliError;

fn map_api_error(error: ApiClientError) -> CliError {
    match error {
        ApiClientError::Api { error, .. } => CliError::Api(error),
        ApiClientError::InvalidUrl(error) => CliError::Local(error.to_string()),
        other => CliError::Connectivity(other.to_string()),
    }
}
