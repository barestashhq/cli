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
