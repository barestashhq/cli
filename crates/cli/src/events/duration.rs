use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationParseError {
    PollIntervalUnit,
    TooLarge,
}

impl fmt::Display for DurationParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PollIntervalUnit => {
                formatter.write_str("Poll interval must include a unit: ms, s, or m.")
            }
            Self::TooLarge => formatter.write_str("Duration is too large."),
        }
    }
}

impl std::error::Error for DurationParseError {}

fn parse_ascii_digits(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub fn parse_poll_interval(value: &str) -> Result<u64, DurationParseError> {
    let (amount, multiplier) = if let Some(amount) = value.strip_suffix("ms") {
        (amount, 1)
    } else if let Some(amount) = value.strip_suffix('s') {
        (amount, 1_000)
    } else if let Some(amount) = value.strip_suffix('m') {
        (amount, 60_000)
    } else {
        return Err(DurationParseError::PollIntervalUnit);
    };
    let amount = parse_ascii_digits(amount).ok_or(DurationParseError::PollIntervalUnit)?;
    amount
        .checked_mul(multiplier)
        .ok_or(DurationParseError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_poll_durations() {
        assert_eq!(parse_poll_interval("500ms"), Ok(500));
        assert_eq!(parse_poll_interval("2s"), Ok(2_000));
        assert_eq!(parse_poll_interval("1m"), Ok(60_000));
        assert_eq!(parse_poll_interval("0ms"), Ok(0));
        assert_eq!(
            parse_poll_interval("2"),
            Err(DurationParseError::PollIntervalUnit)
        );
        for value in ["1.5s", "-1s", "18446744073709551615m"] {
            assert!(parse_poll_interval(value).is_err(), "accepted {value}");
        }
    }
}
