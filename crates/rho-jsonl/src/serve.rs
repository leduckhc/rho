//! The serve loop.
//!
//! It reads one command per line, replies to each one exactly once, and pumps a
//! run's events onto the same stream. See SPEC-jsonl-frontend section 5 for the
//! ordering rules a client may trust.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

use rho_core::{CancelToken, ContentBlock, Session};

use crate::dialog::{Asker, DialogHost};
use crate::factory::{FactoryError, SessionFactory, SessionRequest};
use crate::frame::{Line, LineReader};
use crate::protocol::{Command, Reply, ReplyError};
use crate::pump::pump_run;
use crate::writer::Writer;

/// Serve the protocol over one pair of streams.
///
/// It returns when the input reaches end of file. A run that is still going when the
/// input closes is drained to its `Settled` event first, because an accepted prompt
/// always settles. See D-the-frontend-settles-every-prompt.
pub async fn serve<R, W>(
    factory: Arc<dyn SessionFactory>,
    start: SessionRequest,
    input: R,
    output: W,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + Sync + 'static,
{
    let out = Writer::new(output);
    let dialogs = DialogHost::new(out.clone());
    let asker: Arc<dyn Asker> = Arc::new(dialogs.clone());

    let mut current = start;
    let mut session = match factory.build(&current, Arc::clone(&asker)).await {
        Ok(session) => session,
        Err(error) => {
            // The first session could not be built, so there is nothing to serve.
            // Say why on the stream, then stop. A silent exit would look like a crash.
            let (case, _) = classify(&error);
            out.reply(&Reply::err("start", None, case, error.to_string()))
                .await?;
            return Ok(());
        }
    };

    let mut reader = LineReader::new(input);
    // True once an over-long line was refused. It collapses a run of `TooLong`
    // records for one enormous line into one reply.
    let mut refused_line = false;

    loop {
        let line = match reader.next_line().await? {
            Some(line) => line,
            // End of file. No run is going here, because a run is drained below
            // before the loop comes back round.
            None => return Ok(()),
        };

        let bytes = match line {
            Line::Record(bytes) => {
                refused_line = false;
                bytes
            }
            Line::TooLong { bytes } => {
                if !refused_line {
                    refused_line = true;
                    out.reply(&Reply::err(
                        "unknown",
                        None,
                        ReplyError::LineTooLong,
                        format!(
                            "a command line passed the {} byte cap after {bytes} bytes",
                            crate::frame::MAX_COMMAND_LINE_BYTES
                        ),
                    ))
                    .await?;
                }
                continue;
            }
        };

        let command = match serde_json::from_slice::<Command>(&bytes) {
            Ok(command) => command,
            Err(error) => {
                // An unknown `type` is its own case, so a client can probe for a
                // command instead of guessing from a version number. Every other
                // shape failure is a parse error.
                let case = if error.to_string().starts_with("unknown variant") {
                    ReplyError::UnknownCommand
                } else {
                    ReplyError::ParseError
                };
                out.reply(&Reply::err("unknown", None, case, error.to_string()))
                    .await?;
                continue;
            }
        };

        let name = command.name();
        let req_id = command.req_id().map(str::to_string);

        match command {
            Command::Prompt { message, .. } => {
                let cancel = CancelToken::new();
                let events =
                    session.prompt(vec![ContentBlock::Text { text: message }], cancel.clone());
                // Reply before any event of this run, so a client can correlate.
                out.reply(&Reply::ok(name, req_id)).await?;
                drain_run(&mut reader, &out, &dialogs, &session, events, cancel).await?;
            }
            Command::Steer { message, .. } => {
                match session.steer(vec![ContentBlock::Text { text: message }]) {
                    Ok(position) => {
                        out.reply(&Reply::ok_with(
                            name,
                            req_id,
                            serde_json::json!({ "position": position }),
                        ))
                        .await?;
                    }
                    Err(error) => {
                        out.reply(&Reply::err(
                            name,
                            req_id,
                            ReplyError::QueueFull,
                            error.to_string(),
                        ))
                        .await?;
                    }
                }
            }
            Command::Abort { .. } => {
                // No run is going in this branch, because a run is drained inside the
                // `Prompt` arm. An abort with no run is not an error: a client that
                // races a settling run must not get a failure for a run that ended.
                out.reply(&Reply::ok_with(
                    name,
                    req_id,
                    serde_json::json!({ "running": false }),
                ))
                .await?;
            }
            Command::GetState { .. } => {
                out.reply(&Reply::ok_with(name, req_id, state(&current, false)))
                    .await?;
            }
            Command::SetModel {
                provider, model_id, ..
            } => {
                let wanted = SessionRequest::new(provider, model_id);
                match factory.build(&wanted, Arc::clone(&asker)).await {
                    Ok(fresh) => {
                        session = fresh;
                        current = wanted;
                        out.reply(&Reply::ok_with(
                            name,
                            req_id,
                            serde_json::json!({
                                "provider": current.provider,
                                "model_id": current.model_id,
                            }),
                        ))
                        .await?;
                    }
                    Err(error) => {
                        // The old session is untouched, so the client can carry on.
                        let (case, _) = classify(&error);
                        out.reply(&Reply::err(name, req_id, case, error.to_string()))
                            .await?;
                    }
                }
            }
            Command::NewSession { .. } => match factory.build(&current, Arc::clone(&asker)).await {
                Ok(fresh) => {
                    session = fresh;
                    out.reply(&Reply::ok(name, req_id)).await?;
                }
                Err(error) => {
                    let (case, _) = classify(&error);
                    out.reply(&Reply::err(name, req_id, case, error.to_string()))
                        .await?;
                }
            },
            Command::GetMessages { .. } => {
                let messages = session.messages().await;
                match serde_json::to_value(&messages) {
                    Ok(value) => {
                        out.reply(&Reply::ok_with(
                            name,
                            req_id,
                            serde_json::json!({ "messages": value }),
                        ))
                        .await?;
                    }
                    Err(error) => {
                        out.reply(&Reply::err(
                            name,
                            req_id,
                            ReplyError::Internal,
                            error.to_string(),
                        ))
                        .await?;
                    }
                }
            }
            Command::GetCommands { .. } => {
                out.reply(&Reply::ok_with(
                    name,
                    req_id,
                    serde_json::json!({ "commands": Command::NAMES }),
                ))
                .await?;
            }
            Command::DialogResponse { id, answer, .. } => {
                // A dialog answer outside a run has no open dialog to match, so it is
                // dropped. The reply says whether it landed.
                let delivered = dialogs.answer(&id, answer);
                out.reply(&Reply::ok_with(
                    name,
                    req_id,
                    serde_json::json!({ "delivered": delivered }),
                ))
                .await?;
            }
        }
    }
}

/// Pump one run to its `Settled` event, and serve the commands that may arrive during it.
///
/// Only three commands do work here. `Steer` queues a message, `Abort` cancels, and
/// `DialogResponse` unblocks a dialog. Everything that would replace the session is
/// refused with `AlreadyStreaming`, so a run never loses the session under it.
async fn drain_run<R, W>(
    reader: &mut LineReader<R>,
    out: &Writer<W>,
    dialogs: &DialogHost<W>,
    session: &Session,
    events: rho_core::AgentEvents,
    cancel: CancelToken,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + Sync + 'static,
{
    let mut events = events;
    // The pump owns the one-`Settled` promise, so it runs as a future here and the
    // loop below only feeds it commands.
    let pump = pump_run(&mut events, out);
    let mut pump = std::pin::pin!(pump);

    loop {
        tokio::select! {
            // The run finished. Return at once, so no command is served after
            // `Settled` and a client's next prompt is never refused for a run that
            // has already ended.
            outcome = &mut pump => {
                outcome?;
                return Ok(());
            }
            line = reader.next_line() => {
                match line? {
                    // The client closed stdin during a run. Keep pumping, because an
                    // accepted prompt always settles.
                    None => {
                        (&mut pump).await?;
                        return Ok(());
                    }
                    Some(Line::TooLong { bytes }) => {
                        out.reply(&Reply::err(
                            "unknown",
                            None,
                            ReplyError::LineTooLong,
                            format!("a command line passed the cap after {bytes} bytes"),
                        ))
                        .await?;
                    }
                    Some(Line::Record(bytes)) => {
                        serve_during_run(&bytes, out, dialogs, session, &cancel).await?;
                    }
                }
            }
        }
    }
}

/// Serve one command line that arrived while a run was going.
async fn serve_during_run<W>(
    bytes: &[u8],
    out: &Writer<W>,
    dialogs: &DialogHost<W>,
    session: &Session,
    cancel: &CancelToken,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin + Send + Sync + 'static,
{
    let command = match serde_json::from_slice::<Command>(bytes) {
        Ok(command) => command,
        Err(error) => {
            let case = if error.to_string().starts_with("unknown variant") {
                ReplyError::UnknownCommand
            } else {
                ReplyError::ParseError
            };
            return out
                .reply(&Reply::err("unknown", None, case, error.to_string()))
                .await;
        }
    };

    let name = command.name();
    let req_id = command.req_id().map(str::to_string);

    match command {
        Command::Steer { message, .. } => {
            match session.steer(vec![ContentBlock::Text { text: message }]) {
                Ok(position) => {
                    out.reply(&Reply::ok_with(
                        name,
                        req_id,
                        serde_json::json!({ "position": position }),
                    ))
                    .await
                }
                Err(error) => {
                    out.reply(&Reply::err(
                        name,
                        req_id,
                        ReplyError::QueueFull,
                        error.to_string(),
                    ))
                    .await
                }
            }
        }
        Command::Abort { .. } => {
            // Cancelling twice is the same as cancelling once. The run settles once,
            // because the pump returns after the first `Settled`.
            cancel.cancel();
            out.reply(&Reply::ok_with(
                name,
                req_id,
                serde_json::json!({ "running": true }),
            ))
            .await
        }
        Command::DialogResponse { id, answer, .. } => {
            let delivered = dialogs.answer(&id, answer);
            out.reply(&Reply::ok_with(
                name,
                req_id,
                serde_json::json!({ "delivered": delivered }),
            ))
            .await
        }
        Command::GetCommands { .. } => {
            out.reply(&Reply::ok_with(
                name,
                req_id,
                serde_json::json!({ "commands": Command::NAMES }),
            ))
            .await
        }
        Command::GetMessages { .. } => {
            // The context is locked while a turn appends to it, so this waits for the
            // lock rather than reporting a half-written conversation.
            let messages = session.messages().await;
            match serde_json::to_value(&messages) {
                Ok(value) => {
                    out.reply(&Reply::ok_with(
                        name,
                        req_id,
                        serde_json::json!({ "messages": value }),
                    ))
                    .await
                }
                Err(error) => {
                    out.reply(&Reply::err(
                        name,
                        req_id,
                        ReplyError::Internal,
                        error.to_string(),
                    ))
                    .await
                }
            }
        }
        // Every command that would replace the session, or start a second run.
        Command::Prompt { .. } | Command::SetModel { .. } | Command::NewSession { .. } => {
            out.reply(&Reply::err(
                name,
                req_id,
                ReplyError::AlreadyStreaming,
                "a run is going. Wait for the settled event, or send abort.",
            ))
            .await
        }
        Command::GetState { .. } => {
            out.reply(&Reply::ok_with(
                name,
                req_id,
                serde_json::json!({ "running": true }),
            ))
            .await
        }
    }
}

/// The payload of a `get_state` reply.
fn state(current: &SessionRequest, running: bool) -> serde_json::Value {
    serde_json::json!({
        "provider": current.provider,
        "model_id": current.model_id,
        "running": running,
    })
}

/// Map a factory failure onto its named wire case.
///
/// The match has no wildcard arm, so a new `FactoryError` variant is a compile error
/// rather than a silent `internal`.
fn classify(error: &FactoryError) -> (ReplyError, &'static str) {
    match error {
        FactoryError::UnknownProvider { .. } => (ReplyError::UnknownProvider, "unknown provider"),
        FactoryError::MissingCredential { .. } => {
            (ReplyError::MissingCredential, "missing credential")
        }
        FactoryError::RefusedModel { .. } => (ReplyError::InvalidArgument, "refused model"),
        FactoryError::Internal(_) => (ReplyError::Internal, "internal"),
    }
}
