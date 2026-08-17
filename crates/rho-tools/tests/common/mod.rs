//! Shared test support for the `rho-tools` behaviour tests.
//!
//! It builds a `ToolContext` rooted at a temporary directory. It also collects
//! the streamed update lines a tool sends, so a test can assert on them.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use rho_core::{CancelToken, ToolContext};
use tempfile::TempDir;

/// A temporary session root plus the machinery a tool run needs.
pub struct Harness {
    /// The temporary directory. It deletes on drop, so keep it alive.
    pub dir: TempDir,
    /// The cancel token for the run. A test may cancel it.
    pub cancel: CancelToken,
    /// The lines a tool streamed on `ctx.updates`.
    pub updates: Arc<Mutex<Vec<String>>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl Harness {
    /// Build a harness with a fresh temporary root.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("a test needs a temporary directory");
        let cancel = CancelToken::new();
        let updates = Arc::new(Mutex::new(Vec::new()));
        Self {
            dir,
            cancel,
            updates,
            join: None,
        }
    }

    /// The canonical session root path.
    pub fn root(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// Build a fresh `ToolContext`. It spawns a task that records every streamed
    /// line into `self.updates`.
    pub fn ctx(&mut self) -> ToolContext {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
        let sink = Arc::clone(&self.updates);
        self.join = Some(tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                sink.lock().unwrap().push(line);
            }
        }));
        ToolContext {
            session_root: self.root(),
            cancel: self.cancel.clone(),
            updates: tx,
        }
    }

    /// The lines streamed so far.
    pub fn lines(&self) -> Vec<String> {
        self.updates.lock().unwrap().clone()
    }

    /// Write a file under the root. It creates parent directories.
    pub fn write_file(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    /// Read a file under the root back as text.
    pub fn read_file(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(rel)).unwrap()
    }
}

/// True when `p` exists.
pub fn exists(p: &Path) -> bool {
    p.exists()
}
