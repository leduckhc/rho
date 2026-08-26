//! The shared server pool.
//!
//! rho exists to run many sessions at once, and the measured cost is about 25 KB
//! per extra session. An MCP server is a whole process, often tens of megabytes.
//! If every session spawned its own copy of every server, fifty sessions with
//! three servers each would spawn 150 processes and undo the footprint argument.
//!
//! So a server is shared by default, keyed by the config fingerprint, and
//! reference counted. The last session to release it stops it. A server that
//! sets `shared: false` starts one process per session. See `SPEC-mcp` sections 1
//! and 7, and decision D-mcp-shared-by-default.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::watch;

use crate::client::{McpConnection, handshake};
use crate::config::McpServerConfig;
use crate::error::McpError;
use crate::limits::McpLimits;
use crate::transport::{DefaultTransportFactory, TransportFactory};

/// The connection state of one pooled server.
#[derive(Clone)]
enum ConnState {
    /// The background task has not finished the handshake yet.
    Connecting,
    /// The handshake succeeded. The connection is ready.
    Ready(Arc<McpConnection>),
    /// The handshake failed. The reason names the server.
    Failed(String),
}

/// One pooled server. It holds the connection state and the connect timeout.
struct ServerSlot {
    server: String,
    state: watch::Sender<ConnState>,
    connect_timeout: Duration,
}

impl ServerSlot {
    /// Wait for the connection, bounded by the connect timeout.
    ///
    /// A `tools/call` that arrives before the handshake finishes waits here. This
    /// is connect-on-first-call. A blocked call fails after the timeout instead
    /// of hanging a turn.
    async fn connection(&self) -> Result<Arc<McpConnection>, McpError> {
        let mut rx = self.state.subscribe();
        loop {
            {
                let state = rx.borrow_and_update();
                match &*state {
                    ConnState::Ready(connection) => return Ok(Arc::clone(connection)),
                    ConnState::Failed(reason) => {
                        return Err(McpError::Handshake {
                            server: self.server.clone(),
                            reason: reason.clone(),
                        });
                    }
                    ConnState::Connecting => {}
                }
            }
            match tokio::time::timeout(self.connect_timeout, rx.changed()).await {
                Ok(Ok(())) => continue,
                // The sender dropped. The pool released the slot.
                Ok(Err(_)) => {
                    return Err(McpError::Unavailable {
                        server: self.server.clone(),
                    });
                }
                Err(_) => {
                    return Err(McpError::ConnectTimeout {
                        server: self.server.clone(),
                    });
                }
            }
        }
    }
}

/// One pool entry: the slot plus its reference count.
struct ServerEntry {
    slot: Arc<ServerSlot>,
    refcount: usize,
}

/// The shared pool. One process per config fingerprint, reference counted.
pub struct McpPool {
    limits: McpLimits,
    factory: Arc<dyn TransportFactory>,
    servers: Mutex<HashMap<String, ServerEntry>>,
    private_counter: AtomicU64,
    /// Where a successful handshake records its tool list. `None` records nothing, which is
    /// what a test wants. Without it the cache was never written and no MCP tool ever
    /// reached the model. See `SPEC-wire-the-dead-switches`.
    cache_path: Mutex<Option<std::path::PathBuf>>,
}

impl McpPool {
    /// Build a pool that opens stdio servers. An HTTP server needs a supplied
    /// factory; see [`McpPool::with_factory`].
    pub fn new(limits: McpLimits) -> Arc<Self> {
        Self::with_factory(limits, Arc::new(DefaultTransportFactory))
    }

    /// Build a pool with a supplied transport factory.
    ///
    /// A caller passes an HTTP transport factory here, because rho-mcp links no
    /// HTTP client. A test passes a fake factory, so no test reaches the network.
    pub fn with_factory(limits: McpLimits, factory: Arc<dyn TransportFactory>) -> Arc<Self> {
        Arc::new(Self {
            limits,
            factory,
            servers: Mutex::new(HashMap::new()),
            private_counter: AtomicU64::new(0),
            cache_path: Mutex::new(None),
        })
    }

    /// Record a successful handshake's tool list at this path.
    ///
    /// The pool is behind an `Arc` by the time a caller has it, so this takes `&self` and
    /// stores the path behind its own lock rather than taking `self` by value.
    pub fn set_cache_path(&self, path: std::path::PathBuf) {
        *self
            .cache_path
            .lock()
            .expect("the cache path mutex is never poisoned") = Some(path);
    }

    /// Take a reference to a server, connecting in the background if needed.
    ///
    /// The connect runs on a background task, so this returns at once. It does
    /// not wait for the handshake.
    pub async fn acquire(
        self: &Arc<Self>,
        config: &McpServerConfig,
    ) -> Result<McpHandle, McpError> {
        let key = self.key_for(config);
        let mut servers = self.servers.lock().unwrap();
        let slot = if let Some(entry) = servers.get_mut(&key) {
            entry.refcount += 1;
            Arc::clone(&entry.slot)
        } else {
            let connect_timeout = Duration::from_millis(self.limits.connect_timeout_ms);
            let (tx, _rx) = watch::channel(ConnState::Connecting);
            let slot = Arc::new(ServerSlot {
                server: config.name.clone(),
                state: tx,
                connect_timeout,
            });
            self.spawn_connect(Arc::clone(&slot), config.clone());
            servers.insert(
                key.clone(),
                ServerEntry {
                    slot: Arc::clone(&slot),
                    refcount: 1,
                },
            );
            slot
        };
        Ok(McpHandle {
            pool: Arc::downgrade(self),
            key,
            slot: Arc::downgrade(&slot),
            server: config.name.clone(),
        })
    }

    /// How many live servers the pool holds. For a test and for a status line.
    pub async fn live_count(&self) -> usize {
        self.servers.lock().unwrap().len()
    }

    /// The pool key for a config.
    ///
    /// A shared server keys on its fingerprint, so two sessions with the same
    /// config share one process. A non-shared server gets a unique key, so it
    /// starts one process per session.
    fn key_for(&self, config: &McpServerConfig) -> String {
        let fingerprint = config.fingerprint();
        if config.shared {
            fingerprint
        } else {
            let unique = self.private_counter.fetch_add(1, Ordering::SeqCst);
            format!("{fingerprint}\u{1f}private={unique}")
        }
    }

    /// Spawn the background connect task for a slot.
    fn spawn_connect(&self, slot: Arc<ServerSlot>, config: McpServerConfig) {
        let factory = Arc::clone(&self.factory);
        let limits = self.limits;
        let cache_path = self
            .cache_path
            .lock()
            .expect("the cache path mutex is never poisoned")
            .clone();
        tokio::spawn(async move {
            let result = async {
                let pair = factory.open(&config, &limits).await?;
                handshake(pair, &config, &limits).await
            }
            .await;
            match result {
                Ok((connection, tools)) => {
                    // The only place that knows the handshake succeeded and still holds the
                    // whole list. `extensions::load` returns long before this runs, so a
                    // write there would cache nothing. A failed handshake takes the `Err`
                    // arm and writes nothing at all.
                    if let Some(path) = cache_path
                        && let Err(error) = crate::record_tools(&path, &config, tools)
                    {
                        tracing::debug!(%error, "the MCP schema cache was not written");
                    }
                    // `send_replace` updates the stored value even when no
                    // receiver exists yet, so a call that subscribes later still
                    // sees the ready connection. `send` would drop the update.
                    slot.state.send_replace(ConnState::Ready(connection));
                }
                Err(error) => {
                    slot.state
                        .send_replace(ConnState::Failed(error.to_string()));
                }
            }
        });
    }

    /// Release one reference to a key. Stop the server when the count reaches zero.
    fn release(&self, key: &str) {
        let mut servers = self.servers.lock().unwrap();
        if let Some(entry) = servers.get_mut(key) {
            entry.refcount -= 1;
            if entry.refcount == 0 {
                // Remove the entry. This drops the only strong reference to the
                // slot, so the connection drops and the process is stopped by kill
                // on drop. No orphan is left behind.
                servers.remove(key);
            }
        }
    }
}

/// A session's reference to a pooled server. Dropping it releases the reference.
pub struct McpHandle {
    pool: Weak<McpPool>,
    key: String,
    slot: Weak<ServerSlot>,
    server: String,
}

impl McpHandle {
    /// The live connection, waiting for the handshake if it is still running.
    pub(crate) async fn connection(&self) -> Result<Arc<McpConnection>, McpError> {
        match self.slot.upgrade() {
            Some(slot) => slot.connection().await,
            // The pool dropped the slot, so the server is gone.
            None => Err(McpError::Unavailable {
                server: self.server.clone(),
            }),
        }
    }
}

impl Drop for McpHandle {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            pool.release(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use async_trait::async_trait;
    use tokio::sync::mpsc;

    use crate::config::McpTransport;
    use crate::line::LineOutcome;
    use crate::transport::{TransportPair, TransportReader, TransportWriter};

    /// Counters a fake transport updates, so a test can prove how many processes
    /// the pool started and whether each one was stopped.
    #[derive(Default)]
    struct Counters {
        opens: AtomicUsize,
        alive: AtomicUsize,
    }

    /// A fake transport factory. It never reaches the network. It answers the
    /// handshake in memory, so a background connect finishes deterministically.
    struct FakeFactory {
        counters: Arc<Counters>,
    }

    #[async_trait]
    impl TransportFactory for FakeFactory {
        async fn open(
            &self,
            _config: &McpServerConfig,
            _limits: &McpLimits,
        ) -> Result<TransportPair, McpError> {
            self.counters.opens.fetch_add(1, Ordering::SeqCst);
            self.counters.alive.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = mpsc::unbounded_channel();
            Ok(TransportPair {
                reader: Box::new(FakeReader { rx }),
                writer: Arc::new(FakeWriter { tx }),
                guard: Box::new(AliveGuard {
                    counters: Arc::clone(&self.counters),
                }),
            })
        }
    }

    /// The fake reading half. It yields the responses the writer generated.
    struct FakeReader {
        rx: mpsc::UnboundedReceiver<String>,
    }

    #[async_trait]
    impl TransportReader for FakeReader {
        async fn next_line(&mut self) -> LineOutcome {
            match self.rx.recv().await {
                Some(line) => LineOutcome::Line(line),
                None => LineOutcome::Eof,
            }
        }
    }

    /// The fake writing half. It answers each request in memory.
    struct FakeWriter {
        tx: mpsc::UnboundedSender<String>,
    }

    #[async_trait]
    impl TransportWriter for FakeWriter {
        async fn send_line(&self, line: &str) -> Result<(), McpError> {
            let message: serde_json::Value = serde_json::from_str(line).unwrap();
            let Some(id) = message.get("id").cloned() else {
                // A notification has no id and needs no response.
                return Ok(());
            };
            let result = match message["method"].as_str().unwrap_or("") {
                "initialize" => serde_json::json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": { "name": "fake", "version": "0.1.0" }
                }),
                "tools/list" => serde_json::json!({ "tools": [] }),
                _ => serde_json::json!({ "content": [], "isError": false }),
            };
            let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
            let _ = self.tx.send(response.to_string());
            Ok(())
        }

        async fn close(&self) {}
    }

    /// A process guard. Dropping it marks the fake process stopped.
    struct AliveGuard {
        counters: Arc<Counters>,
    }

    impl Drop for AliveGuard {
        fn drop(&mut self) {
            self.counters.alive.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// A factory whose `open` never returns, so a background connect never
    /// finishes. It proves the pool does not block on a handshake.
    struct BlockingFactory;

    #[async_trait]
    impl TransportFactory for BlockingFactory {
        async fn open(
            &self,
            _config: &McpServerConfig,
            _limits: &McpLimits,
        ) -> Result<TransportPair, McpError> {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    fn config(name: &str, shared: bool) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransport::Stdio {
                command: name.to_string(),
                args: vec![],
            },
            env: Default::default(),
            shared,
            call_timeout_ms: None,
        }
    }

    fn fake_pool() -> (Arc<McpPool>, Arc<Counters>) {
        let counters = Arc::new(Counters::default());
        let factory = Arc::new(FakeFactory {
            counters: Arc::clone(&counters),
        });
        let pool = McpPool::with_factory(McpLimits::default(), factory);
        (pool, counters)
    }

    #[tokio::test]
    async fn two_sessions_sharing_a_config_start_one_process() {
        let (pool, counters) = fake_pool();
        let config = config("shared-server", true);
        let handle_a = pool.acquire(&config).await.unwrap();
        let handle_b = pool.acquire(&config).await.unwrap();
        // Wait for the shared server to connect, so the open count is settled.
        handle_a.connection().await.unwrap();
        handle_b.connection().await.unwrap();
        assert_eq!(counters.opens.load(Ordering::SeqCst), 1, "one process only");
        assert_eq!(pool.live_count().await, 1);
    }

    #[tokio::test]
    async fn a_server_marked_not_shared_starts_one_process_per_session() {
        let (pool, counters) = fake_pool();
        let config = config("private-server", false);
        let handle_a = pool.acquire(&config).await.unwrap();
        let handle_b = pool.acquire(&config).await.unwrap();
        handle_a.connection().await.unwrap();
        handle_b.connection().await.unwrap();
        assert_eq!(
            counters.opens.load(Ordering::SeqCst),
            2,
            "one process per session"
        );
        assert_eq!(pool.live_count().await, 2);
    }

    #[tokio::test]
    async fn releasing_the_last_handle_stops_the_server() {
        let (pool, counters) = fake_pool();
        let config = config("server", true);
        let handle = pool.acquire(&config).await.unwrap();
        handle.connection().await.unwrap();
        assert_eq!(counters.alive.load(Ordering::SeqCst), 1);
        assert_eq!(pool.live_count().await, 1);
        drop(handle);
        assert_eq!(pool.live_count().await, 0, "the pool holds no server");
        assert_eq!(
            counters.alive.load(Ordering::SeqCst),
            0,
            "the process stopped"
        );
    }

    #[tokio::test]
    async fn releasing_one_of_two_handles_keeps_the_server_running() {
        let (pool, counters) = fake_pool();
        let config = config("server", true);
        let handle_a = pool.acquire(&config).await.unwrap();
        let handle_b = pool.acquire(&config).await.unwrap();
        handle_a.connection().await.unwrap();
        drop(handle_a);
        assert_eq!(pool.live_count().await, 1, "the server keeps running");
        assert_eq!(
            counters.alive.load(Ordering::SeqCst),
            1,
            "the process is alive"
        );
        drop(handle_b);
        assert_eq!(pool.live_count().await, 0);
    }

    #[tokio::test]
    async fn a_different_config_starts_a_different_process() {
        let (pool, counters) = fake_pool();
        let handle_a = pool.acquire(&config("server-a", true)).await.unwrap();
        let handle_b = pool.acquire(&config("server-b", true)).await.unwrap();
        handle_a.connection().await.unwrap();
        handle_b.connection().await.unwrap();
        assert_eq!(counters.opens.load(Ordering::SeqCst), 2);
        assert_eq!(pool.live_count().await, 2);
    }

    #[tokio::test]
    async fn dropping_the_pool_kills_every_server() {
        let (pool, counters) = fake_pool();
        let handle_a = pool.acquire(&config("server-a", true)).await.unwrap();
        let handle_b = pool.acquire(&config("server-b", true)).await.unwrap();
        handle_a.connection().await.unwrap();
        handle_b.connection().await.unwrap();
        assert_eq!(counters.alive.load(Ordering::SeqCst), 2);
        drop(pool);
        // The handles still exist, but the pool held the only strong slot
        // reference. Dropping the pool stops every server.
        assert_eq!(
            counters.alive.load(Ordering::SeqCst),
            0,
            "every process stopped"
        );
    }

    #[tokio::test]
    async fn acquire_returns_without_waiting_for_a_handshake() {
        // A blocking factory never finishes the handshake. `acquire` must still
        // return at once. A bounded timeout fails a blocking implementation.
        let pool = McpPool::with_factory(McpLimits::default(), Arc::new(BlockingFactory));
        let acquired = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            pool.acquire(&config("server", true)),
        )
        .await;
        assert!(acquired.is_ok(), "acquire must not wait for a handshake");
    }
}
