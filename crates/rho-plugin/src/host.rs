//! The plugin host. It launches plugins, advertises their tools, and shuts them
//! down cleanly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rho_core::Tool;

use crate::cache::PluginCache;
use crate::process::{DEFAULT_CALL_TIMEOUT, PluginProcess};
use crate::proxy::{CachedTool, PluginTool};

/// An error from the plugin host or a plugin call.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("failed to launch plugin: {0}. Check the command path.")]
    Launch(String),
    #[error("handshake failed: {0}. Check the plugin speaks the rho protocol.")]
    Handshake(String),
    #[error("plugin call timed out. The plugin did not answer in time.")]
    Timeout,
    #[error("plugin process is not available. It may have crashed.")]
    Unavailable,
    #[error("protocol error: {0}.")]
    Protocol(String),
    #[error("the plugin call was canceled.")]
    Canceled,
    #[error(
        "refused to launch the plugin at {path}: {reason}. \
         Move the plugin outside the session root, or set an explicit policy."
    )]
    Refused { path: String, reason: String },
}

/// What the host will agree to launch.
///
/// A plugin is a program the host executes, so the decision to run one is a trust
/// decision. State it once, at construction, rather than at each call. Decision D-session-config
/// records why a security-relevant value belongs in a constructor.
#[derive(Clone, Debug, Default)]
pub struct PluginPolicy {
    /// Refuse a plugin that lives under this directory.
    ///
    /// Set this to the session root. A repository must not be able to hand executable
    /// code to the agent that is reading it. Otherwise a checked-in script becomes a
    /// tool as soon as somebody points rho at the repository, and the model can write
    /// such a script itself with `write` or `bash`.
    pub untrusted_root: Option<PathBuf>,
    /// Refuse a plugin that any user can write.
    ///
    /// A world-writable program is a classic escalation path: another local user
    /// replaces the file, and the host runs their code.
    pub refuse_world_writable: bool,
}

impl PluginPolicy {
    /// A policy that refuses a plugin under `root`, and refuses a world-writable one.
    pub fn confined_to_outside(root: impl Into<PathBuf>) -> Self {
        Self {
            untrusted_root: Some(root.into()),
            refuse_world_writable: true,
        }
    }

    /// A policy that checks only that the plugin exists and can run.
    ///
    /// Name it out loud at the call site. It trusts every path, so use it only where
    /// the caller already controls the path, and in a test.
    pub fn trust_any_path() -> Self {
        Self {
            untrusted_root: None,
            refuse_world_writable: false,
        }
    }
}

impl PluginPolicy {
    /// Decide whether the host may launch `command`.
    ///
    /// The checks run in order of cost. Existence first, then the trust rules.
    pub fn check(&self, command: &str) -> Result<(), PluginError> {
        let path = Path::new(command);

        // Resolve the path, so `..` and a symlink cannot dodge the root check. A path
        // that does not resolve cannot be launched anyway.
        let resolved = std::fs::canonicalize(path).map_err(|error| PluginError::Refused {
            path: command.to_string(),
            reason: format!("cannot resolve the path: {error}"),
        })?;

        let metadata = std::fs::metadata(&resolved).map_err(|error| PluginError::Refused {
            path: command.to_string(),
            reason: format!("cannot read the file: {error}"),
        })?;
        if !metadata.is_file() {
            return Err(PluginError::Refused {
                path: command.to_string(),
                reason: "the path is not a file".to_string(),
            });
        }

        if let Some(root) = &self.untrusted_root {
            // Resolve the root too, or a symlinked root would never match.
            let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            if resolved.starts_with(&root) {
                return Err(PluginError::Refused {
                    path: command.to_string(),
                    reason: format!(
                        "the plugin lives under the session root {}. A repository must \
                         not supply executable code to an agent that reads it",
                        root.display()
                    ),
                });
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = metadata.permissions().mode();
            if mode & 0o111 == 0 {
                return Err(PluginError::Refused {
                    path: command.to_string(),
                    reason: "the file is not executable".to_string(),
                });
            }
            if self.refuse_world_writable && mode & 0o002 != 0 {
                return Err(PluginError::Refused {
                    path: command.to_string(),
                    reason: "any user can write this file, so another user could \
                             replace it before it runs"
                        .to_string(),
                });
            }
        }
        Ok(())
    }
}

/// The plugin host. It owns every launched plugin and the cached tool specs.
pub struct PluginHost {
    plugins: Vec<Arc<PluginProcess>>,
    cached: Vec<PluginCache>,
    call_timeout: Duration,
    policy: PluginPolicy,
}

impl PluginHost {
    /// Build a host with an explicit trust policy.
    ///
    /// There is no `new()` without a policy on purpose. Decision D-no-four-argument-session-new deleted a
    /// convenience constructor that hid a security choice, and this is the same
    /// shape: a host that will run any program must say so.
    pub fn new(policy: PluginPolicy) -> Self {
        Self {
            plugins: Vec::new(),
            cached: Vec::new(),
            call_timeout: DEFAULT_CALL_TIMEOUT,
            policy,
        }
    }

    /// Set the per-call timeout. A test uses a short timeout, so a hung plugin
    /// does not stall the suite.
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Load a schema cache. The host advertises the cached tools before the
    /// plugin connects, so the prompt prefix is stable from turn one. See F-plugin-schema-cache.
    pub fn load_cache(&mut self, cache: PluginCache) {
        self.cached.push(cache);
    }

    /// Launch a plugin from a command, then handshake and read its tool list.
    pub async fn launch(
        &mut self,
        command: &str,
        args: &[String],
    ) -> Result<Arc<PluginProcess>, PluginError> {
        self.policy.check(command)?;
        let process = PluginProcess::launch(command, args, self.call_timeout).await?;
        self.plugins.push(Arc::clone(&process));
        Ok(process)
    }

    /// The proxied tools from every live plugin, plus tools from a cache whose
    /// plugin has not connected yet.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for plugin in &self.plugins {
            for spec in plugin.tools() {
                tools.push(Arc::new(PluginTool::new(Arc::clone(plugin), spec.clone())));
            }
        }
        // Add a cached tool only when no live plugin already advertises it.
        for cache in &self.cached {
            for spec in &cache.tools {
                let already = tools.iter().any(|t| t.name() == spec.name);
                if !already {
                    tools.push(Arc::new(CachedTool::new(spec.clone())));
                }
            }
        }
        tools
    }

    /// Shut down every plugin. No orphan process is left behind.
    pub async fn shutdown(&mut self) {
        for plugin in &self.plugins {
            plugin.shutdown().await;
        }
        self.plugins.clear();
    }
}

// There is deliberately no `Default` impl for `PluginHost`.
//
// A default would have to pick a trust policy, and the only policy that works without
// context is the permissive one. Decision D-no-four-argument-session-new deleted a constructor that hid exactly
// that kind of choice. A caller states its policy.
