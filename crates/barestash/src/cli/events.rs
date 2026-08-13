use std::{fmt, str::FromStr, time::Duration};

use clap::{Args, Subcommand};

/// `barestash events` arguments.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EventsCommand {
    #[command(subcommand)]
    pub action: EventAction,
}

/// Event actions.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum EventAction {
    /// List received events.
    List(EventListArgs),

    /// Show the most recently received event.
    Latest(EventLatestArgs),

    /// Show event details.
    Show(EventShowArgs),

    /// Follow incoming events by polling.
    Tail(EventTailArgs),

    /// Stream incoming events as JSON Lines.
    Stream(EventStreamArgs),
}

/// Arguments for `events list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct EventListArgs {
    /// Endpoint to read; uses endpoint resolution when omitted.
    #[arg(long, value_name = "endpoint-id")]
    pub endpoint: Option<String>,

    /// Number of events to fetch.
    #[arg(long, value_name = "count")]
    pub limit: Option<String>,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `events latest`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct EventLatestArgs {
    /// Endpoint to read; uses endpoint resolution when omitted.
    #[arg(long, value_name = "endpoint-id")]
    pub endpoint: Option<String>,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `events show`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EventShowArgs {
    /// Event ID to show.
    #[arg(value_name = "event-id")]
    pub event_id: String,

    /// Print JSON output.
    #[arg(long)]
    pub json: bool,
}

/// Arguments for `events tail`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct EventTailArgs {
    /// Endpoint to watch; uses endpoint resolution when omitted.
    #[arg(long, value_name = "endpoint-id")]
    pub endpoint: Option<String>,

    /// Show the last N events before watching.
    #[arg(
        long,
        value_name = "count",
        default_value = "0",
        allow_negative_numbers = true
    )]
    pub last: String,

    /// Include request headers.
    #[arg(long)]
    pub headers: bool,

    /// Include transformed request bodies.
    #[arg(long)]
    pub body: bool,

    /// Show the simple screen-updating dashboard.
    #[arg(long)]
    pub view: bool,

    /// Delay between polling requests (`ms`, `s`, or `m`).
    #[arg(long, value_name = "duration", default_value = "2s")]
    pub poll_interval: String,
}

/// Arguments for `events stream`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct EventStreamArgs {
    /// Endpoint to stream; uses endpoint resolution when omitted.
    #[arg(long, value_name = "endpoint-id")]
    pub endpoint: Option<String>,
}

/// Validated polling delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollInterval(Duration);

impl PollInterval {
    #[must_use]
    pub const fn as_duration(self) -> Duration {
        self.0
    }

    #[must_use]
    pub fn as_millis(self) -> u128 {
        self.0.as_millis()
    }
}

impl Default for PollInterval {
    fn default() -> Self {
        Self(Duration::from_secs(2))
    }
}

impl fmt::Display for PollInterval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}ms", self.0.as_millis())
    }
}

impl FromStr for PollInterval {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (amount, multiplier) = if let Some(amount) = value.strip_suffix("ms") {
            (amount, 1_u64)
        } else if let Some(amount) = value.strip_suffix('s') {
            (amount, 1_000)
        } else if let Some(amount) = value.strip_suffix('m') {
            (amount, 60_000)
        } else {
            return Err("Poll interval must include a unit: ms, s, or m.".to_owned());
        };

        if amount.is_empty() || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("Poll interval must include a unit: ms, s, or m.".to_owned());
        }

        let amount = amount
            .parse::<u64>()
            .map_err(|_| "Poll interval is too large.".to_owned())?;
        let milliseconds = amount
            .checked_mul(multiplier)
            .ok_or_else(|| "Poll interval is too large.".to_owned())?;

        Ok(Self(Duration::from_millis(milliseconds)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_poll_intervals() {
        assert_eq!("0ms".parse::<PollInterval>().expect("0ms").as_millis(), 0);
        assert_eq!(
            "500ms".parse::<PollInterval>().expect("500ms").as_millis(),
            500
        );
        assert_eq!("2s".parse::<PollInterval>().expect("2s").as_millis(), 2_000);
        assert_eq!(
            "1m".parse::<PollInterval>().expect("1m").as_millis(),
            60_000
        );
    }

    #[test]
    fn rejects_unitless_fractional_negative_and_overflowing_poll_intervals() {
        for value in ["2", "1.5s", "-1s", "18446744073709551615m"] {
            assert!(value.parse::<PollInterval>().is_err(), "accepted {value}");
        }
    }
}
