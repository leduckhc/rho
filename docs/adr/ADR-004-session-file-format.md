# ADR-004 — Session file format

Status: accepted. Sprint 2.
Deciders: the architect.

## Context

rho must persist a conversation to disk. A user resumes a session, branches from an
earlier turn, and lists past sessions. The owner's app `makit` drives rho over ACP, and
ACP `session/load` and `session/resume` both need a stored format. See F-50 to F-54 and
`SPEC-14`.

The format has hard constraints.

- A third party reads the file directly. F-50 promises this, so the format stays plain.
- The core stays cheap. rho holds many sessions in one process, so a per-session file
  handle and a per-record cost both matter.
- A crash must not corrupt the file. A session runs for a long time, so a crash mid-run
  is normal, not rare.
- A branch must be cheap. A user branches often, so a branch must not copy the file.

Three shapes were possible.

1. Append-only JSONL, one record per line, each record with an id and a parent id.
2. A single JSON document, rewritten on each change.
3. An embedded database, for example SQLite.

## Decision

Use append-only JSONL. One record sits on one line. Each record carries an id, a parent
id, and a timestamp. rho appends a record and never rewrites a line. A branch is a new
record that names an earlier record as its parent. See `SPEC-14`.

## Why JSONL with a parent pointer

- The append path is one write. rho adds a line and never touches an earlier byte. So a
  crash loses at most a buffered tail, and never corrupts a written record.
- A branch is a tree walk, not a file copy. A new leaf names its parent, and two
  branches share their prefix on disk. So a branch costs the new records only.
- A third party reads the file with any JSONL reader. The format needs no rho library.
  This keeps the F-50 promise real, not aspirational.
- The shape matches what pi writes, so the import path in `SPEC-14` section 9 is a
  field map, not a parser. rho keeps its own record set and its own version.
- The record model reuses `rho_core::Message`, so the on-disk content and the in-memory
  content are the one type. A drift between them is then impossible.

## What it costs

- A read parses every line. A very long session costs a linear read at resume. This is
  acceptable, because a resume is rare against a turn, and the read is bounded by the
  file size.
- A record repeats its structural keys on every line. JSONL is not the smallest
  encoding. The record-size cap in `SPEC-14` section 3 bounds the worst case, and the
  format stays readable, which the smaller encodings lose.
- The codec choice matters for throughput. `ADR-005` measures it, and `SPEC-14` makes
  the codec a narrow seam behind `encode` and `decode`. The default is `serde_json`,
  and `sonic-rs` sits behind the `fast-json` feature.

## Alternative rejected: a single rewritten JSON document

- Every change rewrites the whole file. A long session then pays an O(n) write per
  message, which grows without bound as the session grows.
- A crash mid-rewrite corrupts the whole session, not one tail line. So a rewrite trades
  the append-only safety for nothing.
- A branch needs a copy, or an in-document tree that a third party cannot read simply.
  So it breaks the F-50 promise and the cheap-branch requirement at once.

## Alternative rejected: an embedded database such as SQLite

- It adds a C dependency and a file lock. That is heavier than the footprint budget in
  `ADR-002` allows for a core that holds 50 sessions.
- A file lock contends across many sessions in one process. So the density that
  justifies rho works against a per-file database lock.
- A third party can no longer read the session with a plain reader. It needs the
  database engine and the schema. So it breaks F-50.
- The crash story is better than a rewrite, but the cost and the opacity are worse than
  JSONL, and JSONL already meets the crash requirement.

## Alternative rejected: adopting pi's format directly

- pi's format encodes pi's own message and content model. Adopting it leaks pi's model
  into `rho-core`, and the core stops being free to normalise providers its own way.
- This is decision D-001, and it stands. rho keeps its own record set, and a one-way
  import converts a pi file. So rho reads pi's shape without adopting pi's model.

## The compatibility promise

- The first record is the header, and it carries a `version`. A reader refuses a version
  it does not know, rather than guess. The test `resume_refuses_an_unknown_version` in
  `SPEC-14` guards this, so `SessionError::Version` is proved and not just described.
- The header also carries the resolved `approval` and `sandbox` mode names. A resume
  reads them, so a session that ran read-only never resumes under a wider mode by
  accident. A resume refuses to widen either mode unless the user passes `--allow-widen`.
  See `SPEC-14` section 8a. This keeps a stored permission from silently widening under a
  live config.
- A new record type is additive. A reader ignores a record type it does not know, so an
  old reader still loads a newer file's known records.
- The shared fields are stable. Every record carries an `id`, a `parentId`, and a
  `timestamp`, and those names do not change.
- A codec change is invisible. Both codecs produce a byte-identical line for a record,
  and each reads the other's output. See `SPEC-14` section 3 and `ADR-005`.

## Consequences

- The record set in `SPEC-14` is the stable on-disk contract. A change to it is a
  breaking change and needs a spec update and a version bump.
- The parent pointer is the one mechanism for branching, resume, and fork. A frontend
  and the import path both rely on it.
- The session log is a consumer of the event stream, not a field in `Session`. So the
  format and the runtime stay separate, and either can change alone. See `SPEC-14`
  section 5.
