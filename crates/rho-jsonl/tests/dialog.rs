//! Dialog sub-protocol tests. See SPEC-jsonl-frontend section 3.4.
//!
//! No test uses `sleep`. The timeout tests use `tokio::time` pause and advance, so
//! they take no wall-clock time and cannot be flaky.

use std::sync::Arc;

use async_trait::async_trait;

use rho_core::{ApprovalDecision, ApprovalPolicy, ToolKind};
use rho_jsonl::{
    APPROVAL_TIMEOUT_MS, Asker, DialogAnswer, DialogApproval, DialogHost, DialogRequest, Event,
    Writer,
};

/// A writer over a shared buffer, so a test reads what the host wrote.
#[derive(Clone, Default)]
struct Shared(Arc<std::sync::Mutex<Vec<u8>>>);

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
            .map(|line| serde_json::from_str(line).expect("an event per line"))
            .collect()
    }
}

fn host() -> (DialogHost<Shared>, Shared) {
    let shared = Shared::default();
    (DialogHost::new(Writer::new(shared.clone())), shared)
}

#[tokio::test(start_paused = true)]
async fn dialog_timeout_auto_resolves_cancelled() {
    // The agent side owns the timeout. When it expires the dialog resolves as
    // Cancelled, which reads as a denial. See D-a-dialog-timeout-cancels.
    let (host, shared) = host();
    let request = DialogRequest::Select {
        id: "d1".to_string(),
        title: "pick one".to_string(),
        options: vec!["safe".to_string(), "risky".to_string()],
        timeout_ms: 1_000,
    };
    let asking = tokio::spawn({
        let host = host.clone();
        async move { host.ask(request).await }
    });

    // Let the ask register and write its event, then push time past the timeout.
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(1_001)).await;

    let answer = asking.await.expect("the ask task must not panic");
    assert_eq!(
        answer,
        DialogAnswer::Cancelled,
        "a timeout must not choose an option for the user"
    );

    // Exactly one dialog event, and no second one for the same id.
    let events = shared.events();
    let dialogs: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, Event::Dialog(_)))
        .collect();
    assert_eq!(dialogs.len(), 1, "a timeout emits no second request");
    // The slot is released, so a late answer finds nothing and the map cannot grow.
    assert_eq!(host.open_count(), 0, "a resolved dialog must free its slot");
}

#[tokio::test]
async fn dialog_response_id_matches_request() {
    let (host, _shared) = host();
    let request = DialogRequest::Confirm {
        id: "d7".to_string(),
        title: "sure?".to_string(),
        message: "it writes a file".to_string(),
        timeout_ms: 60_000,
    };
    let asking = tokio::spawn({
        let host = host.clone();
        async move { host.ask(request).await }
    });
    tokio::task::yield_now().await;

    // A wrong id is dropped, and the dialog stays open.
    assert!(
        !host.answer("nope", DialogAnswer::Confirmed(true)),
        "a wrong id must not resolve a dialog"
    );
    assert_eq!(host.open_count(), 1, "the dialog must still be open");

    // The right id resolves it.
    assert!(host.answer("d7", DialogAnswer::Confirmed(true)));
    assert_eq!(
        asking.await.expect("no panic"),
        DialogAnswer::Confirmed(true)
    );
    assert_eq!(host.open_count(), 0);
}

#[tokio::test]
async fn a_late_dialog_answer_is_dropped() {
    let (host, shared) = host();
    let request = DialogRequest::Input {
        id: "d8".to_string(),
        title: "name".to_string(),
        placeholder: None,
        timeout_ms: 60_000,
    };
    let asking = tokio::spawn({
        let host = host.clone();
        async move { host.ask(request).await }
    });
    tokio::task::yield_now().await;
    assert!(host.answer("d8", DialogAnswer::Value("first".to_string())));
    assert_eq!(
        asking.await.expect("no panic"),
        DialogAnswer::Value("first".to_string())
    );

    // The second answer for a resolved id changes nothing and emits nothing.
    let before = shared.events().len();
    assert!(
        !host.answer("d8", DialogAnswer::Value("second".to_string())),
        "an answer for a resolved dialog must be dropped"
    );
    assert_eq!(
        shared.events().len(),
        before,
        "a late answer emits no event"
    );
}

#[tokio::test]
async fn dialog_notify_expects_no_reply() {
    // A Notify blocks nothing. If it blocked, a run would hang for ever waiting for
    // an answer the contract tells the client not to send.
    let (host, shared) = host();
    host.notify("the build finished".to_string()).await;
    let events = shared.events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::Dialog(DialogRequest::Notify { message, .. }) => {
            assert_eq!(message, "the build finished");
        }
        other => panic!("expected a notify, got {other:?}"),
    }
    assert_eq!(host.open_count(), 0, "a notify must open no dialog slot");
}

#[tokio::test]
async fn asking_a_notify_does_not_block() {
    // Passing a Notify to `ask` must return rather than wait, or one wrong call hangs
    // the whole run. This is the fail-safe half of the notify rule.
    let (host, _shared) = host();
    let answer = host
        .ask(DialogRequest::Notify {
            id: "d0".to_string(),
            message: "hello".to_string(),
        })
        .await;
    assert_eq!(answer, DialogAnswer::Cancelled);
    assert_eq!(host.open_count(), 0);
}

#[tokio::test]
async fn dialog_ids_are_unique() {
    let (host, _shared) = host();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..100 {
        assert!(
            seen.insert(host.next_dialog_id()),
            "a repeated dialog id would let one answer resolve two dialogs"
        );
    }
}

/// An asker that answers with a fixed value, and counts the questions.
struct Canned {
    answer: DialogAnswer,
    asked: Arc<std::sync::atomic::AtomicUsize>,
    last: Arc<std::sync::Mutex<Option<DialogRequest>>>,
}

#[async_trait]
impl Asker for Canned {
    async fn ask(&self, request: DialogRequest) -> DialogAnswer {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.last.lock().expect("lock") = Some(request);
        self.answer.clone()
    }
    async fn notify(&self, _message: String) {}
    fn next_dialog_id(&self) -> String {
        "d-canned".to_string()
    }
}

fn canned(answer: DialogAnswer) -> (Arc<Canned>, Arc<std::sync::Mutex<Option<DialogRequest>>>) {
    let last = Arc::new(std::sync::Mutex::new(None));
    let asker = Arc::new(Canned {
        answer,
        asked: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        last: Arc::clone(&last),
    });
    (asker, last)
}

#[tokio::test]
async fn dialog_approval_allows_only_an_explicit_yes() {
    // The table in SPEC-jsonl-frontend section 3.4. A denied safe tool is an
    // annoyance, and an approved destructive tool is a breach, so everything that is
    // not an explicit yes denies.
    let cases = vec![
        (DialogAnswer::Confirmed(true), ApprovalDecision::Allow),
        (DialogAnswer::Confirmed(false), ApprovalDecision::Deny),
        (DialogAnswer::Cancelled, ApprovalDecision::Deny),
        // A client that answers a confirm with a text value gets a denial, not a
        // guess about what the text meant.
        (
            DialogAnswer::Value("yes".to_string()),
            ApprovalDecision::Deny,
        ),
        (DialogAnswer::Value(String::new()), ApprovalDecision::Deny),
    ];
    for (answer, expected) in cases {
        let (asker, _) = canned(answer.clone());
        let policy = DialogApproval::new(asker);
        let decision = policy
            .approve("bash", ToolKind::Execute, &serde_json::json!({}))
            .await;
        assert_eq!(decision, expected, "answer {answer:?} decided wrongly");
    }
}

#[tokio::test(start_paused = true)]
async fn dialog_approval_denies_on_a_timeout() {
    // A client that answers nothing must not get its tool call approved. The whole
    // point of the fail-closed rule.
    let (host, _shared) = host();
    let asker: Arc<dyn Asker> = Arc::new(host);
    let policy = DialogApproval::with_timeout_ms(Arc::clone(&asker), 500);
    let deciding = tokio::spawn(async move {
        policy
            .approve("bash", ToolKind::Execute, &serde_json::json!({}))
            .await
    });
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(501)).await;
    assert_eq!(
        deciding.await.expect("no panic"),
        ApprovalDecision::Deny,
        "a silent client must never approve a command"
    );
}

#[tokio::test]
async fn the_approval_dialog_carries_no_tool_arguments() {
    // A tool argument can hold a secret, and this crate has no redactor. So the
    // confirm names the tool and its kind, and never the arguments. See
    // D-redact-tool-arguments.
    let (asker, last) = canned(DialogAnswer::Confirmed(true));
    let policy = DialogApproval::new(asker);
    let secret = "sk-live-000111222";
    policy
        .approve(
            "bash",
            ToolKind::Execute,
            &serde_json::json!({ "command": format!("curl -H 'Authorization: {secret}'") }),
        )
        .await;
    let request = last.lock().expect("lock").clone().expect("one request");
    let text = serde_json::to_string(&request).expect("serialise");
    assert!(
        !text.contains(secret),
        "a tool argument must never reach the dialog: {text}"
    );
    assert!(text.contains("bash"), "the tool name must be shown: {text}");
    assert!(
        text.contains("command"),
        "the kind must be shown in plain words: {text}"
    );
}

#[tokio::test]
async fn the_approval_dialog_always_carries_a_timeout() {
    // A confirm with no timeout hangs the run for ever when the client is silent.
    // The approval gate is the one caller that must not be able to hang.
    let (asker, last) = canned(DialogAnswer::Confirmed(true));
    let policy = DialogApproval::new(asker);
    policy
        .approve("read", ToolKind::Read, &serde_json::json!({}))
        .await;
    let request = last.lock().expect("lock").clone().expect("one request");
    assert_eq!(request.timeout_ms(), Some(APPROVAL_TIMEOUT_MS));
}

#[tokio::test]
async fn every_tool_kind_gets_a_plain_word() {
    // A security prompt must never show a tool kind as an empty string or a debug
    // name. Every variant gets a word a human can read.
    let kinds = [
        ToolKind::Read,
        ToolKind::Edit,
        ToolKind::Delete,
        ToolKind::Move,
        ToolKind::Search,
        ToolKind::Execute,
        ToolKind::Think,
        ToolKind::Fetch,
        ToolKind::SwitchMode,
        ToolKind::Other,
    ];
    for kind in kinds {
        let (asker, last) = canned(DialogAnswer::Confirmed(false));
        let policy = DialogApproval::new(asker);
        policy.approve("t", kind, &serde_json::json!({})).await;
        let request = last.lock().expect("lock").clone().expect("one request");
        match request {
            DialogRequest::Confirm { message, .. } => {
                // Two independent checks. Joining them with `or` let either one pass
                // for the other, so the empty-word case could slip through.
                assert!(
                    !message.contains("a  "),
                    "{kind:?} produced an empty word: {message}"
                );
                assert!(
                    message.contains("operation."),
                    "{kind:?} lost the sentence shape: {message}"
                );
                assert!(message.len() > 30, "{kind:?} produced {message}");
            }
            other => panic!("the approval gate must use a confirm, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_dropped_ask_frees_its_dialog_slot() {
    // A run can be aborted while a dialog is open, and the whole `ask` future is then
    // dropped between the insert and the answer. The explicit cleanup on the timeout path
    // did not cover that, so the slot stayed in the map for the life of the process and a
    // client that aborted often leaked one entry per abort. A drop guard frees it.
    let (host, _shared) = host();
    {
        let asking = host.ask(DialogRequest::Confirm {
            id: "d-dropped".to_string(),
            title: "sure?".to_string(),
            message: "it deletes a file".to_string(),
            timeout_ms: 60_000,
        });
        let mut asking = Box::pin(asking);
        let waker = futures::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        // One poll registers the dialog and writes the event, then waits.
        assert!(
            std::future::Future::poll(asking.as_mut(), &mut cx).is_pending(),
            "the ask must wait for an answer"
        );
        assert_eq!(host.open_count(), 1, "the dialog must be registered");
        // Drop it, exactly as an aborted run does.
    }
    assert_eq!(
        host.open_count(),
        0,
        "a dropped ask must free its slot, or the map grows for ever"
    );
}
