//! The dialog sub-protocol, and the approval policy built on it.
//!
//! The agent side owns every timeout, so the two sides can never disagree about
//! whether a dialog is open. A timeout resolves as `Cancelled`, which reads as a
//! denial everywhere. See decisions D-a-dialog-timeout-cancels and
//! D-ask-policy-fails-closed.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::io::AsyncWrite;
use tokio::sync::oneshot;

use rho_core::{ApprovalDecision, ApprovalPolicy, ToolKind};

use crate::protocol::{DialogAnswer, DialogRequest, Event};
use crate::writer::Writer;

/// Ask the client a question, and wait for its answer.
///
/// It is a trait, and it is object safe, so a host holds `Arc<dyn Asker>` with no
/// knowledge of the output stream type. A host uses it to build an approval policy or
/// a hook that needs a human. `rho-jsonl` implements it over the protocol.
#[async_trait]
pub trait Asker: Send + Sync {
    /// Ask a blocking question. It returns the client's answer, or `Cancelled` when
    /// the request carries a timeout and the timeout expires.
    ///
    /// A `Notify` request blocks nothing, so passing one returns `Cancelled` at once.
    /// Use [`Asker::notify`] for that instead.
    async fn ask(&self, request: DialogRequest) -> DialogAnswer;

    /// Tell the client something. It expects no answer and blocks nothing.
    async fn notify(&self, message: String);

    /// Mint a fresh dialog id. Ids are unique for the life of the process.
    fn next_dialog_id(&self) -> String;
}

/// The open dialogs, keyed by id.
///
/// It is a plain `std::sync::Mutex`, because every critical section is a map
/// insert or remove and none of them awaits.
#[derive(Default)]
struct Pending {
    open: Mutex<HashMap<String, oneshot::Sender<DialogAnswer>>>,
    next: AtomicU64,
}

/// Frees one open dialog slot on every exit path, including a dropped future.
///
/// The explicit cleanup on the timeout path was not enough. A run can be aborted while a
/// dialog is open, and then the whole `ask` future is dropped between the insert and the
/// answer. Without this guard that slot stayed in the map for the life of the process, so
/// a client that aborted often leaked one entry per abort.
struct Slot {
    pending: Arc<Pending>,
    id: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        // A poisoned lock must not panic inside a drop, so this ignores that case.
        if let Ok(mut open) = self.pending.open.lock() {
            open.remove(&self.id);
        }
    }
}

/// The client-facing side of the dialog sub-protocol.
pub struct DialogHost<W> {
    pending: Arc<Pending>,
    out: Writer<W>,
}

impl<W> Clone for DialogHost<W> {
    fn clone(&self) -> Self {
        Self {
            pending: Arc::clone(&self.pending),
            out: self.out.clone(),
        }
    }
}

impl<W: AsyncWrite + Unpin> DialogHost<W> {
    /// Build a host that writes its requests to this writer.
    pub fn new(out: Writer<W>) -> Self {
        Self {
            pending: Arc::new(Pending::default()),
            out,
        }
    }

    /// Deliver an answer from the client.
    ///
    /// It returns `false` when the id is unknown, which covers both a wrong id and an
    /// answer that arrives after the dialog resolved. A late answer changes nothing
    /// and emits no event, because a second reply for a resolved dialog would race
    /// with the run. See D-a-dialog-timeout-cancels.
    pub fn answer(&self, id: &str, answer: DialogAnswer) -> bool {
        // A poisoned map means another thread panicked while holding it. Treat that as
        // "no such dialog" rather than panic here. It matches `Slot::drop`, and it stays
        // fail-closed, because an unanswered dialog denies.
        let sender = match self.pending.open.lock() {
            Ok(mut open) => open.remove(id),
            Err(_) => None,
        };
        match sender {
            // The receiver is gone only when the asking side stopped waiting, so a
            // failed send is the same as an unknown id.
            Some(sender) => sender.send(answer).is_ok(),
            None => false,
        }
    }

    /// How many dialogs are open. A test asserts it, so a resolved dialog cannot
    /// leak its slot.
    pub fn open_count(&self) -> usize {
        // Same rule as `answer`: a poisoned map reports no open dialogs.
        self.pending.open.lock().map(|open| open.len()).unwrap_or(0)
    }
}

#[async_trait]
impl<W: AsyncWrite + Unpin + Send + Sync + 'static> Asker for DialogHost<W> {
    async fn ask(&self, request: DialogRequest) -> DialogAnswer {
        if !request.blocks() {
            // A `Notify` has no answer. Emitting it and then waiting would hang the
            // run for ever, so refuse instead of waiting.
            let _ = self.out.event(&Event::Dialog(request)).await;
            return DialogAnswer::Cancelled;
        }

        let id = request.id().to_string();
        let timeout = request.timeout_ms();
        let (tx, rx) = oneshot::channel();
        {
            let mut open = self
                .pending
                .open
                .lock()
                .expect("the dialog map is poisoned");
            // Refuse a duplicate id rather than overwrite the first sender.
            //
            // The id comes from the request, and a host builds its own requests, so two
            // open dialogs can carry one id. An overwrite stranded the pair: the first
            // `ask` resolved as cancelled, its guard then removed the entry by id, and
            // that deleted the second dialog's sender. A `Select` with no timeout could
            // then never resolve at all. `DialogApproval` mints its ids with
            // `next_dialog_id`, so only a hand-built request reaches this.
            if open.contains_key(&id) {
                return DialogAnswer::Cancelled;
            }
            open.insert(id.clone(), tx);
        }

        // From here on every return, and every drop of this future, frees the slot.
        let _slot = Slot {
            pending: Arc::clone(&self.pending),
            id,
        };

        if self.out.event(&Event::Dialog(request)).await.is_err() {
            // The client is gone. Fail closed rather than wait for an answer that
            // cannot arrive.
            return DialogAnswer::Cancelled;
        }

        match timeout {
            Some(ms) => {
                match tokio::time::timeout(std::time::Duration::from_millis(ms), rx).await {
                    Ok(Ok(answer)) => answer,
                    // The timeout expired, or the sender was dropped. Both resolve as a
                    // denial, and the guard frees the slot so a late answer finds no
                    // dialog and the map cannot grow.
                    _ => DialogAnswer::Cancelled,
                }
            }
            None => match rx.await {
                Ok(answer) => answer,
                Err(_) => DialogAnswer::Cancelled,
            },
        }
    }

    async fn notify(&self, message: String) {
        let id = self.next_dialog_id();
        let _ = self
            .out
            .event(&Event::Dialog(DialogRequest::Notify { id, message }))
            .await;
    }

    fn next_dialog_id(&self) -> String {
        let n = self.pending.next.fetch_add(1, Ordering::Relaxed);
        format!("d{n}")
    }
}

/// The default time a tool approval waits for a human. Thirty seconds.
///
/// It is never `None`. A dialog with no timeout would hang a run for ever when the
/// client answers nothing, and the approval gate is the one caller that must not be
/// able to hang.
pub const APPROVAL_TIMEOUT_MS: u64 = 30_000;

/// An approval policy that asks the client.
///
/// It gives the dialog sub-protocol a real producer, so the sub-protocol is not dead
/// surface. Before this, no rho frontend could answer `approval = "ask"`.
///
/// It is fail-closed. Only an explicit yes allows the call. See the table in
/// SPEC-jsonl-frontend section 3.4.
pub struct DialogApproval {
    asker: Arc<dyn Asker>,
    timeout_ms: u64,
}

impl DialogApproval {
    /// Ask through this asker, with the standard timeout.
    pub fn new(asker: Arc<dyn Asker>) -> Self {
        Self {
            asker,
            timeout_ms: APPROVAL_TIMEOUT_MS,
        }
    }

    /// Ask through this asker, with a stated timeout. A test uses a short one.
    pub fn with_timeout_ms(asker: Arc<dyn Asker>, timeout_ms: u64) -> Self {
        Self { asker, timeout_ms }
    }
}

#[async_trait]
impl ApprovalPolicy for DialogApproval {
    /// Ask the client, and allow only an explicit yes.
    ///
    /// The message names the tool and its kind, and **not the arguments**. A tool
    /// argument can hold a secret, and this crate has no redactor, so sending the
    /// arguments would put a secret on the wire. See D-redact-tool-arguments. A later
    /// change may add redacted arguments.
    async fn approve(
        &self,
        tool: &str,
        kind: ToolKind,
        _args: &serde_json::Value,
    ) -> ApprovalDecision {
        let id = self.asker.next_dialog_id();
        let request = DialogRequest::Confirm {
            id,
            title: format!("Allow the tool {tool}?"),
            message: format!(
                "The agent wants to run {tool}, which is a {} operation.",
                kind_word(kind)
            ),
            timeout_ms: Some(self.timeout_ms),
        };
        match self.asker.ask(request).await {
            DialogAnswer::Confirmed(true) => ApprovalDecision::Allow,
            // Everything else denies: an explicit no, a dismissal, a timeout, and a
            // client that answers with the wrong shape. A denied safe tool is an
            // annoyance, and an approved destructive tool is a breach.
            DialogAnswer::Confirmed(false) | DialogAnswer::Cancelled | DialogAnswer::Value(_) => {
                ApprovalDecision::Deny
            }
        }
    }
}

/// One plain word for a tool kind, for a human reading a dialog.
///
/// The match has no wildcard arm, so a new `ToolKind` variant is a compile error and
/// never an unnamed operation in a security prompt.
fn kind_word(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "file edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "command",
        ToolKind::Think => "thinking",
        ToolKind::Fetch => "network fetch",
        ToolKind::SwitchMode => "mode change",
        // `Other` means the tool author declared no kind. Say so, rather than call it
        // safe. See D-plugin-does-not-classify-itself.
        ToolKind::Other => "unclassified",
    }
}
