use serde::Serialize;

use crate::CliError;

pub(crate) fn print_lines(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(), CliError> {
    barestash_presentation::print_lines(lines).map_err(CliError::from)
}

pub(crate) fn print_json(value: &impl Serialize) -> Result<(), CliError> {
    barestash_presentation::print_json(value).map_err(CliError::from)
}

pub(crate) fn print_json_line(value: &impl Serialize) -> Result<(), CliError> {
    barestash_presentation::print_json_line(value).map_err(CliError::from)
}
