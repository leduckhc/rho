//! A child's transcript — JSONL, streamed, parent-readable.
//!
//! rho writes a transcript for every child session, in JSONL format. Each event
//! arrives as one line, so a parent can read a transcript as it arrives. The path
//! is returned to the parent, so it can read the file if it needs to.
//!
//! Transcript files live in a temp directory (`/tmp` on Unix, or the system temp).
//! Permissions are `0o700` so only the owning user reads them.

use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

/// A line written to the transcript.
///
/// **This is a persisted format, so it binds the next version of rho.**
///
/// Two things every line carries, borrowed from pi's `.output` format: a timestamp
/// and the agent that produced the line. Without them a reader cannot order two
/// children's files or tell them apart. The timestamp is epoch milliseconds as a
/// decimal string, like every other timestamp rho writes. See decision
/// D-one-timestamp-format.
///
/// An unknown `type` must be skipped by a reader, never rejected, so a new body kind
/// does not break an old reader.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// Epoch milliseconds, as a decimal string.
    pub ts: String,
    /// The agent that produced this line.
    pub agent: String,
    #[serde(flatten)]
    pub body: TranscriptBody,
}

impl TranscriptEntry {
    /// Stamp a body with the time and the agent.
    pub fn now(agent: impl Into<String>, body: TranscriptBody) -> Self {
        Self {
            ts: now_unix_ms().to_string(),
            agent: agent.into(),
            body,
        }
    }
}

/// What happened, in a shape a reader can match on.
///
/// The first draft carried `Event { turn, event: String }`, where `event` was a
/// debug-formatted name. That keeps the unreadable part unreadable, which is the
/// defect the whole file exists to fix. A body names its own fields instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TranscriptBody {
    /// A provider turn began.
    TurnStart { turn: u32 },
    /// The child produced text. This is the part a human reads.
    Text { text: String },
    /// A tool call started.
    ToolStart { id: String, name: String },
    /// Tool output arrived.
    ToolUpdate { id: String, line: String },
    /// Tool completed.
    ToolEnd { id: String, error: bool },
    /// Usage the provider reported for a turn.
    Usage { input: u64, output: u64 },
    /// The run ended. `outcome` matches `AgentOutcome`'s own wire name.
    End { outcome: String },
}

/// The current unix time in milliseconds. A clock error yields zero, which is
/// harmless for a transcript timestamp.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
}

/// A transcript writer. It streams JSONL to disk.
pub struct TranscriptWriter {
    /// The file path, so the parent knows where to read.
    pub path: PathBuf,
    /// A handle for appending lines. Guarded because multiple tasks may write.
    file: Mutex<std::fs::File>,
}

impl TranscriptWriter {
    /// Create a new transcript writer. The directory is created if needed.
    /// File permissions are set to `0o600` so only the owning user reads it.
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            // The directory needs the same care as the file. A 0o600 file inside a
            // world-readable directory still leaks how many children ran and what
            // they were called. pi sets 0o700 on its root for this reason.
            // Every directory rho created, not only the leaf. A 0o755 ancestor lets
            // any user list how many sessions ran. pi sets 0o700 on its root for
            // this reason, so the walk goes up while the name still looks like ours.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut dir = Some(parent);
                while let Some(current) = dir {
                    let ours = current
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name == "tasks"
                                || name.starts_with("rho-transcripts")
                                || name.chars().all(|c| c.is_ascii_digit())
                        });
                    if !ours {
                        break;
                    }
                    let _ =
                        std::fs::set_permissions(current, std::fs::Permissions::from_mode(0o700));
                    dir = current.parent();
                }
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            file: Mutex::new(file),
        })
    }

    /// Append one entry as a JSON line.
    pub async fn write(&self, entry: &TranscriptEntry) -> std::io::Result<()> {
        let line = serde_json::to_string(entry)?;
        let mut file = self.file.lock().await;
        writeln!(file, "{}", line)?;
        file.flush()?;
        Ok(())
    }

    /// The file's path. The parent can read it.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Construct the standard transcript path. On Unix, this is `/tmp/rho-transcripts-{pid}/{agent-id}.jsonl`.
pub fn transcript_path(agent_id: impl std::fmt::Display) -> PathBuf {
    session_transcript_dir(std::process::id().to_string()).join(format!("{agent_id}.jsonl"))
}

/// The directory that holds one session's child transcripts.
///
/// It is under the system temp directory, not under the session root. A transcript
/// written into the repository pollutes the user's project and can be committed by
/// accident, and rho's `.gitignore` does not cover it.
///
/// The layout follows pi: a per-user root, then a per-session directory. Two sessions
/// in one process must not share a directory, or one session's list of children is
/// visible to the other.
pub fn session_transcript_dir(session: impl std::fmt::Display) -> PathBuf {
    let mut root = std::env::temp_dir();
    #[cfg(unix)]
    {
        // A per-user root, so two users on one machine never share a directory.
        root = root.join(format!("rho-transcripts-{}", unsafe { libc_getuid() }));
    }
    #[cfg(not(unix))]
    {
        root = root.join("rho-transcripts");
    }
    root.join(session.to_string()).join("tasks")
}

/// The current user id. `getuid` cannot fail.
#[cfg(unix)]
unsafe fn libc_getuid() -> u32 {
    // `std` exposes no uid, and a whole crate for one number is not worth it. The
    // call is infallible and has no side effect.
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_uses_temp_dir() {
        let path = transcript_path("test");
        assert!(path.to_string_lossy().contains("rho-transcripts"));
    }

    #[tokio::test]
    async fn write_appends_jsonl_lines() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.jsonl");
        let writer = TranscriptWriter::new(&path).unwrap();

        let entry = TranscriptEntry::now("scout", TranscriptBody::TurnStart { turn: 1 });
        writer.write(&entry).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("turn_start") || content.contains("TurnStart"));
        assert!(content.contains("\"turn\":1"));
        assert!(
            content.contains("\"agent\":\"scout\""),
            "every line names its agent"
        );
        assert!(
            content.contains("\"ts\":"),
            "every line carries a timestamp"
        );
    }

    #[tokio::test]
    async fn permissions_are_0o600_on_unix() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.jsonl");
        let _writer = TranscriptWriter::new(&path).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }
    }
}
