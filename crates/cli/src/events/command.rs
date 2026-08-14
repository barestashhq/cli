use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use barestash_protocol::{EventDetail, EventMetadata};

use super::fetch::{
    EventWithBody, LatestEventWithBody, fetch_event_body, fetch_event_detail, list_events,
    show_event, show_latest_event,
};
use super::stream::{StreamOptions, stream_events};
use super::tail::{TailOptions, parse_tail_last, tail_events};
use super::{
    EventAction, EventListArgs, EventShowArgs, EventStreamArgs, EventTailArgs, EventsCommand,
    TailView, parse_poll_interval, print_event_body, print_event_detail, print_event_headers,
    print_event_list, print_event_summary, print_tail_header, redact_headers_for_display,
};
use crate::output::{TerminalCapabilities, print_json, print_json_line, print_lines};
use crate::{AppContext, CliError};

pub(crate) async fn run(context: &AppContext, command: EventsCommand) -> Result<(), CliError> {
    match command.action {
        EventAction::List(arguments) => run_list(context, arguments).await,
        EventAction::Latest(arguments) => {
            let endpoint_id = context
                .selected_endpoint(arguments.endpoint.as_deref())
                .await?;
            let value = show_latest_event(context, &endpoint_id).await?;
            if arguments.json {
                let value = redact_latest_for_display(value);
                print_json(&value).map_err(Into::into)
            } else if let Some(event) = value.event {
                print_event_detail(&event, value.body.as_ref()).map_err(CliError::from)
            } else {
                print_lines(["No events received yet."]).map_err(Into::into)
            }
        }
        EventAction::Show(arguments) => run_show(context, arguments).await,
        EventAction::Tail(arguments) => run_tail(context, arguments).await,
        EventAction::Stream(arguments) => run_stream(context, arguments).await,
    }
}

async fn run_list(context: &AppContext, arguments: EventListArgs) -> Result<(), CliError> {
    let endpoint_id = context
        .selected_endpoint(arguments.endpoint.as_deref())
        .await?;
    let events = list_events(context, &endpoint_id, arguments.limit.as_deref(), None).await?;
    if arguments.json {
        #[derive(Serialize)]
        struct Output<'a> {
            events: &'a [EventMetadata],
        }
        let events = events
            .into_iter()
            .map(redact_event_metadata_for_display)
            .collect::<Vec<_>>();
        return print_json(&Output { events: &events }).map_err(Into::into);
    }
    print_event_list(&events).map_err(CliError::from)
}

async fn run_show(context: &AppContext, arguments: EventShowArgs) -> Result<(), CliError> {
    let value = show_event(context, &arguments.event_id).await?;
    if arguments.json {
        return print_json(&redact_event_with_body_for_display(value)).map_err(Into::into);
    }
    print_event_detail(&value.event, Some(&value.body)).map_err(CliError::from)
}

async fn run_tail(context: &AppContext, arguments: EventTailArgs) -> Result<(), CliError> {
    let capabilities = TerminalCapabilities::detect();
    if arguments.view && !capabilities.interactive {
        return Err(CliError::Local(
            "--view requires an interactive terminal.".into(),
        ));
    }
    if arguments.view && (arguments.headers || arguments.body) {
        return Err(CliError::Local(
            "--view cannot be combined with --headers or --body.".into(),
        ));
    }
    let endpoint_id = context
        .selected_endpoint(arguments.endpoint.as_deref())
        .await?;
    let poll_interval = parse_poll_interval(&arguments.poll_interval)
        .map_err(|error| CliError::Local(error.to_string()))?;
    let poll_interval = Duration::from_millis(poll_interval);
    if std::time::Instant::now()
        .checked_add(poll_interval)
        .is_none()
    {
        return Err(CliError::Local("Poll interval is too large.".into()));
    }
    let last = parse_tail_last(&arguments.last)?;
    let view = arguments
        .view
        .then(|| Arc::new(Mutex::new(TailView::new(&endpoint_id, capabilities))));
    let ready_view = view.clone();
    let event_view = view.clone();
    let include_headers = arguments.headers;
    let include_body = arguments.body;
    let options = TailOptions::watching(last, poll_interval);
    let operation = tail_events(
        context,
        &endpoint_id,
        options,
        || {
            if let Some(view) = &ready_view {
                return view
                    .lock()
                    .map_err(|_| CliError::Infrastructure("tail view lock was poisoned".into()))?
                    .start()
                    .map_err(CliError::from);
            }
            print_tail_header(&endpoint_id, capabilities).map_err(CliError::from)
        },
        |event| {
            let event_view = event_view.clone();
            async move {
                if let Some(view) = event_view {
                    return view
                        .lock()
                        .map_err(|_| {
                            CliError::Infrastructure("tail view lock was poisoned".into())
                        })?
                        .add(event)
                        .map_err(CliError::from);
                }
                print_event_summary(&event)?;
                if include_headers || include_body {
                    let detail = fetch_event_detail(context, &event.id).await?;
                    if include_headers {
                        print_event_headers(&detail)?;
                    }
                    if include_body {
                        let body = fetch_event_body(context, &detail).await?;
                        print_event_body(&body)?;
                    }
                }
                Ok(())
            }
        },
    );

    let result = tokio::select! {
        biased;
        interrupted = tokio::signal::ctrl_c() => {
            interrupted.map_err(|error| CliError::Infrastructure(error.to_string()))?;
            Ok(())
        }
        result = operation => result,
    };
    drop(view);
    result
}

async fn run_stream(context: &AppContext, arguments: EventStreamArgs) -> Result<(), CliError> {
    let endpoint_id = context
        .selected_endpoint(arguments.endpoint.as_deref())
        .await?;
    let operation = stream_events(context, &endpoint_id, StreamOptions::default(), |payload| {
        print_json_line(&payload).map_err(Into::into)
    });

    tokio::select! {
        biased;
        interrupted = tokio::signal::ctrl_c() => {
            interrupted.map_err(|error| CliError::Infrastructure(error.to_string()))?;
            Ok(())
        }
        result = operation => result,
    }
}

fn redact_event_for_display(mut event: EventDetail) -> EventDetail {
    event.request.headers = redact_headers_for_display(&event.request.headers);
    event
}

fn redact_event_metadata_for_display(mut event: EventMetadata) -> EventMetadata {
    event.headers = redact_headers_for_display(&event.headers);
    event
}

fn redact_event_with_body_for_display(mut value: EventWithBody) -> EventWithBody {
    value.event = redact_event_for_display(value.event);
    value
}

fn redact_latest_for_display(mut value: LatestEventWithBody) -> LatestEventWithBody {
    value.event = value.event.map(redact_event_for_display);
    value
}
