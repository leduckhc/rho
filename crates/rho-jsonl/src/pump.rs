//! The event pump: it maps one run's events onto the wire and settles it.
//!
//! This module owns the promise a client relies on: **exactly one `Settled` for every
//! accepted prompt**. `rho-core` cannot keep that promise, because a run that fails at
//! the provider returns early and emits no `AgentEnd`. See
//! decision D-the-frontend-settles-every-prompt.

use futures::{Stream, StreamExt};
use tokio::io::AsyncWrite;

use rho_core::{AgentEvent, StreamEvent};

use crate::protocol::{Event, FaultKind, SettleReason};
use crate::writer::Writer;

/// Map one core event onto its wire event. `None` means the variant is out of scope
/// for this protocol, and the frontend drops it. See SPEC-jsonl-frontend section 7.
///
/// The match has no wildcard arm. A new `AgentEvent` variant is a compile error here,
/// so a new kind of event cannot be dropped in silence. That is the same rule
/// `ToolKind::is_read_only` follows, and it is why the out-of-scope variants are
/// listed one by one instead of behind a `_`.
pub fn map_event(event: &AgentEvent) -> Option<Event> {
    match event {
        AgentEvent::TurnStart => Some(Event::TurnStart),
        AgentEvent::TurnEnd { stop_reason } => Some(Event::TurnEnd {
            stop_reason: *stop_reason,
        }),
        AgentEvent::Stream(StreamEvent::TextDelta { index, delta }) => Some(Event::TextDelta {
            index: *index,
            delta: delta.clone(),
        }),
        AgentEvent::ToolStart { id, name, kind } => Some(Event::ToolStart {
            id: id.clone(),
            name: name.clone(),
            kind: *kind,
        }),
        AgentEvent::ToolUpdate { id, output } => Some(Event::ToolUpdate {
            id: id.clone(),
            output: output.clone(),
        }),
        AgentEvent::ToolEnd { id, output } => Some(Event::ToolEnd {
            id: id.clone(),
            // A tool failure is a result, not the end of the run. rho-core turns it
            // into an output with `is_error`, so the wire reports the inverse.
            ok: !output.is_error,
        }),
        AgentEvent::MessageQueued { position } => Some(Event::MessageQueued {
            position: *position,
        }),
        AgentEvent::MessageDelivered { count } => Some(Event::MessageDelivered { count: *count }),
        AgentEvent::AgentEnd { stop_reason } => Some(Event::Settled {
            stop_reason: SettleReason::from(*stop_reason),
        }),
        // Out of scope for this spec. Every other `StreamEvent` is a lower-level
        // detail the wire does not carry: thinking, usage, and the tool-call
        // assembly that `ToolStart` already reports.
        AgentEvent::Stream(_) => None,
        // Out of scope: background tasks and subagents. Each needs its own event
        // family, and adding one is an extension. See SPEC-jsonl-frontend section 7.
        //
        // **A warning for whoever wires the task bridge here.** `TaskRegistry::session_events`
        // now gives a frontend every task event, and the terminal draws a row from it. Feeding
        // that stream into this pump would drop all three arms below, at runtime, in silence.
        // The no-wildcard match catches a *new* variant; it cannot catch these three, because
        // they already return `None`. A task event also carries no `Settled`, so it sits
        // outside this pump's one-`Settled`-per-prompt rule. Give the wire a task event family
        // first, in a spec, because a wire format binds every client. See
        // `SPEC-the-task-event-bridge` section 8.
        AgentEvent::TaskStart { .. }
        | AgentEvent::TaskProgressed { .. }
        | AgentEvent::TaskEnd { .. }
        | AgentEvent::AgentSpawned { .. }
        | AgentEvent::AgentProgressed { .. }
        | AgentEvent::AgentFinished { .. } => None,
    }
}

/// What the pump did with one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
    /// The reason written in the one `Settled` event.
    pub settled_as: SettleReason,
    /// True when the pump had to settle the run itself, because the stream ended
    /// with no `AgentEnd`.
    pub settled_by_frontend: bool,
}

/// Drive one run's events onto the writer, and settle it exactly once.
///
/// It returns as soon as it writes `Settled`, so no event for this run can follow
/// it. When the stream ends with no `AgentEnd`, the pump writes a `Fault` and then
/// `Settled { stop_reason: faulted }`.
///
/// It takes any stream of run events, not only `rho_core::AgentEvents`. That is what
/// makes the pairing testable: a test builds a stream that ends the way a provider
/// failure ends, with no live provider and no network.
pub async fn pump_run<S, W>(mut events: S, out: &Writer<W>) -> std::io::Result<RunOutcome>
where
    S: Stream<Item = Result<AgentEvent, rho_core::Error>> + Unpin,
    W: AsyncWrite + Unpin,
{
    while let Some(item) = events.next().await {
        match item {
            Ok(event) => {
                let Some(wire) = map_event(&event) else {
                    continue;
                };
                if let Event::Settled { stop_reason } = wire {
                    out.event(&Event::Settled { stop_reason }).await?;
                    // Return now. The run is over, so nothing may follow, and a
                    // caller that waited for the stream to end could reject the
                    // client's next prompt for a run that had already settled.
                    return Ok(RunOutcome {
                        settled_as: stop_reason,
                        settled_by_frontend: false,
                    });
                }
                out.event(&wire).await?;
            }
            Err(error) => {
                out.event(&Event::Fault {
                    kind: FaultKind::from(&error),
                    message: error.to_string(),
                })
                .await?;
                // An error on the event stream ends the run, so settle here.
                //
                // rho-core returns `TurnOutcome::Failed` at every site that sends an
                // error, and the driver loop then returns with no `AgentEnd`. Waiting
                // for the stream to close instead would hang: that early return skips
                // `queue.unobserve()`, so the task that forwards queue announcements
                // keeps a clone of the event sender alive and the stream never ends.
                // A live probe found that hang. See
                // D-an-error-on-the-event-stream-ends-the-run.
                out.event(&Event::Settled {
                    stop_reason: SettleReason::Faulted,
                })
                .await?;
                return Ok(RunOutcome {
                    settled_as: SettleReason::Faulted,
                    settled_by_frontend: true,
                });
            }
        }
    }

    // The stream ended with no `AgentEnd` and no error. Settle it here, because the
    // client is waiting and only this crate can keep that promise.
    out.event(&Event::Fault {
        kind: FaultKind::Incomplete,
        message: "the run ended with no result and no error".to_string(),
    })
    .await?;
    out.event(&Event::Settled {
        stop_reason: SettleReason::Faulted,
    })
    .await?;
    Ok(RunOutcome {
        settled_as: SettleReason::Faulted,
        settled_by_frontend: true,
    })
}
