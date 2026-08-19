use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;

use crate::agent::{AgentEvent, AgentEvents, AgentStopReason};
use crate::cancel::CancelToken;
use crate::event::StreamEvent;
use crate::subagent::report::{AgentOutcome, AgentReport, MAX_SUMMARY_CHARS};
use crate::subagent::tree::AgentProgress;
use crate::usage::Usage;

// --- Running a child to a report (SPEC-subagents section 6) ---

/// Drive a child's event stream to an [`AgentReport`].
///
/// It sums usage, counts turns, keeps the child's final answer as the capped
/// summary, and writes the full transcript to `transcript_path`. The transcript
/// never reaches the model, only the summary does. See `SPEC-subagents` section 6.
///
/// A child that passes `timeout` is cancelled through the shared token and
/// reported `Canceled`. A child that ends without a report is reported `Failed`.
/// A child failure is a result, not the end of the parent's run. See decision
/// D-measured-cost-and-cache.
/// What [`collect_report`] needs besides the stream.
///
/// A struct, not three more parameters. The argument list already carried three
/// things, and a fourth and fifth would repeat the mistake in decision
/// D-no-four-argument-session-new. A new need arrives as a new field with a default.
#[derive(Default)]
pub struct CollectOptions {
    /// How long the child may run before it is cancelled.
    pub timeout: Duration,
    /// Where to write the child's full transcript, for a human.
    pub transcript_path: Option<PathBuf>,
    /// Where to publish progress **as the child works**.
    ///
    /// Without this, a handle read zero for the whole run and then jumped to the
    /// final number. That is a post-mortem, and the event is called
    /// `AgentProgressed`. See `SPEC-subagents` section 9.
    pub progress: Option<tokio::sync::watch::Sender<AgentProgress>>,
}

impl CollectOptions {
    /// The options with a timeout and nothing else.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::default()
        }
    }

    /// Write the transcript here.
    pub fn transcript(mut self, path: PathBuf) -> Self {
        self.transcript_path = Some(path);
        self
    }

    /// Publish progress here, turn by turn.
    pub fn publishing(mut self, sender: tokio::sync::watch::Sender<AgentProgress>) -> Self {
        self.progress = Some(sender);
        self
    }
}

pub async fn collect_report(
    agent: impl Into<String>,
    mut events: AgentEvents,
    cancel: CancelToken,
    options: CollectOptions,
) -> AgentReport {
    let CollectOptions {
        timeout,
        transcript_path,
        progress,
    } = options;
    let agent = agent.into();
    let mut usage = Usage::default();
    let mut turns = 0u32;
    let mut current_text = String::new();
    let mut last_answer: Option<String> = None;
    // A streaming writer, not a buffer. The run a transcript is most wanted for is
    // the one that did not finish, and a buffer loses exactly that one. A writer that
    // cannot open is `None`: a transcript is a convenience and must never fail a run.
    let transcript = transcript_path.clone().and_then(|path| {
        crate::TranscriptWriter::new(&path)
            .map_err(|error| {
                tracing::warn!(path = %path.display(), "cannot open the child transcript: {error}");
            })
            .ok()
    });
    let mut outcome: Option<AgentOutcome> = None;

    let sleep = tokio::time::sleep(timeout);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            biased;
            () = &mut sleep => {
                // The child passed its timeout. Cancel it through the shared
                // token, then report what it had.
                cancel.cancel();
                outcome = Some(AgentOutcome::Canceled);
                break;
            }
            item = events.next() => {
                match item {
                    Some(Ok(event)) => {
                        if let Some(writer) = transcript.as_ref() {
                            // `StreamEvent::TextEnd` carries only an index, so the
                            // text comes from what this loop already accumulated.
                            let body = match &event {
                                AgentEvent::TurnStart => Some(crate::TranscriptBody::TurnStart {
                                    turn: turns + 1,
                                }),
                                AgentEvent::Stream(StreamEvent::TextEnd { .. })
                                    if !current_text.is_empty() =>
                                {
                                    Some(crate::TranscriptBody::Text {
                                        text: current_text.clone(),
                                    })
                                }
                                AgentEvent::ToolStart { id, name, .. } => {
                                    Some(crate::TranscriptBody::ToolStart {
                                        id: id.clone(),
                                        name: name.clone(),
                                    })
                                }
                                AgentEvent::ToolUpdate { id, output } => {
                                    Some(crate::TranscriptBody::ToolUpdate {
                                        id: id.clone(),
                                        line: output.clone(),
                                    })
                                }
                                AgentEvent::ToolEnd { id, output } => {
                                    Some(crate::TranscriptBody::ToolEnd {
                                        id: id.clone(),
                                        error: output.is_error,
                                    })
                                }
                                AgentEvent::Stream(StreamEvent::Usage(reported)) => {
                                    Some(crate::TranscriptBody::Usage {
                                        input: reported.input_tokens,
                                        output: reported.output_tokens,
                                    })
                                }
                                _ => None,
                            };
                            if let Some(body) = body {
                                let _ = writer
                                    .write(&crate::TranscriptEntry::now(&agent, body))
                                    .await;
                            }
                        }
                        match &event {
                            AgentEvent::TurnStart => {
                                turns += 1;
                                current_text.clear();
                                // Publish as the child works, not once at the end. A
                                // failed send means nobody is watching.
                                if let Some(sender) = &progress {
                                    let _ = sender.send(AgentProgress {
                                        turns,
                                        usage,
                                    });
                                }
                            }
                            AgentEvent::Stream(StreamEvent::TextDelta { delta, .. }) => {
                                current_text.push_str(delta);
                            }
                            AgentEvent::Stream(StreamEvent::TextEnd { .. })
                                if !current_text.is_empty() =>
                            {
                                last_answer = Some(std::mem::take(&mut current_text));
                            }
                            AgentEvent::Stream(StreamEvent::Usage(reported)) => {
                                usage.add(reported);
                                if let Some(sender) = &progress {
                                    let _ = sender.send(AgentProgress { turns, usage });
                                }
                            }
                            AgentEvent::AgentEnd { stop_reason } => {
                                outcome = Some(outcome_from_stop(*stop_reason));
                                break;
                            }
                            _ => {}
                        }
                    }
                    // A transport or provider fault. A child failure is a result.
                    Some(Err(error)) => {
                        outcome = Some(AgentOutcome::Failed {
                            reason: error.to_string(),
                        });
                        break;
                    }
                    // The stream ended with no `AgentEnd`. The child died holding
                    // work. Silence is the failure mode that wastes the most time.
                    None => break,
                }
            }
        }
    }

    let outcome = outcome.unwrap_or_else(|| AgentOutcome::Failed {
        reason: "the child ended without a report.".to_string(),
    });
    let summary = cap_summary(last_answer.unwrap_or_default());
    // One last line, naming the outcome, then hand back the path. The path is `Some`
    // only when the file really opened, so a caller is never pointed at nothing.
    let transcript = match transcript.as_ref() {
        Some(writer) => {
            let _ = writer
                .write(&crate::TranscriptEntry::now(
                    &agent,
                    crate::TranscriptBody::End {
                        outcome: outcome.wire_name().to_string(),
                    },
                ))
                .await;
            Some(writer.path.clone())
        }
        None => None,
    };
    let _ = transcript_path;

    AgentReport {
        agent,
        outcome,
        summary,
        usage,
        turns,
        // `collect_report` watches a stream. It does not run the gate, because a
        // gate needs a sandboxed command runner that `rho-core` must not hold. A
        // caller runs the gate and merges the verdict.
        gate: crate::GateReport::default(),
        claims: crate::ChildClaims::default(),
        transcript,
    }
}

/// Map a run's stop reason onto a child outcome.
fn outcome_from_stop(stop_reason: AgentStopReason) -> AgentOutcome {
    match stop_reason {
        AgentStopReason::EndTurn => AgentOutcome::Done,
        AgentStopReason::MaxTurnRequests => AgentOutcome::OutOfTurns,
        // The child spent its tool-call budget. It is out of room, like a child
        // out of turns, so the parent gets what it had rather than nothing.
        AgentStopReason::MaxToolCalls => AgentOutcome::OutOfTurns,
        AgentStopReason::Canceled => AgentOutcome::Canceled,
        AgentStopReason::MaxTokens => AgentOutcome::Failed {
            reason: "the child hit the token limit.".to_string(),
        },
        AgentStopReason::Refusal => AgentOutcome::Failed {
            reason: "the model refused, or a content filter stopped the output.".to_string(),
        },
    }
}

/// Truncate a summary to [`MAX_SUMMARY_CHARS`] characters. It cuts on a character
/// boundary, so a multi-byte character never splits.
fn cap_summary(mut summary: String) -> String {
    if summary.chars().count() <= MAX_SUMMARY_CHARS {
        return summary;
    }
    let cut = summary
        .char_indices()
        .nth(MAX_SUMMARY_CHARS)
        .map(|(index, _)| index)
        .unwrap_or(summary.len());
    summary.truncate(cut);
    summary
}
