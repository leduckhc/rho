# rho non-goals

This document lists what rho will not do and the reason for each decision. Read this before proposing a new feature.

---

## N-01 — No WASM plugin tier in sprint 1

**What rho will not do:** rho will not load WebAssembly modules as plugins in sprint 1.

**Reason:** WASM sandboxing requires a runtime (`wasmtime` or `wasmer`), which adds binary size, compile time, and a new security surface. The Tier 1 (compiled Rust traits) and Tier 2 (stdio JSON-RPC) plugin model covers all known use cases. WASM can be added in a later sprint if a compelling use case appears. See `docs/adr/ADR-001-plugin-mechanism.md` (written in stage S2).

---

## N-02 — No Node.js runtime

**What rho will not do:** rho will not embed or require a Node.js runtime.

**Reason:** Node.js is the primary cost driver for pi. pi uses ~76.5 MB resident memory per session and ~596 ms cold start (jcode.sh benchmarks, August 2026). Fifty concurrent sessions would consume over 3.8 GB. rho's goal is the opposite: one Rust binary with no script engine.

---

## N-03 — No built-in OAuth login flows

**What rho will not do:** rho will not implement OAuth for subscription-based providers (ChatGPT Plus, Claude Pro, GitHub Copilot).

**Reason:** OAuth with PKCE for chat subscription APIs requires reverse-engineering private endpoints. This is fragile and carries legal risk. rho supports API key and standard credential-chain auth. A third party can implement OAuth as a `CredentialResolver` plugin if needed.

---

## N-04 — No embedded semantic memory

**What rho will not do:** rho will not ship a built-in local embedding model or semantic vector store.

**Reason:** An embedding model adds 100 MB or more to the binary and requires GPU or CPU inference setup. This conflicts directly with the memory footprint goal. A third party can implement semantic memory as an out-of-process plugin (F-42).

---

## N-05 — No desktop app

**What rho will not do:** rho will not ship an Electron or Tauri desktop application.

**Reason:** Electron and Tauri each add hundreds of megabytes to the installed size. The owner's use case does not need a desktop app. `rho-tui` provides a terminal UI. `rho-acp` provides a headless JSON-RPC interface that any desktop app can drive.

---

## N-06 — No self-modifying source mode

**What rho will not do:** rho will not allow the agent to modify its own source code or configuration without explicit user review.

**Reason:** Self-modification without a trust boundary is a security risk. The owner's use case (multi-session coding assistant) does not require it. If a future sprint adds this, a full security audit is required first.

---

## N-07 — No built-in telemetry reporting to a remote service

**What rho will not do:** rho will not send usage data, crash reports, or analytics to any remote service.

**Reason:** rho is MIT-licensed infrastructure. Users run it in private environments. Opt-in telemetry adds a network dependency to the critical path and a privacy concern for enterprise users. Operators who want usage data can attach a `tracing::Subscriber` that exports to their own endpoint (F-100).

---

## N-08 — No GUI session export or sharing

**What rho will not do:** rho will not export sessions to HTML or upload them as shareable links in sprint 1.

**Reason:** These are convenience features that do not affect the core agent loop. They can be added as plugins in a later sprint.

---

## N-09 — No global extension registry or package manager

**What rho will not do:** rho will not ship a built-in registry or install command for third-party extensions in sprint 1.

**Reason:** A registry requires infrastructure (server, auth, signing) and a security review. Users reference plugins by crate path or subprocess path. A community registry can be built later, outside the rho binary.

---

## N-10 — No HTTP server mode

**What rho will not do:** `rho-core` will not start an HTTP server.

**Reason:** HTTP adds a dependency on an async HTTP server crate and is not necessary for the use cases in sprint 1. `rho-acp` uses stdio JSON-RPC, which works over any byte-stream transport. Callers who need HTTP can proxy stdio to HTTP outside rho.
