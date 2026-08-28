//! Pump tests: the event mapping, and the one-`Settled` pairing.
//!
//! `pump_run` takes any stream of run events, so these tests need no provider and no
//! network. That is deliberate: it is the only way to build the stream shape that a
//! provider failure produces, where rho-core emits no end event at all.

use futures::stream;

use rho_core::{
    AgentEvent, AgentStopReason, ContentBlock, Error, ProviderError, StopReason, StreamEvent,
    ToolError, ToolKind, ToolOutput,
};
use rho_jsonl::{Event, FaultKind, SettleReason, Writer, map_event, pump_run};

/// A writer over a shared buffer, so a test can read what was written.
#[derive(Clone, Default)]
struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl tokio::io::AsyncWrite for Shared {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.0.lock().expect("buffer").extend_from_slice(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl Shared {
    fn events(&self) -> Vec<Event> {
        let bytes = self.0.lock().expect("buffer").clone();
        String::from_utf8(bytes)
            .expect("valid utf8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("every line is an event"))
            .collect()
    }
}

/// Pump a scripted stream and return every event it wrote.
async fn run(items: Vec<Result<AgentEvent, Error>>) -> (Vec<Event>, rho_jsonl::RunOutcome) {
    let shared = Shared::default();
    let writer = Writer::new(shared.clone());
    let stream = stream::iter(items);
    let mut stream = std::pin::pin!(stream);
    let outcome = pump_run(&mut stream, &writer)
        .await
        .expect("writing to a vector cannot fail");
    (shared.events(), outcome)
}

fn settled_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::Settled { .. }))
        .count()
}

#[tokio::test]
async fn settled_is_the_last_event_of_a_run() {
    let (events, outcome) = run(vec![
        Ok(AgentEvent::TurnStart),
        Ok(AgentEvent::TurnEnd {
            stop_reason: StopReason::EndTurn,
        }),
        Ok(AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        }),
    ])
    .await;
    assert_eq!(
        events.last(),
        Some(&Event::Settled {
            stop_reason: SettleReason::EndTurn
        })
    );
    assert_eq!(settled_count(&events), 1);
    assert!(!outcome.settled_by_frontend);
}

#[tokio::test]
async fn no_event_arrives_after_settled() {
    // rho-core should never do this, but the frontend owns the promise, so it must
    // hold even when the stream keeps talking after its end event.
    let (events, _) = run(vec![
        Ok(AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        }),
        Ok(AgentEvent::Stream(StreamEvent::TextDelta {
            index: 0,
            delta: "late".to_string(),
        })),
    ])
    .await;
    assert_eq!(events.len(), 1, "the pump must stop at Settled: {events:?}");
}

#[tokio::test]
async fn a_faulting_run_still_settles_once() {
    // This is the shape rho-core really produces on a provider failure: one Err, and
    // no AgentEnd at all. A client that waits for Settled would otherwise wait for ever.
    let (events, outcome) = run(vec![
        Ok(AgentEvent::TurnStart),
        Err(Error::Provider(ProviderError::Server { status: 503 })),
    ])
    .await;
    assert_eq!(
        settled_count(&events),
        1,
        "a faulting run must settle: {events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&Event::Settled {
            stop_reason: SettleReason::Faulted
        })
    );
    assert!(outcome.settled_by_frontend);
    // The fault comes before the settle, and it says why.
    assert!(matches!(
        events[1],
        Event::Fault {
            kind: FaultKind::Provider,
            ..
        }
    ));
    // Exactly one fault. A second one for the same run would be noise.
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Fault { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn a_silent_stream_end_is_an_incomplete_fault() {
    // The stream ends with neither an end event nor an error. The pump must still
    // settle, and it must say that nothing explained the end.
    let (events, outcome) = run(vec![Ok(AgentEvent::TurnStart)]).await;
    assert_eq!(settled_count(&events), 1);
    assert!(outcome.settled_by_frontend);
    assert!(
        matches!(
            events[1],
            Event::Fault {
                kind: FaultKind::Incomplete,
                ..
            }
        ),
        "{events:?}"
    );
}

#[tokio::test]
async fn every_accepted_prompt_settles_exactly_once() {
    // The invariant, not one example. Three runs of three different shapes, and the
    // count of Settled events must equal the count of runs.
    let shapes: Vec<Vec<Result<AgentEvent, Error>>> = vec![
        vec![Ok(AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        })],
        vec![Err(Error::Provider(ProviderError::Transport(
            "down".into(),
        )))],
        vec![Ok(AgentEvent::TurnStart)],
    ];
    let mut total = 0;
    for shape in shapes {
        let (events, _) = run(shape).await;
        total += settled_count(&events);
    }
    assert_eq!(total, 3, "each run settles exactly once");
}

#[tokio::test]
async fn every_error_variant_has_a_named_fault_kind() {
    // `pump_run` takes any stream of run events, so it must name every rho_core::Error
    // case. The map has no wildcard arm, so a new Error variant is a compile error
    // rather than a fault labelled as something it is not.
    let cases = vec![
        (
            Error::Provider(ProviderError::Auth("no key".into())),
            FaultKind::Provider,
        ),
        (Error::Tool(ToolError::Denied), FaultKind::Tool),
        (Error::Canceled, FaultKind::Canceled),
    ];
    for (error, expected) in cases {
        let (events, _) = run(vec![Err(error)]).await;
        match &events[0] {
            Event::Fault { kind, message } => {
                assert_eq!(*kind, expected);
                assert!(!message.is_empty(), "a fault must carry prose");
            }
            other => panic!("expected a fault, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_tool_failure_is_reported_as_not_ok() {
    // rho-core turns a tool failure into an output with is_error, so the wire reports
    // the inverse. Getting this backwards would tell a client every tool failed.
    let (events, _) = run(vec![
        Ok(AgentEvent::ToolEnd {
            id: "t1".to_string(),
            output: ToolOutput::text("all good"),
        }),
        Ok(AgentEvent::ToolEnd {
            id: "t2".to_string(),
            output: ToolOutput {
                content: vec![ContentBlock::Text {
                    text: "it broke".to_string(),
                }],
                is_error: true,
            },
        }),
        Ok(AgentEvent::AgentEnd {
            stop_reason: AgentStopReason::EndTurn,
        }),
    ])
    .await;
    assert_eq!(
        events[0],
        Event::ToolEnd {
            id: "t1".to_string(),
            ok: true
        }
    );
    assert_eq!(
        events[1],
        Event::ToolEnd {
            id: "t2".to_string(),
            ok: false
        }
    );
}

#[test]
fn the_event_map_drops_only_what_the_spec_lists() {
    // Every mapped variant, with its wire shape.
    assert_eq!(map_event(&AgentEvent::TurnStart), Some(Event::TurnStart));
    assert_eq!(
        map_event(&AgentEvent::TurnEnd {
            stop_reason: StopReason::ToolUse
        }),
        Some(Event::TurnEnd {
            stop_reason: StopReason::ToolUse
        })
    );
    assert_eq!(
        map_event(&AgentEvent::Stream(StreamEvent::TextDelta {
            index: 3,
            delta: "x".to_string()
        })),
        Some(Event::TextDelta {
            index: 3,
            delta: "x".to_string()
        })
    );
    assert_eq!(
        map_event(&AgentEvent::ToolStart {
            id: "t".to_string(),
            name: "bash".to_string(),
            kind: ToolKind::Execute
        }),
        Some(Event::ToolStart {
            id: "t".to_string(),
            name: "bash".to_string(),
            kind: ToolKind::Execute
        })
    );
    assert_eq!(
        map_event(&AgentEvent::ToolUpdate {
            id: "t".to_string(),
            output: "line".to_string()
        }),
        Some(Event::ToolUpdate {
            id: "t".to_string(),
            output: "line".to_string()
        })
    );
    assert_eq!(
        map_event(&AgentEvent::MessageQueued { position: 2 }),
        Some(Event::MessageQueued { position: 2 })
    );
    assert_eq!(
        map_event(&AgentEvent::MessageDelivered { count: 5 }),
        Some(Event::MessageDelivered { count: 5 })
    );

    // Dropped on purpose. Each one is named in the spec's out-of-scope list, and the
    // test name promises the list is complete, so every one of them is asserted.
    assert_eq!(
        map_event(&AgentEvent::Stream(StreamEvent::ThinkingDelta {
            index: 0,
            delta: "hmm".to_string()
        })),
        None
    );
    assert_eq!(
        map_event(&AgentEvent::Stream(StreamEvent::Usage(Default::default()))),
        None
    );
    let task = rho_core::TaskId("t1".to_string());
    assert_eq!(
        map_event(&AgentEvent::TaskStart {
            id: task.clone(),
            command: "sleep 1".to_string(),
            reason: rho_core::BackgroundReason::ModelRequested,
        }),
        None
    );
    assert_eq!(
        map_event(&AgentEvent::TaskProgressed {
            id: task.clone(),
            progress: rho_core::TaskProgress::default(),
        }),
        None
    );
    assert_eq!(
        map_event(&AgentEvent::TaskEnd {
            id: task,
            state: rho_core::TaskState::Exited { code: 0 },
            output_tail: String::new(),
        }),
        None
    );
    let agent = rho_core::AgentId(1);
    assert_eq!(
        map_event(&AgentEvent::AgentSpawned {
            id: agent,
            agent: "scout".to_string(),
            depth: 1,
        }),
        None
    );
    assert_eq!(
        map_event(&AgentEvent::AgentProgressed {
            id: agent,
            turns: 2,
            usage: Default::default(),
        }),
        None
    );
    assert_eq!(
        map_event(&AgentEvent::AgentFinished {
            id: agent,
            report: rho_core::AgentReport {
                agent: "scout".to_string(),
                outcome: rho_core::AgentOutcome::Done,
                summary: "done".to_string(),
                usage: Default::default(),
                turns: 1,
                gate: Default::default(),
                claims: Default::default(),
                transcript: None,
            },
        }),
        None
    );
}
