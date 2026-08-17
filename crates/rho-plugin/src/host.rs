//! The plugin host. It launches plugins, advertises their tools, and shuts them
//! down cleanly.

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
}

/// The plugin host. It owns every launched plugin and the cached tool specs.
pub struct PluginHost {
    plugins: Vec<Arc<PluginProcess>>,
    cached: Vec<PluginCache>,
    call_timeout: Duration,
}

impl PluginHost {
    /// Build an empty host with the default per-call timeout.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            cached: Vec::new(),
            call_timeout: DEFAULT_CALL_TIMEOUT,
        }
    }

    /// Set the per-call timeout. A test uses a short timeout, so a hung plugin
    /// does not stall the suite.
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Load a schema cache. The host advertises the cached tools before the
    /// plugin connects, so the prompt prefix is stable from turn one. See F-43.
    pub fn load_cache(&mut self, cache: PluginCache) {
        self.cached.push(cache);
    }

    /// Launch a plugin from a command, then handshake and read its tool list.
    pub async fn launch(
        &mut self,
        command: &str,
        args: &[String],
    ) -> Result<Arc<PluginProcess>, PluginError> {
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

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}
