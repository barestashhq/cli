use std::io::{self, Write};

use serde::Serialize;

use crate::application::CliError;
use crate::presentation::renderer::sanitize_terminal_text;

pub fn print_lines(lines: impl IntoIterator<Item = impl AsRef<str>>) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for line in lines {
        writeln!(output, "{}", sanitize_terminal_text(line.as_ref()))?;
    }
    output.flush()?;
    Ok(())
}

pub fn print_json(value: &impl Serialize) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

pub fn print_json_line(value: &impl Serialize) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, value)
        .map_err(|error| CliError::Infrastructure(error.to_string()))?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
