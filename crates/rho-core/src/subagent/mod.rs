//! Subagents: a child session, confined, bounded, and observable.
//!
//! One file held all of this and grew to 1241 lines, which is where a reader stops
//! being able to scan it. `intersect_tools` has nothing to say to `collect_report`, so
//! each concern now has a file:
//!
//! | Module | Concern |
//! | --- | --- |
//! | `confine` | The security core: composition, intersection, narrowing, budget caps |
//! | `limits` | [`SubagentLimits`], the four bounds and their defaults |
//! | `report` | The result contract: [`AgentReport`] and [`AgentOutcome`] |
//! | `error` | [`SubagentError`], where every refusal teaches |
//! | `tree` | The spawn tree, the registry, and a live child's handle |
//! | `retry` | [`RetryLedger`], jcode's reclaim cap |
//! | `collect` | Turning a child's event stream into a report |
//!
//! The public surface is unchanged. Everything is re-exported here, so
//! `crate::subagent::X` still resolves and `lib.rs` needed no edit.
//!
//! See `docs/specs/20260818-000223-SPEC-subagents.md` and
//! `docs/contracts-subagents.md`.

mod collect;
mod confine;
mod error;
mod limits;
mod report;
mod retry;
mod tree;

pub use collect::{CollectOptions, collect_report};
pub use confine::{BothPolicies, ToolIntersection, intersect_tools, narrow_sandbox};
pub use error::SubagentError;
pub use limits::{DEFAULT_SUBAGENT_GRACE_TURNS, SubagentLimits};
pub use report::{AgentOutcome, AgentReport, MAX_SUMMARY_CHARS};
pub use retry::{MAX_CHILD_RETRIES, RetryLedger};
pub use tree::{
    AgentId, AgentNode, AgentProgress, AgentRegistry, AgentStatus, ChildSlot, ChildSpawn,
    LiveAgent, cap_tool_calls, check_no_cycle,
};
