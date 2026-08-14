//! Terminal input helpers.

use std::io::{self, Read, Write};

pub fn read_stdin_to_string() -> io::Result<String> {
    let mut value = String::new();
    io::stdin().read_to_string(&mut value)?;
    Ok(value)
}

pub fn confirm(message: &str) -> io::Result<bool> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    write!(output, "{message} Type yes to continue: ")?;
    output.flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("yes"))
}
