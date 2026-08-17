# SPEC-12 — The budget governor

Status: draft. Blocked on `SPEC-11`, the subagent work, because a governor must cap a
whole agent tree and not one session.
Owning crate: `rho-core`, with the enforcement in a `Hook`.
Features: F-143 restated and widened, plus a new row for the shared case.

## 1. Why a governor, and why rho can do it differently

A coding agent spends money on every turn. An agent that can spawn children spends it
faster, and `SPEC-11` makes a child cost about 247 KB, so the cheap thing to create is not
the cheap thing to run. **Density without a budget is a way to spend money in silence.**

Two harnesses meter usage. Both meter **per process**, because both run one process per
session. So neither can cap a fleet without an outside coordinator that watches many
processes and cannot stop one mid-turn.

rho holds many sessions in one address space. A fleet-wide cap is therefore a shared
counter and an atomic read, checked inside the hook chain that already runs before every
tool call and around every model call. That is the whole mechanism.

**So the lead is not "rho has a cost meter". Every harness has one. The lead is that one
limit can bind fifty sessions and every subagent under them, and stop the next call
rather than report the overspend afterwards.**

## 2. What it counts

| Quantity | Source | Note |
| --- | --- | --- |
| Input and output tokens | `Usage` on the model response | Already reported by all three providers. |
| Cache reads and writes | `Usage` | Counted, and priced differently by a provider, which is why the next row matters. |
| Money | `Usage::cost_usd` | The charge the provider reported. **Never an estimate from a price table.** See decision D-032. |
| Wall-clock time | The session clock | The only limit that binds when a provider reports no charge. |
| Turns | The agent loop | Already capped per run by `AgentConfig::max_turns`. The governor caps a tree. |

**The awkward case, and the spec must not hide it.** Azure and Bedrock report no charge on
the stream, so `cost_usd` is absent there. A money cap therefore binds only where a
provider reports money. On the others the governor must fall back to tokens and wall time,
and it must **say so at configuration time** rather than silently failing to enforce. A cap
that quietly does nothing is worse than no cap.

## 3. Scopes

```rust
/// What a limit applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScope {
    /// One session, and nothing else.
    Session,
    /// One session plus every subagent beneath it. See SPEC-11.
    Tree,
    /// Every session in the process. The scope no competitor can offer.
    Fleet,
}
```

A `Fleet` limit is the interesting one, and it is only possible because the sessions share a
process.

## 4. Public API

```rust
/// One limit. Every field is optional, and an absent field means no limit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Budget {
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_cost_usd: Option<f64>,
    pub max_wall_clock: Option<Duration>,
}

/// The shared meter. Cheap to clone, and safe to share across sessions.
pub struct Governor { /* private */ }

impl Governor {
    pub fn new() -> Arc<Self>;

    /// Set the limit for a scope. Setting it again replaces it.
    pub fn set(&self, scope: BudgetScope, budget: Budget);

    /// Record usage against a session and its ancestors.
    pub fn record(&self, session: SessionId, usage: &Usage);

    /// Would the next call pass every limit that binds this session?
    ///
    /// Returns the first limit that would be exceeded, so a caller can name it.
    pub fn check(&self, session: SessionId) -> Result<(), BudgetExceeded>;

    /// A snapshot, for a status line and for a test.
    pub fn spent(&self, scope: BudgetScope, session: SessionId) -> Usage;
}

/// Which limit stopped the work, and by how much.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetExceeded {
    pub scope: BudgetScope,
    pub limit: &'static str,
    pub spent: f64,
    pub allowed: f64,
}
```

`BudgetExceeded` carries the numbers on purpose. "Budget exceeded" tells a user nothing.
"The fleet money cap is 5.00 dollars and 5.02 are spent" tells them what to change.

## 5. Where it binds

The governor is enforced in two places, and both already exist.

- **Before a tool call**, in `Hook::before_tool_call`. Over budget means `HookOutcome::Block`
  with a reason that names the limit. The model reads the reason, so it stops rather than
  retrying.
- **Around a model call**, which needs the request and response hook points that
  `SPEC-04` records as F-162. Until those land, the governor reads usage from the event
  stream, which is one turn late. **One turn late is the honest limit of the first
  version**, and the spec says so rather than implying the cap is exact.

## 6. Design rules

- **A limit is absent by default.** No cap unless a caller sets one, and the default is
  stated in the constructor. Decision D-013 removed a constructor that hid a choice.
- **Refusing must teach.** Name the scope, the limit, the amount spent, and what to change.
- **Absent is not zero.** A provider that reports no charge must not read as free. Decision
  D-032 made `Usage::add` keep that distinction, and the governor must not undo it.
- **A stopped session stays inspectable.** Hitting a cap ends the work and keeps the
  transcript, so a user can read what was bought.

## 7. Test cases

- `a_session_limit_stops_that_session_only`
- `a_tree_limit_stops_a_parent_and_its_children`
- `a_fleet_limit_stops_every_session` — the case a competitor cannot express.
- `the_error_names_the_scope_the_limit_and_the_amount`
- `an_absent_limit_never_blocks`
- `an_absent_cost_does_not_count_as_zero_spend` — the D-032 rule, restated where it is easy
  to break.
- `a_money_cap_on_a_provider_that_reports_no_cost_warns_at_configuration_time`
- `a_wall_clock_cap_binds_when_no_cost_is_reported`
- `recording_usage_from_many_sessions_at_once_is_exact` — a concurrency test with many
  tasks and no `sleep`, since the whole point is a shared counter.
- `a_blocked_call_leaves_the_transcript_readable`

## 8. What this does not give you

- **It is not a rate limiter.** It caps a total, not a rate.
- **The first version is one turn late** on the model call, until F-162 lands.
- **A money cap is only as good as the provider's reporting.** Two of the three report no
  charge today.
- **It does not stop a runaway shell command.** That is `bash`'s timeout and `SPEC-10`'s
  sandbox.

## 9. Out of scope

- Persisting spend across a restart.
- A per-user or per-tenant scope. That needs a tenancy model, and see decision D-034 for
  why rho does not have one yet.
- Predicting the cost of a call before making it.
