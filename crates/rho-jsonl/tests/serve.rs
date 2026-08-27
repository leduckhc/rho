//! Serve-loop tests: the whole protocol over a pair of in-memory pipes.
//!
//! Each test drives `serve` the way a real client does, by writing command lines and
//! reading the lines that come back. No network. No `sleep`. The session root is a
//! `tempfile::TempDir`, so no test reads the real `~/.rho`.

mod support;

use std::sync::Arc;

use rho_jsonl::{Event, Reply, ReplyError, SessionFactory, SessionRequest, serve};
use support::{ScriptedFactory, Turn};

/// One line of output, already routed into a reply or an event.
#[derive(Debug, Clone)]
enum Out {
    Reply(Reply),
    Event(Event),
}

/// Route one line the way the contract tells a client to: an event carries `type`,
/// and a reply carries `command` and `success`.
fn route(line: &str) -> Out {
    let value: serde_json::Value =
        serde_json::from_str(line).unwrap_or_else(|error| panic!("{line} is not JSON: {error}"));
    if value.get("type").is_some() {
        assert!(
            value.get("success").is_none(),
            "an event must not carry success: {line}"
        );
        Out::Event(serde_json::from_str(line).expect("an event must parse"))
    } else {
        assert!(
            value.get("success").is_some(),
            "a reply must carry success: {line}"
        );
        Out::Reply(serde_json::from_str(line).expect("a reply must parse"))
    }
}

/// Drive `serve` with a script of command lines, and return the routed output.
///
/// It feeds every command at once and then closes stdin, which is what a shell script
/// client does. `serve` must still settle any accepted run before it returns.
async fn drive(factory: ScriptedFactory, commands: &str) -> Vec<Out> {
    let factory: Arc<dyn SessionFactory> = Arc::new(factory);
    let input = std::io::Cursor::new(commands.as_bytes().to_vec());
    let output: Vec<u8> = Vec::new();
    let collected = Arc::new(std::sync::Mutex::new(output));
    let sink = Sink(Arc::clone(&collected));

    serve(
        factory,
        SessionRequest::new("scripted", "scripted-model"),
        input,
        sink,
    )
    .await
    .expect("serve must not fail on an in-memory pipe");

    let bytes = collected.lock().expect("lock").clone();
    String::from_utf8(bytes)
        .expect("valid utf8")
        .lines()
        .map(route)
        .collect()
}

/// An output stream that keeps every byte, so a test can read the transcript.
#[derive(Clone)]
struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);

impl tokio::io::AsyncWrite for Sink {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.0.lock().expect("lock").extend_from_slice(buf);
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

/// A real request and response client, over a duplex pipe.
///
/// `drive` above feeds every command at once, which is what a pipelining client does.
/// This one waits for what it asked for before it sends the next command, which is what
/// a shell-script client does. The difference matters: a second prompt sent before the
/// first run settles is genuinely a second run, and it must be refused.
struct Client {
    to_server: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    from_server:
        tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    served: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Client {
    fn start(factory: ScriptedFactory) -> Self {
        let (client_side, server_side) = tokio::io::duplex(64 * 1024);
        let (client_read, to_server) = tokio::io::split(client_side);
        let (server_read, server_write) = tokio::io::split(server_side);
        let factory: Arc<dyn SessionFactory> = Arc::new(factory);
        let served = tokio::spawn(async move {
            serve(
                factory,
                SessionRequest::new("scripted", "scripted-model"),
                server_read,
                server_write,
            )
            .await
        });
        Self {
            to_server,
            from_server: tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(client_read)),
            served,
        }
    }

    async fn send(&mut self, line: &str) {
        use tokio::io::AsyncWriteExt;
        self.to_server
            .write_all(format!("{line}\n").as_bytes())
            .await
            .expect("the pipe must accept a command");
        self.to_server.flush().await.expect("flush");
    }

    /// Read lines until one satisfies `done`, and return everything read.
    async fn read_until(&mut self, done: impl Fn(&Out) -> bool) -> Vec<Out> {
        let mut seen = Vec::new();
        while let Some(line) = self
            .from_server
            .next_line()
            .await
            .expect("the pipe must not fail")
        {
            let item = route(&line);
            let stop = done(&item);
            seen.push(item);
            if stop {
                return seen;
            }
        }
        panic!("the stream ended before the expected line: {seen:?}");
    }

    /// Read up to and including the next reply.
    async fn reply(&mut self) -> Vec<Out> {
        self.read_until(|item| matches!(item, Out::Reply(_))).await
    }

    /// Read up to and including the next settled event.
    async fn settled(&mut self) -> Vec<Out> {
        self.read_until(|item| matches!(item, Out::Event(Event::Settled { .. })))
            .await
    }

    /// Close stdin and wait for `serve` to return.
    ///
    /// It calls `shutdown`, and it does not merely drop the write half. Dropping a
    /// split `WriteHalf` leaves the `DuplexStream` open while the `ReadHalf` lives, so
    /// the server never sees end of file. A drop here hung every test in this file
    /// until a probe found it.
    async fn finish(mut self) {
        use tokio::io::AsyncWriteExt;
        self.to_server.shutdown().await.expect("shutdown");
        self.served
            .await
            .expect("the serve task must not panic")
            .expect("serve must return cleanly");
    }
}

fn replies(out: &[Out]) -> Vec<&Reply> {
    out.iter()
        .filter_map(|item| match item {
            Out::Reply(reply) => Some(reply),
            Out::Event(_) => None,
        })
        .collect()
}

fn events(out: &[Out]) -> Vec<&Event> {
    out.iter()
        .filter_map(|item| match item {
            Out::Event(event) => Some(event),
            Out::Reply(_) => None,
        })
        .collect()
}

fn settled(out: &[Out]) -> usize {
    events(out)
        .iter()
        .filter(|event| matches!(event, Event::Settled { .. }))
        .count()
}

fn error_case(reply: &Reply) -> Option<ReplyError> {
    match reply {
        Reply::Err(err) => Some(err.error),
        Reply::Ok(_) => None,
    }
}

fn data(reply: &Reply) -> serde_json::Value {
    match reply {
        Reply::Ok(ok) => ok.data.clone().unwrap_or(serde_json::Value::Null),
        Reply::Err(err) => panic!("expected a success reply, got {:?}", err.error),
    }
}

#[tokio::test]
async fn a_prompt_replies_then_streams_then_settles() {
    let out = drive(
        ScriptedFactory::new(vec![Turn::Text("hello there".to_string())]),
        "{\"type\":\"prompt\",\"req_id\":\"r1\",\"message\":\"hi\"}\n",
    )
    .await;

    // The reply comes before any event of that run, so a client can correlate.
    match &out[0] {
        Out::Reply(reply) => {
            assert!(matches!(reply, Reply::Ok(_)));
            match reply {
                Reply::Ok(ok) => {
                    assert_eq!(ok.req_id.as_deref(), Some("r1"));
                    assert_eq!(ok.command, "prompt");
                }
                Reply::Err(_) => unreachable!(),
            }
        }
        Out::Event(event) => panic!("the reply must come first, got {event:?}"),
    }

    let events = events(&out);
    assert!(events.iter().any(|event| matches!(
        event,
        Event::TextDelta { delta, .. } if delta == "hello there"
    )));
    assert_eq!(settled(&out), 1);
    assert!(matches!(events.last(), Some(Event::Settled { .. })));

    // Exactly one reply for one command line.
    assert_eq!(replies(&out).len(), 1);
}

#[tokio::test]
async fn unknown_command_is_reply_error() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"compact\",\"req_id\":\"r1\"}\n{\"type\":\"get_commands\"}\n",
    )
    .await;
    let replies = replies(&out);
    assert_eq!(error_case(replies[0]), Some(ReplyError::UnknownCommand));
    // The session stays open: the next command works.
    assert!(matches!(replies[1], Reply::Ok(_)));
}

#[tokio::test]
async fn parse_error_is_reply_error_and_the_session_stays_open() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{not json\n{\"type\":\"get_state\"}\n",
    )
    .await;
    let replies = replies(&out);
    assert_eq!(error_case(replies[0]), Some(ReplyError::ParseError));
    assert!(matches!(replies[1], Reply::Ok(_)));
}

#[tokio::test]
async fn an_unknown_field_on_a_command_is_refused() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"prompt\",\"message\":\"hi\",\"only_if_cheap\":true}\n",
    )
    .await;
    let replies = replies(&out);
    assert_eq!(error_case(replies[0]), Some(ReplyError::ParseError));
    // And no run started, so the whole instruction was refused.
    assert_eq!(settled(&out), 0);
}

#[tokio::test]
async fn a_crlf_command_line_is_accepted() {
    let out = drive(ScriptedFactory::new(vec![]), "{\"type\":\"get_state\"}\r\n").await;
    assert!(matches!(replies(&out)[0], Reply::Ok(_)));
}

#[tokio::test]
async fn steer_before_a_run_is_accepted() {
    // rho-core keeps an early steer and delivers it next run, so a reply that refused
    // it would make the client send the message twice. See
    // D-a-steer-is-never-rejected-for-being-early.
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"steer\",\"req_id\":\"s1\",\"message\":\"later\"}\n",
    )
    .await;
    let replies = replies(&out);
    assert!(
        matches!(replies[0], Reply::Ok(_)),
        "an early steer must be accepted: {:?}",
        error_case(replies[0])
    );
    assert_eq!(data(replies[0])["position"], 1);
}

#[tokio::test]
async fn steer_lands_at_a_turn_boundary() {
    // The steered message must be queued during the run and delivered between turns.
    // Two turns, so there is a boundary for it to land on.
    let out = drive(
        ScriptedFactory::new(vec![
            Turn::Text("first".to_string()),
            Turn::Text("second".to_string()),
        ]),
        // The steer is queued before the prompt, so the first boundary delivers it.
        "{\"type\":\"steer\",\"message\":\"also do this\"}\n{\"type\":\"prompt\",\"message\":\"go\"}\n",
    )
    .await;

    let events = events(&out);
    let delivered = events
        .iter()
        .position(|event| matches!(event, Event::MessageDelivered { count: 1 }));
    let delivered = delivered.unwrap_or_else(|| {
        panic!("a queued message must reach the model at a boundary: {events:?}")
    });

    // It lands at a boundary: a turn ended before it, and a turn starts after it.
    assert!(
        events[..delivered]
            .iter()
            .any(|event| matches!(event, Event::TurnEnd { .. }))
            || events[..delivered].is_empty(),
        "delivery must not land inside a turn: {events:?}"
    );
    assert!(
        events[delivered + 1..]
            .iter()
            .any(|event| matches!(event, Event::TurnStart)),
        "a turn must follow the delivery: {events:?}"
    );
    assert_eq!(settled(&out), 1);
}

#[tokio::test]
async fn a_full_queue_is_queue_full() {
    // The queue holds STEER_QUEUE_CAPACITY messages. One more must be refused by name,
    // and the earlier ones must stay queued.
    let mut script = String::new();
    for i in 0..=rho_core::STEER_QUEUE_CAPACITY {
        script.push_str(&format!(
            "{{\"type\":\"steer\",\"req_id\":\"s{i}\",\"message\":\"m{i}\"}}\n"
        ));
    }
    let out = drive(ScriptedFactory::new(vec![]), &script).await;
    let replies = replies(&out);
    assert_eq!(replies.len(), rho_core::STEER_QUEUE_CAPACITY + 1);
    for reply in &replies[..rho_core::STEER_QUEUE_CAPACITY] {
        assert!(matches!(reply, Reply::Ok(_)), "an early steer must be kept");
    }
    assert_eq!(
        error_case(replies[rho_core::STEER_QUEUE_CAPACITY]),
        Some(ReplyError::QueueFull),
        "a full queue needs its own named case, not internal"
    );
}

#[tokio::test]
async fn abort_with_no_run_is_accepted() {
    // A client that races a settling run must not get a failure for a run that ended.
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"abort\",\"req_id\":\"a1\"}\n",
    )
    .await;
    let replies = replies(&out);
    assert!(matches!(replies[0], Reply::Ok(_)));
    assert_eq!(data(replies[0])["running"], false);
    assert_eq!(settled(&out), 0, "an abort with no run emits no event");
}

#[tokio::test]
async fn a_prompt_after_a_settled_run_is_accepted() {
    // The same thing twice, from a client that waits. A frontend that frees the
    // session only when its event stream ends would refuse this second prompt with
    // AlreadyStreaming, because the stream needs one more poll after Settled.
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Text("one".to_string()),
        Turn::Text("two".to_string()),
    ]));

    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"a"}"#)
        .await;
    let first = client.settled().await;
    assert_eq!(settled(&first), 1);

    // The client has read Settled. The next prompt must be accepted.
    client
        .send(r#"{"type":"prompt","req_id":"p2","message":"b"}"#)
        .await;
    let second = client.reply().await;
    let reply = replies(&second)[0];
    assert!(
        matches!(reply, Reply::Ok(_)),
        "a prompt sent after Settled must be accepted, got {:?}",
        error_case(reply)
    );
    let rest = client.settled().await;
    assert_eq!(settled(&rest), 1, "the second run settles too");
    client.finish().await;
}

#[tokio::test]
async fn a_second_prompt_while_running_is_refused() {
    // The other half. A prompt that really does arrive during a run is refused by
    // name, and the running run is untouched.
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Text("one".to_string()),
        Turn::Text("two".to_string()),
    ]));
    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"a"}"#)
        .await;
    // Do not wait for Settled. Send the second prompt straight away.
    client
        .send(r#"{"type":"prompt","req_id":"p2","message":"b"}"#)
        .await;

    let seen = client.settled().await;
    let refused = replies(&seen)
        .into_iter()
        .find(|reply| error_case(reply) == Some(ReplyError::AlreadyStreaming));
    assert!(
        refused.is_some(),
        "a second prompt during a run must be refused: {seen:?}"
    );
    // The first run still settled exactly once.
    assert_eq!(settled(&seen), 1);
    client.finish().await;
}

#[tokio::test]
async fn abort_during_stream_settles_cancelled() {
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Text("a".to_string()),
        Turn::Text("b".to_string()),
        Turn::Text("c".to_string()),
    ]));
    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"go"}"#)
        .await;
    client.send(r#"{"type":"abort","req_id":"a1"}"#).await;
    let seen = client.settled().await;
    let last = events(&seen).last().copied().cloned();
    assert_eq!(
        last,
        Some(Event::Settled {
            stop_reason: rho_jsonl::SettleReason::Canceled
        }),
        "an aborted run must settle as cancelled: {seen:?}"
    );
    client.finish().await;
}

#[tokio::test]
async fn two_aborts_settle_once() {
    // The same thing twice. Cancelling twice must not settle twice, or a client that
    // counts runs loses track of them.
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Text("a".to_string()),
        Turn::Text("b".to_string()),
        Turn::Text("c".to_string()),
    ]));
    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"go"}"#)
        .await;
    client.send(r#"{"type":"abort","req_id":"a1"}"#).await;
    client.send(r#"{"type":"abort","req_id":"a2"}"#).await;
    let seen = client.settled().await;
    assert_eq!(
        settled(&seen),
        1,
        "two aborts settle one run once: {seen:?}"
    );
    client.finish().await;
}

#[tokio::test]
async fn set_model_while_running_is_refused() {
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Text("a".to_string()),
        Turn::Text("b".to_string()),
    ]));
    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"go"}"#)
        .await;
    client
        .send(r#"{"type":"set_model","req_id":"m1","provider":"other","model_id":"big"}"#)
        .await;
    let seen = client.settled().await;
    assert!(
        replies(&seen)
            .into_iter()
            .any(|reply| error_case(reply) == Some(ReplyError::AlreadyStreaming)),
        "set_model during a run must be refused, or a run loses the session under it: {seen:?}"
    );
    // The model did not change.
    client.send(r#"{"type":"get_state"}"#).await;
    let state = client.reply().await;
    assert_eq!(data(replies(&state)[0])["model_id"], "scripted-model");
    client.finish().await;
}

#[tokio::test]
async fn stdin_eof_mid_run_still_settles() {
    // The client goes away during a run. An accepted prompt still settles, because
    // `serve` drains the run before it returns. Otherwise a transcript would end with
    // no result and a reader could not tell a crash from a finished answer.
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Text("a".to_string()),
        Turn::Text("b".to_string()),
    ]));
    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"go"}"#)
        .await;
    // Close stdin at once, without waiting for anything. `shutdown` really closes it;
    // a plain drop would not, because the read half keeps the pipe open.
    {
        use tokio::io::AsyncWriteExt;
        client.to_server.shutdown().await.expect("shutdown");
    }
    let mut seen = Vec::new();
    while let Some(line) = client
        .from_server
        .next_line()
        .await
        .expect("the pipe must not fail")
    {
        seen.push(route(&line));
    }
    assert_eq!(
        settled(&seen),
        1,
        "a run must settle even when stdin closes: {seen:?}"
    );
    client
        .served
        .await
        .expect("no panic")
        .expect("serve returns cleanly");
}

#[tokio::test]
async fn new_session_clears_the_messages() {
    let mut client = Client::start(ScriptedFactory::new(vec![Turn::Text(
        "remembered".to_string(),
    )]));
    client.send(r#"{"type":"prompt","message":"hi"}"#).await;
    client.settled().await;

    client
        .send(r#"{"type":"get_messages","req_id":"g1"}"#)
        .await;
    let before = client.reply().await;
    let count = data(replies(&before)[0])["messages"]
        .as_array()
        .expect("an array")
        .len();
    assert!(count >= 2, "the conversation must hold the turn: {count}");

    client.send(r#"{"type":"new_session"}"#).await;
    client.reply().await;
    client
        .send(r#"{"type":"get_messages","req_id":"g2"}"#)
        .await;
    let after = client.reply().await;
    assert_eq!(
        data(replies(&after)[0])["messages"]
            .as_array()
            .expect("an array")
            .len(),
        0,
        "a new session starts empty"
    );
    client.finish().await;
}

#[tokio::test]
async fn a_faulting_provider_settles_and_the_session_survives() {
    // rho-core emits no AgentEnd on a provider failure, so the frontend must settle
    // it. And the same failure twice must behave the same way: "twice" has caught two
    // defects in this project.
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Fail("the host is down".to_string()),
        Turn::Fail("the host is still down".to_string()),
    ]));

    for attempt in ["p1", "p2"] {
        client
            .send(&format!(
                r#"{{"type":"prompt","req_id":"{attempt}","message":"a"}}"#
            ))
            .await;
        let seen = client.settled().await;
        assert_eq!(
            settled(&seen),
            1,
            "attempt {attempt} must settle exactly once: {seen:?}"
        );
        assert_eq!(
            events(&seen).last().copied().cloned(),
            Some(Event::Settled {
                stop_reason: rho_jsonl::SettleReason::Faulted
            }),
            "a provider failure settles as faulted"
        );
        assert!(
            events(&seen)
                .iter()
                .any(|event| matches!(event, Event::Fault { .. })),
            "the failure must be reported: {seen:?}"
        );
        // Exactly one reply for the prompt. A fault after acceptance is an event, and
        // never a second reply for the same req_id.
        assert_eq!(replies(&seen).len(), 1);
    }

    // The session is still usable after two failures.
    client.send(r#"{"type":"get_state"}"#).await;
    let state = client.reply().await;
    assert!(matches!(replies(&state)[0], Reply::Ok(_)));
    client.finish().await;
}

#[tokio::test]
async fn a_stream_that_ends_with_no_done_still_settles() {
    let out = drive(
        ScriptedFactory::new(vec![Turn::NoDone]),
        "{\"type\":\"prompt\",\"message\":\"a\"}\n",
    )
    .await;
    assert_eq!(settled(&out), 1);
}

#[tokio::test]
async fn set_model_round_trip() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"set_model\",\"req_id\":\"m1\",\"provider\":\"other\",\"model_id\":\"big\"}\n{\"type\":\"get_state\"}\n",
    )
    .await;
    let replies = replies(&out);
    assert_eq!(data(replies[0])["provider"], "other");
    assert_eq!(data(replies[0])["model_id"], "big");
    // The state reports the new model, so the switch really took effect.
    assert_eq!(data(replies[1])["provider"], "other");
    assert_eq!(data(replies[1])["model_id"], "big");
    assert_eq!(data(replies[1])["running"], false);
}

#[tokio::test]
async fn every_factory_failure_has_its_own_named_case() {
    // Each FactoryError maps to one wire case, so a client learns which of them
    // happened without reading prose. And the old session must survive each one.
    let cases = [
        ("nosuch", ReplyError::UnknownProvider),
        ("nocreds", ReplyError::MissingCredential),
        ("badmodel", ReplyError::InvalidArgument),
        ("broken", ReplyError::Internal),
    ];
    for (provider, expected) in cases {
        let script = format!(
            "{{\"type\":\"set_model\",\"provider\":\"{provider}\",\"model_id\":\"m\"}}\n{{\"type\":\"get_state\"}}\n"
        );
        let out = drive(ScriptedFactory::new(vec![]), &script).await;
        let replies = replies(&out);
        assert_eq!(
            error_case(replies[0]),
            Some(expected),
            "provider {provider} produced the wrong case"
        );
        // The old session is untouched, so the client can carry on.
        assert_eq!(data(replies[1])["provider"], "scripted");
    }
}

#[tokio::test]
async fn a_missing_credential_reply_names_the_variable_and_no_secret() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"set_model\",\"provider\":\"nocreds\",\"model_id\":\"m\"}\n",
    )
    .await;
    match replies(&out)[0] {
        Reply::Err(err) => {
            assert_eq!(err.error, ReplyError::MissingCredential);
            assert!(
                err.message.contains("SCRIPTED_KEY"),
                "the reply must name the variable to set: {}",
                err.message
            );
        }
        Reply::Ok(_) => panic!("a missing credential must fail"),
    }
}

#[tokio::test]
async fn get_commands_lists_every_command() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"get_commands\",\"req_id\":\"c1\"}\n",
    )
    .await;
    let listed = data(replies(&out)[0])["commands"]
        .as_array()
        .expect("an array")
        .len();
    assert_eq!(listed, rho_jsonl::Command::NAMES.len());
}

#[tokio::test]
async fn an_over_long_line_is_refused_once_and_the_next_command_works() {
    // One enormous line must produce one reply, not one per buffer, and the reader
    // must resume afterwards.
    let long = format!(
        "{{\"type\":\"prompt\",\"message\":\"{}\"}}\n{{\"type\":\"get_state\"}}\n",
        "x".repeat(rho_jsonl::MAX_COMMAND_LINE_BYTES + 1024)
    );
    let out = drive(ScriptedFactory::new(vec![]), &long).await;
    let replies = replies(&out);
    assert_eq!(
        error_case(replies[0]),
        Some(ReplyError::LineTooLong),
        "an over-long line needs its own case"
    );
    assert_eq!(
        replies.len(),
        2,
        "one refused line is one reply, not one per read buffer"
    );
    assert!(matches!(replies[1], Reply::Ok(_)));
    assert_eq!(settled(&out), 0, "a refused line starts no run");
}

#[tokio::test]
async fn a_dialog_answer_with_a_wrong_shape_is_invalid_argument() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"dialog_response\",\"id\":\"d1\",\"answer\":{\"confirmed\":true,\"cancelled\":true}}\n{\"type\":\"get_state\"}\n",
    )
    .await;
    let replies = replies(&out);
    // It arrives as a parse failure of the command, which is the ParseError case.
    assert!(
        matches!(
            error_case(replies[0]),
            Some(ReplyError::ParseError) | Some(ReplyError::InvalidArgument)
        ),
        "a two-key answer must be refused, got {:?}",
        error_case(replies[0])
    );
    assert!(matches!(replies[1], Reply::Ok(_)), "the session stays open");
}

#[tokio::test]
async fn a_dialog_answer_for_no_open_dialog_is_dropped() {
    let out = drive(
        ScriptedFactory::new(vec![]),
        "{\"type\":\"dialog_response\",\"req_id\":\"d1\",\"id\":\"nope\",\"answer\":{\"cancelled\":true}}\n",
    )
    .await;
    let replies = replies(&out);
    assert_eq!(data(replies[0])["delivered"], false);
}

#[tokio::test]
async fn the_first_session_failure_is_reported_and_serve_stops() {
    // A frontend that exited in silence here would look like a crash to the client.
    let factory: Arc<dyn SessionFactory> = Arc::new(ScriptedFactory::new(vec![]));
    let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
    serve(
        factory,
        SessionRequest::new("nosuch", "m"),
        std::io::Cursor::new(Vec::new()),
        Sink(Arc::clone(&collected)),
    )
    .await
    .expect("serve returns cleanly");
    let text = String::from_utf8(collected.lock().expect("lock").clone()).expect("utf8");
    let out: Vec<Out> = text.lines().map(route).collect();
    assert_eq!(
        error_case(replies(&out)[0]),
        Some(ReplyError::UnknownProvider)
    );
}

#[tokio::test]
async fn no_line_is_ever_half_written() {
    // Replies and events share one stream, so a partial line would corrupt the record
    // after it. Every line must be whole JSON on its own.
    let out = drive(
        ScriptedFactory::new(vec![
            Turn::Text("a longer answer, to make several writes race".to_string()),
            Turn::Text("and another".to_string()),
        ]),
        "{\"type\":\"prompt\",\"message\":\"a\"}\n{\"type\":\"prompt\",\"message\":\"b\"}\n{\"type\":\"get_state\"}\n",
    )
    .await;
    // `route` already parsed every line, so reaching here proves each one is whole.
    assert!(out.len() > 4, "expected a real transcript: {out:?}");
}

#[tokio::test]
async fn a_dialog_reaches_the_client_and_its_answer_runs_the_tool() {
    // The whole dialog sub-protocol over the wire, which is the only proof that
    // matters: the agent asks, the client answers, and the tool then runs. A unit test
    // of the policy cannot see the wire.
    let factory =
        ScriptedFactory::new(vec![Turn::CallTool, Turn::Text("done".to_string())]).asking();
    let ran = Arc::clone(&factory.tool_ran);
    let mut client = Client::start(factory);

    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"touch it"}"#)
        .await;

    // Read up to the dialog request the approval gate raised.
    let seen = client
        .read_until(|item| matches!(item, Out::Event(Event::Dialog(_))))
        .await;
    let request = events(&seen)
        .into_iter()
        .find_map(|event| match event {
            Event::Dialog(request) => Some(request.clone()),
            _ => None,
        })
        .expect("the approval gate must ask the client");
    let id = request.id().to_string();
    assert!(request.blocks(), "an approval dialog must block the run");

    // Answer yes with the matching id, and the tool must run.
    client
        .send(&format!(
            r#"{{"type":"dialog_response","id":"{id}","answer":{{"confirmed":true}}}}"#
        ))
        .await;
    let rest = client.settled().await;
    assert_eq!(
        ran.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "an explicit yes must let the tool run: {rest:?}"
    );
    assert!(
        events(&rest)
            .iter()
            .any(|event| matches!(event, Event::ToolEnd { ok: true, .. })),
        "the tool result must reach the client: {rest:?}"
    );
    client.finish().await;
}

#[tokio::test]
async fn a_denied_dialog_stops_the_tool() {
    // The fail-closed half, over the wire. An explicit no must leave the tool unrun.
    let factory =
        ScriptedFactory::new(vec![Turn::CallTool, Turn::Text("done".to_string())]).asking();
    let ran = Arc::clone(&factory.tool_ran);
    let mut client = Client::start(factory);

    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"touch it"}"#)
        .await;
    let seen = client
        .read_until(|item| matches!(item, Out::Event(Event::Dialog(_))))
        .await;
    let id = events(&seen)
        .into_iter()
        .find_map(|event| match event {
            Event::Dialog(request) => Some(request.id().to_string()),
            _ => None,
        })
        .expect("a dialog");

    client
        .send(&format!(
            r#"{{"type":"dialog_response","id":"{id}","answer":{{"confirmed":false}}}}"#
        ))
        .await;
    let rest = client.settled().await;
    assert_eq!(
        ran.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a denial must stop the tool: {rest:?}"
    );
    client.finish().await;
}

#[tokio::test]
async fn a_client_that_answers_no_dialog_denies_the_tool_and_the_run_continues() {
    // The client answers nothing at all. The agent-side timeout resolves the dialog as
    // cancelled, the tool is denied, and the run still settles. A client that
    // implements no dialog support must not hang rho.
    let factory =
        ScriptedFactory::new(vec![Turn::CallTool, Turn::Text("done".to_string())]).asking();
    let ran = Arc::clone(&factory.tool_ran);
    let mut client = Client::start(factory);

    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"touch it"}"#)
        .await;
    // Send no answer. The factory sets a 50 ms approval timeout, so this resolves on
    // its own. This is the one place a real clock is unavoidable: the timeout lives
    // inside a spawned agent task, so `tokio::time::advance` cannot reach it.
    let seen = client.settled().await;
    assert_eq!(
        ran.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a silent client must never approve a tool: {seen:?}"
    );
    assert_eq!(settled(&seen), 1, "the run must still settle: {seen:?}");
    // Exactly one dialog request, and no second one for the same id.
    assert_eq!(
        events(&seen)
            .iter()
            .filter(|event| matches!(event, Event::Dialog(_)))
            .count(),
        1,
        "a timeout emits no second request: {seen:?}"
    );
    client.finish().await;
}

#[tokio::test]
async fn two_over_long_lines_get_two_replies() {
    // The protocol promises one reply per command line. Collapsing a run of refusals is
    // right for one unterminated line, but two whole over-long lines are two lines, and
    // each needs its own reply. Before this, the second was silently unanswered.
    let long = "x".repeat(rho_jsonl::MAX_COMMAND_LINE_BYTES + 512);
    let script = format!("{long}\n{long}\n{{\"type\":\"get_state\"}}\n");
    let out = drive(ScriptedFactory::new(vec![]), &script).await;
    let replies = replies(&out);
    let refused = replies
        .iter()
        .filter(|reply| error_case(reply) == Some(ReplyError::LineTooLong))
        .count();
    assert_eq!(
        refused, 2,
        "each over-long line needs its own reply: {out:?}"
    );
    assert!(
        matches!(replies.last(), Some(Reply::Ok(_))),
        "the good command after them still works"
    );
}

#[tokio::test]
async fn get_state_during_a_run_reports_the_model() {
    // A client that polls state while the agent works must not lose the model it is
    // talking to. The in-run path used to report only the running flag.
    let mut client = Client::start(ScriptedFactory::new(vec![
        Turn::Text("a".to_string()),
        Turn::Text("b".to_string()),
    ]));
    client
        .send(r#"{"type":"prompt","req_id":"p1","message":"go"}"#)
        .await;
    client.send(r#"{"type":"get_state","req_id":"s1"}"#).await;
    let seen = client
        .read_until(|item| match item {
            Out::Reply(reply) => matches!(reply, Reply::Ok(ok) if ok.command == "get_state"),
            Out::Event(_) => false,
        })
        .await;
    let state = replies(&seen)
        .into_iter()
        .find(|reply| matches!(reply, Reply::Ok(ok) if ok.command == "get_state"))
        .expect("a get_state reply");
    let payload = data(state);
    assert_eq!(payload["running"], true, "a run is going");
    assert_eq!(payload["provider"], "scripted");
    assert_eq!(payload["model_id"], "scripted-model");
    client.settled().await;
    client.finish().await;
}

#[tokio::test]
async fn new_session_builds_a_fresh_session() {
    // `new_session` must ask the factory for a new session, not clear the old one in
    // place. The count is the only way to see the difference from outside.
    let factory = ScriptedFactory::new(vec![Turn::Text("a".to_string())]);
    let builds = Arc::clone(&factory.builds);
    let mut client = Client::start(factory);
    // One build for the first session.
    client.send(r#"{"type":"get_state"}"#).await;
    client.reply().await;
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);

    client.send(r#"{"type":"new_session","req_id":"n1"}"#).await;
    let reply = client.reply().await;
    assert!(matches!(replies(&reply)[0], Reply::Ok(_)));
    assert_eq!(
        builds.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "new_session must build a second session"
    );

    // And a model switch builds another.
    client
        .send(r#"{"type":"set_model","provider":"other","model_id":"big"}"#)
        .await;
    client.reply().await;
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 3);
    client.finish().await;
}

#[tokio::test]
async fn a_duplicate_dialog_id_does_not_strand_a_dialog() {
    // `Asker::ask` is public, and a host builds its own requests, so two open dialogs can
    // carry one id. Overwriting the first sender stranded the pair: the first ask resolved
    // as cancelled, its guard removed the entry by id, and that deleted the second
    // dialog's sender. A request with no timeout could then never resolve.
    use rho_jsonl::{Asker, DialogAnswer, DialogHost, DialogRequest, Writer};

    let sink = Sink(Arc::new(std::sync::Mutex::new(Vec::new())));
    let host = DialogHost::new(Writer::new(sink));
    let request = || DialogRequest::Input {
        id: "same".to_string(),
        title: "name".to_string(),
        placeholder: None,
        timeout_ms: None,
    };

    let first = tokio::spawn({
        let host = host.clone();
        async move { host.ask(request()).await }
    });
    tokio::task::yield_now().await;
    assert_eq!(host.open_count(), 1);

    // The second ask reuses the id. It must be refused, not swap the sender out.
    let second = host.ask(request()).await;
    assert_eq!(
        second,
        DialogAnswer::Cancelled,
        "a duplicate id must be refused, and refusing denies"
    );
    assert_eq!(
        host.open_count(),
        1,
        "the first dialog is still the only one"
    );

    // The first dialog is still answerable, which is the whole point.
    assert!(host.answer("same", DialogAnswer::Value("kept".to_string())));
    assert_eq!(
        first.await.expect("no panic"),
        DialogAnswer::Value("kept".to_string())
    );
}
