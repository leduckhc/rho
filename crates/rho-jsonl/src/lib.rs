//! `rho-jsonl` is the headless JSONL frontend for rho.
//!
//! A client writes one JSON command per line to stdin. rho writes one reply or one
//! event per line to stdout. Any process in any language can drive rho this way, so
//! a host embeds the agent with no terminal.
//!
//! The contract lives in `docs/specs/20260819-102749-SPEC-jsonl-frontend.md`. It is
//! public, so a third party writes a client without forking rho.
//!
//! This crate depends on `rho-core` only. It links no provider and no HTTP client. A
//! host supplies a [`SessionFactory`], which is the extension point. See decision
//! D-rho-jsonl-asks-a-factory-for-a-session.
//!
//! This protocol is not JSON-RPC. The wire has no `jsonrpc` field, no `method` at
//! the top level, and no numeric error codes. Do not call it RPC. See decision
//! D-jsonl-before-acp.

mod factory;
mod frame;
mod protocol;

pub use factory::{FactoryError, SessionFactory, SessionRequest};
pub use frame::{Line, LineReader, MAX_COMMAND_LINE_BYTES};
pub use protocol::{
    AnswerError, Command, DialogAnswer, DialogRequest, Event, False, FaultKind, Reply, ReplyErr,
    ReplyError, ReplyOk, SettleReason, True,
};
