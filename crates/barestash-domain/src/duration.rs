use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationParseError {
    PollIntervalUnit,
    TokenExpirationUnit,
    TokenExpirationNotPositive,
    TooLarge,
}

impl fmt::Display for DurationParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PollIntervalUnit => {
                formatter.write_str("Poll interval must include a unit: ms, s, or m.")
            }
            Self::TokenExpirationUnit => {
                formatter.write_str("Token expiration must include a unit: d or y.")
            }
            Self::TokenExpirationNotPositive => {
                formatter.write_str("Token expiration must be a positive duration.")
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

pub fn parse_token_duration_seconds(value: &str) -> Result<u64, DurationParseError> {
    let (amount, days_multiplier) = if let Some(amount) = value.strip_suffix('d') {
        (amount, 1_u64)
    } else if let Some(amount) = value.strip_suffix('y') {
        (amount, 365_u64)
    } else {
        return Err(DurationParseError::TokenExpirationUnit);
    };
    let amount = parse_ascii_digits(amount).ok_or(DurationParseError::TokenExpirationUnit)?;

    if amount == 0 {
        return Err(DurationParseError::TokenExpirationNotPositive);
    }

    amount
        .checked_mul(days_multiplier)
        .and_then(|days| days.checked_mul(24 * 60 * 60))
        .ok_or(DurationParseError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_poll_and_token_durations() {
        assert_eq!(parse_poll_interval("500ms"), Ok(500));
        assert_eq!(parse_poll_interval("2s"), Ok(2_000));
        assert_eq!(parse_poll_interval("1m"), Ok(60_000));
        assert_eq!(parse_poll_interval("0ms"), Ok(0));
        assert_eq!(
            parse_poll_interval("2"),
            Err(DurationParseError::PollIntervalUnit)
        );

        assert_eq!(parse_token_duration_seconds("30d"), Ok(2_592_000));
        assert_eq!(parse_token_duration_seconds("1y"), Ok(31_536_000));
        assert_eq!(
            parse_token_duration_seconds("0d"),
            Err(DurationParseError::TokenExpirationNotPositive)
        );
        assert_eq!(
            parse_token_duration_seconds("90days"),
            Err(DurationParseError::TokenExpirationUnit)
        );
    }
}
