mod args;
mod body;
mod command;
mod duration;
mod fetch;
mod headers;
mod stream;
mod tail;
mod tail_view;
mod transform;
mod view;

#[cfg(test)]
mod test_support;

pub(crate) use args::*;
pub(crate) use body::*;
pub(crate) use command::run;
pub(crate) use duration::parse_poll_interval;
pub(crate) use headers::redact_headers_for_display;
pub(crate) use tail_view::TailView;
pub(crate) use transform::*;
pub(crate) use view::*;

use barestash_client::ApiClientError;

use crate::CliError;

fn map_api_error(error: ApiClientError) -> CliError {
    match error {
        ApiClientError::Api { error, .. } => CliError::Api(error),
        ApiClientError::InvalidUrl(error) => CliError::Local(error.to_string()),
        other => CliError::Connectivity(other.to_string()),
    }
}
