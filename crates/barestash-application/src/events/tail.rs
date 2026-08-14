use std::future::Future;
use std::time::Duration;

use barestash_protocol::EventMetadata;

use super::fetch::list_events;
use crate::{AppContext, CliError};

#[derive(Debug, Clone, Copy)]
pub(super) struct TailOptions {
    pub last: u64,
    pub poll_interval: Duration,
    /// `None` for the production infinite tail; finite values support tests and
    /// embedded agent callers without changing polling semantics.
    pub max_polls: Option<usize>,
}

impl TailOptions {
    pub(super) const fn watching(last: u64, poll_interval: Duration) -> Self {
        Self {
            last,
            poll_interval,
            max_polls: None,
        }
    }
}

pub(super) async fn tail_events<Ready, Event, EventFuture>(
    context: &AppContext,
    endpoint_id: &str,
    options: TailOptions,
    mut on_ready: Ready,
    mut on_event: Event,
) -> Result<(), CliError>
where
    Ready: FnMut() -> Result<(), CliError>,
    Event: FnMut(EventMetadata) -> EventFuture,
    EventFuture: Future<Output = Result<(), CliError>>,
{
    let mut cursor = None;

    if options.last > 0 {
        let last = options.last.to_string();
        let initial = list_events(context, endpoint_id, Some(&last), None).await?;
        on_ready()?;

        for event in initial.into_iter().rev() {
            let event_id = event.id.clone();
            on_event(event).await?;
            cursor = Some(event_id);
        }
    } else {
        cursor = list_events(context, endpoint_id, Some("1"), None)
            .await?
            .into_iter()
            .next()
            .map(|event| event.id);
        on_ready()?;
    }

    let mut polls = 0usize;
    while options.max_polls.is_none_or(|maximum| polls < maximum) {
        if polls > 0 || options.last > 0 {
            tokio::time::sleep(options.poll_interval).await;
        }

        let requested_after = cursor.clone();
        let mut events =
            list_events(context, endpoint_id, None, requested_after.as_deref()).await?;

        if requested_after.is_none() {
            events.reverse();
        }

        for event in events {
            let event_id = event.id.clone();
            on_event(event).await?;
            cursor = Some(event_id);
        }
        polls += 1;
    }

    Ok(())
}

pub(super) fn parse_tail_last(value: &str) -> Result<u64, CliError> {
    let value = value.trim();
    let prefixed = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|digits| u64::from_str_radix(digits, 16))
        .or_else(|| {
            value
                .strip_prefix("0b")
                .or_else(|| value.strip_prefix("0B"))
                .map(|digits| u64::from_str_radix(digits, 2))
        })
        .or_else(|| {
            value
                .strip_prefix("0o")
                .or_else(|| value.strip_prefix("0O"))
                .map(|digits| u64::from_str_radix(digits, 8))
        });
    if let Some(parsed) = prefixed {
        return parsed
            .map_err(|_| CliError::Local("--last must be a non-negative integer.".into()));
    }
    let parsed = value
        .parse::<f64>()
        .map_err(|_| CliError::Local("--last must be a non-negative integer.".into()))?;
    if !parsed.is_finite() || parsed < 0.0 || parsed.fract() != 0.0 || parsed > u64::MAX as f64 {
        return Err(CliError::Local(
            "--last must be a non-negative integer.".into(),
        ));
    }
    Ok(parsed as u64)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::events::test_support::{context, event};

    #[tokio::test]
    async fn tail_without_last_suppresses_existing_event_and_uses_after_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events"))
            .and(query_param("limit", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [event("evt_existing")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events"))
            .and(query_param("after", "evt_existing"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [event("evt_new")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (context, _directory) = context(&server);
        let output = Arc::new(Mutex::new(Vec::new()));
        let callback_output = output.clone();

        tail_events(
            &context,
            "ep_test",
            TailOptions {
                last: 0,
                poll_interval: Duration::ZERO,
                max_polls: Some(1),
            },
            || Ok(()),
            move |event| {
                callback_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(event.id);
                async { Ok(()) }
            },
        )
        .await
        .unwrap_or_else(|error| panic!("tail succeeds: {error}"));

        assert_eq!(
            *output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec!["evt_new"]
        );
    }

    #[tokio::test]
    async fn tail_last_events_are_emitted_oldest_first() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/endpoints/ep_test/events"))
            .and(query_param("limit", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "events": [event("evt_newer"), event("evt_older")]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (context, _directory) = context(&server);
        let output = Arc::new(Mutex::new(Vec::new()));
        let callback_output = output.clone();

        tail_events(
            &context,
            "ep_test",
            TailOptions {
                last: 2,
                poll_interval: Duration::ZERO,
                max_polls: Some(0),
            },
            || Ok(()),
            move |event| {
                callback_output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(event.id);
                async { Ok(()) }
            },
        )
        .await
        .unwrap_or_else(|error| panic!("tail succeeds: {error}"));

        assert_eq!(
            *output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec!["evt_older", "evt_newer"]
        );
    }

    #[test]
    fn tail_last_accepts_javascript_style_integer_prefixes() {
        assert_eq!(parse_tail_last("0x10").ok(), Some(16));
        assert_eq!(parse_tail_last("0b10").ok(), Some(2));
        assert_eq!(parse_tail_last("0o10").ok(), Some(8));
    }

    #[test]
    fn tail_last_rejects_negative_fractional_and_non_finite_values() {
        for value in ["-1", "1.5", "NaN", "Infinity"] {
            assert!(matches!(parse_tail_last(value), Err(CliError::Local(_))));
        }
    }
}
