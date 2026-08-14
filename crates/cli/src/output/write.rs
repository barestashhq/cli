use std::io::{self, Write};

use serde::Serialize;

use super::{PresentationError, sanitize_terminal_text};

pub fn print_lines(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<(), PresentationError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for line in lines {
        writeln!(output, "{}", sanitize_terminal_text(line.as_ref()))?;
    }
    output.flush()?;
    Ok(())
}

pub fn print_json(value: &impl Serialize) -> Result<(), PresentationError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

pub fn print_json_line(value: &impl Serialize) -> Result<(), PresentationError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
