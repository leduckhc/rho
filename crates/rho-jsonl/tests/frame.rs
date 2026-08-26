//! Framing tests. See SPEC-jsonl-frontend section 4.
//!
//! No test uses the network. No test uses `sleep`. No test touches the real
//! filesystem, so no test needs `tempfile` here.

use rho_jsonl::{Line, LineReader};

use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, ReadBuf};

/// Read from a byte slice, and report every record.
async fn records(input: &str, cap: usize) -> Vec<Line> {
    let mut reader = LineReader::with_cap(input.as_bytes(), cap);
    let mut out = Vec::new();
    while let Some(line) = reader.next_line().await.expect("the reader must not fail") {
        out.push(line);
    }
    out
}

fn text(line: &Line) -> String {
    match line {
        Line::Record(bytes) => String::from_utf8(bytes.clone()).expect("valid utf8"),
        Line::TooLong { bytes } => panic!("expected a record, got TooLong after {bytes} bytes"),
    }
}

#[tokio::test]
async fn crlf_line_parses() {
    // A client on Windows, or a client using a naive writer, ends a line with CRLF.
    // The CR must not reach the JSON parser, because `{"a":1}\r` is not valid JSON.
    let lines = records("{\"type\":\"abort\"}\r\n{\"type\":\"get_state\"}\n", 1024).await;
    assert_eq!(lines.len(), 2, "CRLF must not split into two records");
    assert_eq!(text(&lines[0]), r#"{"type":"abort"}"#);
    assert_eq!(text(&lines[1]), r#"{"type":"get_state"}"#);
}

#[tokio::test]
async fn unicode_line_separator_inside_string() {
    // U+2028 is legal inside a JSON string. Node's readline splits on it, so a
    // client built on readline corrupts this line. rho must not.
    let one = "{\"type\":\"prompt\",\"message\":\"a\u{2028}b\u{2029}c\"}\n";
    let lines = records(one, 1024).await;
    assert_eq!(lines.len(), 1, "the reader must split on LF only");
    let parsed: serde_json::Value = serde_json::from_slice(match &lines[0] {
        Line::Record(bytes) => bytes,
        Line::TooLong { .. } => panic!("expected a record"),
    })
    .expect("the record must still be valid JSON");
    assert_eq!(parsed["message"], "a\u{2028}b\u{2029}c");
}

#[tokio::test]
async fn a_last_line_with_no_newline_is_a_record() {
    let lines = records("{\"type\":\"abort\"}", 1024).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(text(&lines[0]), r#"{"type":"abort"}"#);
}

#[tokio::test]
async fn an_empty_input_yields_no_record() {
    assert!(records("", 1024).await.is_empty());
}

#[tokio::test]
async fn an_over_long_line_is_refused_and_bounded() {
    // The cap is 32 bytes here, so the test proves the rule without allocating a
    // megabyte. The whole input arrives in one read buffer, so this also pins the
    // case where the newline is present but sits past the cap. The cap must still
    // clear the second line, or the test would pass for the wrong reason.
    let long = format!("{}\n{{\"type\":\"abort\"}}\n", "x".repeat(64));
    let lines = records(&long, 32).await;
    assert_eq!(lines.len(), 2, "the over-long line is one record, refused");
    assert_eq!(
        text(&lines[1]),
        r#"{"type":"abort"}"#,
        "the line after the refused one must be a whole record"
    );
    match &lines[0] {
        Line::TooLong { bytes } => {
            // Assert the bytes read, not the bytes kept. A memory-cap test here once
            // passed against the very bug it was written for, because it measured
            // what was kept while the buffer still grew. See D-bash-line-cap.
            assert_eq!(*bytes, 65, "TooLong must report every byte it consumed");
        }
        Line::Record(_) => panic!("a line past the cap must not arrive as a record"),
    }
}

#[tokio::test]
async fn the_reader_resumes_after_an_over_long_line() {
    let input = format!("{}\n{{\"type\":\"get_state\"}}\n", "x".repeat(64));
    let lines = records(&input, 32).await;
    assert!(matches!(lines[0], Line::TooLong { .. }));
    assert_eq!(
        text(&lines[1]),
        r#"{"type":"get_state"}"#,
        "the command after an over-long line must parse normally"
    );
}

/// A reader that serves `b'x'` for ever and never a newline. It counts what it
/// served, so a test can assert the bytes the line reader consumed.
struct Endless {
    served: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl AsyncRead for Endless {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let room = buf.remaining().min(4096);
        buf.initialize_unfilled_to(room);
        for slot in &mut buf.initialized_mut()[..room] {
            *slot = b'x';
        }
        buf.advance(room);
        self.served
            .fetch_add(room, std::sync::atomic::Ordering::SeqCst);
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn an_unterminated_line_cannot_grow_without_limit() {
    // A peer opens the pipe and never writes a newline. Without a cap the reader
    // grows until the host runs out of memory, and it never returns. This test
    // asserts the bytes the reader took from the stream, which is the only
    // measurement that can see the memory half of that defect.
    //
    // It needs no timeout. The rule under test is that `next_line` always returns,
    // so a reader that loops for ever fails this test by never finishing.
    let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let endless = Endless {
        served: std::sync::Arc::clone(&served),
    };
    // One `BufReader` refill is 8 KiB, so the reader always takes whole buffers.
    // The cap is a multiple of that, and the slack below is one buffer.
    let cap = 64 * 1024;
    const SLACK: usize = 8 * 1024;
    let mut reader = LineReader::with_cap(endless, cap);

    let first = reader
        .next_line()
        .await
        .expect("the reader must not fail")
        .expect("an endless stream is not end of file");
    match first {
        Line::TooLong { bytes } => assert!(
            bytes > cap && bytes <= cap + SLACK,
            "the reader consumed {bytes} bytes for a {cap} byte cap"
        ),
        Line::Record(_) => panic!("an unterminated line must never arrive as a record"),
    }

    // Every later call must also return, and each one is bounded the same way.
    for _ in 0..3 {
        let next = reader
            .next_line()
            .await
            .expect("the reader must not fail")
            .expect("the stream is endless");
        assert!(
            matches!(next, Line::TooLong { .. }),
            "an endless run with no newline stays refused"
        );
    }

    let total = served.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        total <= (cap + SLACK) * 4,
        "four calls read {total} bytes for a {cap} byte cap"
    );
}

#[tokio::test]
async fn a_blank_line_is_an_empty_record() {
    // A blank line is a record that fails to parse, not a record the reader hides.
    // Hiding it would make the reply count differ from the line count.
    let lines = records("\n{\"type\":\"abort\"}\n", 1024).await;
    assert_eq!(lines.len(), 2);
    assert_eq!(text(&lines[0]), "");
}

#[tokio::test]
async fn invalid_utf8_reaches_the_caller_as_bytes() {
    // The reader must not fail on invalid UTF-8. The caller turns it into a parse
    // error reply, and the session stays open.
    let mut reader = LineReader::with_cap(&b"{\"a\":\"\xff\"}\n"[..], 1024);
    let line = reader
        .next_line()
        .await
        .expect("invalid utf8 must not be a reader error")
        .expect("one record");
    match line {
        Line::Record(bytes) => {
            assert!(String::from_utf8(bytes.clone()).is_err());
            assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_err());
        }
        Line::TooLong { .. } => panic!("expected a record"),
    }
}

/// A reader that serves its chunks one at a time, and returns `Pending` between them.
///
/// It models a real pipe, where a command line can arrive in pieces.
struct Chunked {
    chunks: std::sync::Mutex<std::collections::VecDeque<Vec<u8>>>,
    /// True when the next poll must return `Pending` instead of a chunk.
    gap: std::sync::Mutex<bool>,
}

impl AsyncRead for Chunked {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut gap = self.gap.lock().expect("gap");
        if *gap {
            *gap = false;
            // Wake at once, so the next poll makes progress. The point is the Pending,
            // not a real delay.
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        drop(gap);
        let mut chunks = self.chunks.lock().expect("chunks");
        match chunks.pop_front() {
            Some(chunk) => {
                buf.put_slice(&chunk);
                // A gap follows every chunk, so a caller that wants the rest of the
                // line must poll again. That is what a real pipe does.
                *self.gap.lock().expect("gap") = true;
                Poll::Ready(Ok(()))
            }
            // No more chunks means end of file.
            None => Poll::Ready(Ok(())),
        }
    }
}

#[tokio::test]
async fn a_dropped_next_line_loses_no_bytes() {
    // `next_line` must be cancel safe. The serve loop races it against the run's event
    // stream in a `select!`, so the future is dropped whenever an event wins. If the
    // partial line lived in the future, those bytes would vanish and the command would
    // be lost. A live abort was swallowed exactly that way.
    let reader = Chunked {
        chunks: std::sync::Mutex::new(
            vec![b"{\"type\":\"ab".to_vec(), b"ort\"}\n".to_vec()]
                .into_iter()
                .collect(),
        ),
        gap: std::sync::Mutex::new(false),
    };
    let mut reader = LineReader::with_cap(reader, 1024);

    // Poll once. It consumes the first chunk, finds no newline, and returns Pending.
    {
        let mut future = Box::pin(reader.next_line());
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        // The first poll takes chunk one, then asks for more and gets the gap.
        assert!(
            std::future::Future::poll(future.as_mut(), &mut cx).is_pending(),
            "the first poll must not complete the line"
        );
        // Drop the future, exactly as `select!` does when the other branch wins.
    }

    // The bytes from the first chunk must still be in the reader.
    let line = reader
        .next_line()
        .await
        .expect("the reader must not fail")
        .expect("one record");
    assert_eq!(
        text(&line),
        r#"{"type":"abort"}"#,
        "a dropped read must lose no byte of the command"
    );
}
