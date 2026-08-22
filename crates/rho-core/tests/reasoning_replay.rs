//! The rules that govern a replay payload: the log rule, the bound, and the report.
//!
//! See `SPEC-reasoning-across-providers` section 4 and rules 8 to 11, and decision
//! `D-reasoning-replay-is-opaque-provider-state`.

use std::sync::{Arc, Mutex};

use rho_core::{ContentBlock, ProviderState, ReasoningOwner, SessionLog};

/// A writer that collects every log byte into a shared buffer.
#[derive(Clone)]
struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
    type Writer = BufferWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn state(value: serde_json::Value) -> ProviderState {
    ProviderState {
        owner: ReasoningOwner {
            provider: "bedrock".to_string(),
            model: "claude".to_string(),
        },
        value,
    }
}

/// Write one message through the recorder, and return every line of the session file.
///
/// The block travels the real write path: `redact_block`, then `cap_record`. It goes in as a
/// prompt, because that is the one entry point the recorder exposes for a message, and the
/// two functions under test do not care about the role.
///
/// **No production caller writes a session file yet.** See
/// `D-no-caller-writes-a-session-file`. These tests cover the format, and they claim nothing
/// about a lifecycle that has no caller.
fn recorded(block: ContentBlock) -> Vec<String> {
    recorded_in_dir(block).0
}

/// The same, and the directory, so a test can look for a spill sidecar.
fn recorded_in_dir(block: ContentBlock) -> (Vec<String>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = rho_core::SessionStore::new(dir.path());
    let writer = store
        .create("test-session", dir.path(), "allow-all", "off")
        .expect("the session file opens");
    let path = writer.path().to_path_buf();
    let mut recorder = rho_core::SessionRecorder::new(SessionLog::File(writer));
    recorder.record_prompt(&[block]);
    drop(recorder);
    let lines = std::fs::read_to_string(&path)
        .expect("the file reads")
        .lines()
        .map(str::to_string)
        .collect();
    (lines, dir)
}

/// Rule 9. The payload is exempt from redaction, so it must never reach a log instead.
#[test]
fn a_state_value_never_reaches_a_log() {
    let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufferWriter(Arc::clone(&buffer)))
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let block = ContentBlock::ReasoningReplay {
            text: "a plan".to_string(),
            state: Some(state(serde_json::json!({ "signature": "sig-do-not-log" }))),
        };
        let lines = recorded(block);
        assert!(
            lines.iter().any(|line| line.contains("sig-do-not-log")),
            "the payload is stored verbatim, because a rewritten payload cannot replay"
        );
        // Prove the capture works before trusting an empty result. See
        // `D-log-capture-proves-itself`.
        tracing::warn!("the capture is live");
    });
    let logged = String::from_utf8(buffer.lock().unwrap().clone()).expect("valid utf8");
    assert!(logged.contains("the capture is live"), "the capture works");
    assert!(
        !logged.contains("sig-do-not-log"),
        "no payload in a log, at any level: {logged}"
    );
}

/// Rule 10. A payload over the record cap is dropped whole, and the drop is reported.
#[test]
fn a_value_over_the_record_cap_is_dropped_and_reported() {
    let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufferWriter(Arc::clone(&buffer)))
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let joined = tracing::subscriber::with_default(subscriber, || {
        let huge = "x".repeat(rho_core::MAX_RECORD_BYTES + 1);
        let block = ContentBlock::ReasoningReplay {
            text: "a plan".to_string(),
            state: Some(state(serde_json::json!({ "signature": huge }))),
        };
        recorded(block).join("\n")
    });
    assert!(
        !joined.contains(&"x".repeat(1000)),
        "the oversize payload is not written"
    );
    assert!(
        joined.contains("a plan"),
        "the readable text survives the drop"
    );
    let logged = String::from_utf8(buffer.lock().unwrap().clone()).expect("valid utf8");
    // The report names the size and the provider, and never the value.
    assert!(
        logged.contains("exceeded the record cap") && logged.contains("bedrock"),
        "the drop is reported: {logged}"
    );
    assert!(
        !logged.contains(&"x".repeat(1000)),
        "the report never carries the payload"
    );
}

/// A payload under the cap is written unchanged, or a replay after a resume would fail.
#[test]
fn a_payload_under_the_cap_is_written_verbatim() {
    let block = ContentBlock::ReasoningReplay {
        text: "a plan".to_string(),
        state: Some(state(serde_json::json!({ "signature": "keep-me" }))),
    };
    let joined = recorded(block).join("\n");
    assert!(joined.contains("keep-me"));
    assert!(joined.contains("\"replay\":true"));
}

/// Rule 11. A record that claims a replay and carries no payload loads as history, and
/// rho says so, because a provider bug must not hide.
#[test]
fn a_replay_record_with_no_state_is_reported() {
    let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufferWriter(Arc::clone(&buffer)))
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let block: ContentBlock = tracing::subscriber::with_default(subscriber, || {
        serde_json::from_value(serde_json::json!({
            "type": "thinking", "thinking": "x", "replay": true
        }))
        .expect("the record loads")
    });
    assert_eq!(
        block,
        ContentBlock::ReasoningTrace {
            text: "x".to_string()
        }
    );
    let logged = String::from_utf8(buffer.lock().unwrap().clone()).expect("valid utf8");
    assert!(
        logged.contains("no payload"),
        "the load reports the claim it could not honour: {logged}"
    );
}

/// A reasoning text is capped like any assistant text, and the body really spills.
///
/// The first version of this test asserted only that the record was smaller than twice the
/// cap. A mutation review named it: a bug that dropped the text entirely, or truncated it
/// instead of spilling it, would have passed. That is the same shape as the memory-cap defect
/// this project already shipped once.
#[test]
fn a_reasoning_text_over_the_string_cap_spills() {
    let long = "y".repeat(rho_core::MAX_RECORD_BYTES);
    let (lines, dir) = recorded_in_dir(ContentBlock::ReasoningTrace { text: long.clone() });
    let joined = lines.join("\n");
    assert!(
        joined.len() < rho_core::MAX_RECORD_BYTES,
        "the record itself is bounded"
    );
    assert!(
        !joined.contains(&long),
        "the whole body is not in the record"
    );
    assert!(
        joined.contains("yyyy"),
        "a head of the text stays in the record: {joined}"
    );
    // The body is not lost. It spills to a sidecar beside the session file.
    let sidecars: Vec<String> = std::fs::read_dir(dir.path())
        .expect("the directory reads")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_none_or(|ext| ext != "jsonl"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect();
    assert!(
        sidecars.iter().any(|body| body.contains(&long)),
        "the spilled body is kept beside the session file"
    );
}

/// A payload of exactly the cap is kept. One byte more is dropped.
///
/// A mutation review found the boundary untested, so `<=` could become `<` unseen.
#[test]
fn the_payload_cap_is_inclusive() {
    // The encoded value carries `{"signature":"..."}`, so the padding is computed from that.
    let padding = r#"{"signature":""}"#.len();
    let exact = "z".repeat(rho_core::MAX_RECORD_BYTES - padding);
    let joined = recorded(ContentBlock::ReasoningReplay {
        text: "a plan".to_string(),
        state: Some(state(serde_json::json!({ "signature": exact.clone() }))),
    })
    .join("\n");
    assert!(
        joined.contains(&exact),
        "a payload of exactly the cap is kept"
    );
}

/// Is this line the start of a catch-all match arm?
///
/// A catch-all is `_` or a bare binding such as `other`. Both take every case that the
/// named arms above them did not, which is how a new block kind vanishes in silence.
///
/// The first version of this check looked for the literal `_ => {}`. A review pointed out
/// that `other => ()` walks straight past it, and a guard that catches one spelling teaches
/// the next author to use another.
fn is_catch_all_arm(line: &str) -> bool {
    let Some((pattern, _)) = line.trim().split_once("=>") else {
        return false;
    };
    let pattern = pattern.trim();
    if pattern == "_" {
        return true;
    }
    // A bare binding: one lowercase identifier, with no path, no struct pattern, and no
    // guard. `ContentBlock::Text { text }` and `}` are both excluded by that.
    !pattern.is_empty()
        && pattern
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Each `match block {` region of a source file, up to the arm that closes it.
///
/// Crude on purpose. It reads the text, because the alternative is a parser, and the guard
/// only needs to see whether a wildcard sits beside the named arms.
fn match_block_regions(source: &str) -> Vec<String> {
    let mut regions = Vec::new();
    for (start, _) in source.match_indices("match block {") {
        let rest = &source[start..];
        // The region ends at the first line that closes the match at its own indentation.
        let end = rest
            .find("\n        }")
            .or_else(|| rest.find("\n    }"))
            .unwrap_or(rest.len());
        // Comments are dropped first. Every named arm here carries a comment that explains
        // the drop, and one of them writes `_ => {}` to name what it avoids. The first
        // version of this guard read that prose as code and failed the build for it.
        let code: String = rest[..end]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        regions.push(code);
    }
    regions
}

/// Every request builder names every content block. A stream parser may keep a wildcard,
/// because a wire event set is open, but a request builder must not: that is how a reasoning
/// block was dropped in silence, and how Azure kept dropping one for a whole sprint.
#[test]
fn every_request_builder_has_an_explicit_arm() {
    for path in [
        "../rho-provider-bedrock/src/lib.rs",
        "../rho-provider-openrouter/src/lib.rs",
        "../rho-provider-azure/src/lib.rs",
    ] {
        let source = std::fs::read_to_string(path).expect("the provider source reads");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("a source file has a first part");
        for block in [
            "ContentBlock::ReasoningTrace",
            "ContentBlock::ReasoningReplay",
            "ContentBlock::Image",
        ] {
            assert!(
                production.contains(block),
                "{path} must name {block} in an arm of its own"
            );
        }
        // Naming the blocks is not enough. A wildcard beside them takes the traffic and
        // the named arms become decoration. A deliberate break proved that: it added
        // `_ => {}` and kept every name, and the first version of this guard passed. So
        // the match over a content block is now read arm by arm.
        for region in match_block_regions(production) {
            // Every spelling of a catch-all, not one. A review pointed out that the first
            // version caught `_ => {}` and missed `other => ()`, `_ =>` with any body, and
            // a binding arm. A guard that catches one spelling teaches the next author to
            // use another.
            for line in region.lines() {
                assert!(
                    !is_catch_all_arm(line),
                    "{path} has a catch-all arm in a match over a content block: {}",
                    line.trim()
                );
            }
        }
    }
}

/// A payload from a file rho did not write is bounded on the way **in**, not only on the way
/// out.
///
/// A security review found the bound was write-only. A session file is untrusted input: it
/// can be copied between machines, and rho may not have written it. Without a read-side
/// bound, a foreign file could carry a payload up to the line cap and rho would replay it on
/// every later turn.
#[test]
fn an_oversize_payload_in_a_file_is_dropped_on_read() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("hostile.jsonl");
    let huge = "x".repeat(rho_core::MAX_RECORD_BYTES * 2);
    let header = serde_json::json!({
        "id": "r0", "parentId": null, "timestamp": "1700000000000", "type": "session",
        "version": 1, "cwd": "/tmp", "approval": "allow-all", "sandbox": "off"
    });
    let record = serde_json::json!({
        "id": "r1", "parentId": "r0", "timestamp": "1700000000000", "type": "message",
        "message": { "role": "assistant", "content": [{
            "type": "thinking", "thinking": "a plan", "replay": true,
            "state": {
                "owner": { "provider": "bedrock", "model": "claude" },
                "value": { "signature": huge }
            }
        }]}
    });
    std::fs::write(&path, format!("{header}\n{record}\n")).expect("write the file");

    let result = rho_core::SessionReader::read(&path).expect("the file loads");
    let payloads: Vec<_> = result
        .entries
        .iter()
        .filter_map(|entry| match &entry.record {
            rho_core::Record::Message { message } => Some(message.content.iter()),
            _ => None,
        })
        .flatten()
        .filter_map(|block| match block {
            ContentBlock::ReasoningReplay { state, .. } => state.clone(),
            _ => None,
        })
        .collect();
    assert!(
        payloads.is_empty(),
        "an oversize payload must not survive the read"
    );
}

/// A bad record in the middle of a file is skipped and counted, and every later record
/// survives.
///
/// A security review found that any bad line stopped the read, so one flipped byte silently
/// discarded the rest of the session and reported it as a truncated tail.
#[test]
fn a_bad_middle_record_does_not_discard_the_rest() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("corrupt.jsonl");
    let header = serde_json::json!({
        "id": "r0", "parentId": null, "timestamp": "1700000000000", "type": "session",
        "version": 1, "cwd": "/tmp", "approval": "allow-all", "sandbox": "off"
    });
    let good = |id: &str, text: &str| {
        serde_json::json!({
            "id": id, "parentId": "r0", "timestamp": "1700000000000", "type": "message",
            "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] }
        })
        .to_string()
    };
    std::fs::write(
        &path,
        format!(
            "{header}\n{}\n{{not json at all\n{}\n",
            good("r1", "first"),
            good("r2", "second")
        ),
    )
    .expect("write the file");

    let result = rho_core::SessionReader::read(&path).expect("the file loads");
    assert_eq!(
        result.entries.len(),
        2,
        "the record after the corruption survives"
    );
    assert_eq!(result.dropped_records, 1, "the drop is counted");
    assert!(
        !result.truncated_tail,
        "corruption in the middle is not a truncated tail"
    );
}
