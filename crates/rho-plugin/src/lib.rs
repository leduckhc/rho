//! The out-of-process plugin host for rho (Tier 2).
//!
//! A plugin is a subprocess in any language. It speaks JSON-RPC 2.0 over stdio,
//! one JSON object per line, LF framing. This crate launches the subprocess,
//! runs the handshake, lists the plugin tools, and serves each call. A plugin
//! runs in its own process, so a crash cannot take down the session. See
//! `SPEC-04`.

mod cache;
mod host;
mod process;
mod proxy;

pub use cache::{PluginCache, PluginToolSpec};
pub use host::{PluginError, PluginHost};
pub use process::PluginProcess;
