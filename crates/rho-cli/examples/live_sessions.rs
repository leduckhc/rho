//! Measure many live sessions at once.
//!
//! This is the claim the project rests on, and until now only the idle case was measured.
//! `docs/benchmarks.md` reports 101 idle sessions in 10.75 MiB and about 25 KB per extra
//! session. An idle session holds a provider client, a tool registry, and a context. It
//! sends nothing.
//!
//! A live session is different. It holds a response stream, a decode buffer, and a
//! parsed event queue, and it does so while the model is generating. So the honest
//! question is what the marginal cost becomes when every session is actually working.
//!
//! **Why no competitor can publish this number.** jcode and pi both run one operating
//! system process per session, so their marginal cost includes a whole process. rho is a
//! library, so a host holds many sessions in one address space. That is not only a smaller
//! number, it is a different measurement.
//!
//! Run it:
//!
//! ```sh
//! export OPENROUTER_API_KEY=...
//! cargo build --release -p rho-cli --example live_sessions
//! RHO_SESSIONS=1  /usr/bin/time -l ./target/release/examples/live_sessions
//! RHO_SESSIONS=50 /usr/bin/time -l ./target/release/examples/live_sessions
//! ```
//!
//! Read `maximum resident set size` on macOS, which is bytes. On Linux read
//! `Maximum resident set size`, which is kilobytes.
//!
//! Every session sends a real prompt and drains a real streamed answer. Set
//! `RHO_MODEL` to change the model. The prompt is deliberately tiny, so the run measures
//! the harness rather than the model's output length.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use futures::StreamExt;
use rho_core::{
    AgentEvent, AllowAllPolicy, CancelToken, ContentBlock, Context, HookChain, Session,
    SessionConfig, StreamEvent, Usage,
};
use rho_provider_openrouter::{OpenRouterConfig, OpenRouterProvider, Secret};

#[tokio::main]
async fn main() {
    let count: usize = std::env::var("RHO_SESSIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let model =
        std::env::var("RHO_MODEL").unwrap_or_else(|_| "anthropic/claude-haiku-4.5".to_string());
    let key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            eprintln!("set OPENROUTER_API_KEY. This example makes real requests.");
            std::process::exit(1);
        }
    };

    let root = std::env::current_dir().expect("a working directory");
    let started = Instant::now();
    let ok = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicU64::new(0));
    let first_token_us = Arc::new(AtomicU64::new(0));
    let totals = Arc::new(std::sync::Mutex::new(Usage::default()));

    let mut handles = Vec::with_capacity(count);
    for index in 0..count {
        // Every session gets its own provider client, tool registry, hooks, and context,
        // because a real host does not share those between users.
        let provider = Arc::new(OpenRouterProvider::new(OpenRouterConfig::new(Secret::new(
            key.clone(),
        ))));
        let config = SessionConfig::new(&model, root.clone(), Arc::new(AllowAllPolicy));
        let session = Session::with_config(
            config,
            provider,
            Arc::new(rho_tools::builtin_registry()),
            Arc::new(HookChain::new()),
            Context::new(
                Some("You are rho. Answer in one short word.".to_string()),
                Vec::new(),
            ),
        );

        let ok = Arc::clone(&ok);
        let failed = Arc::clone(&failed);
        let first_token_us = Arc::clone(&first_token_us);
        let totals = Arc::clone(&totals);
        handles.push(tokio::spawn(async move {
            let turn_started = Instant::now();
            let mut seen_first = false;
            let mut events = session.prompt(
                vec![ContentBlock::Text {
                    text: format!("Reply with exactly the word: session{index}"),
                }],
                CancelToken::new(),
            );
            let mut had_error = false;
            while let Some(item) = events.next().await {
                match item {
                    Ok(AgentEvent::Stream(StreamEvent::TextDelta { .. })) => {
                        if !seen_first {
                            seen_first = true;
                            // Record the slowest first token, which is what a user waiting
                            // on the worst session actually feels.
                            let us = turn_started.elapsed().as_micros() as u64;
                            first_token_us.fetch_max(us, Ordering::SeqCst);
                        }
                    }
                    Ok(AgentEvent::Stream(StreamEvent::Usage(usage))) => {
                        totals.lock().expect("the lock holds").add(&usage);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        had_error = true;
                        eprintln!("session {index} failed: {error}");
                        break;
                    }
                }
            }
            if had_error {
                failed.fetch_add(1, Ordering::SeqCst);
            } else {
                ok.fetch_add(1, Ordering::SeqCst);
            }
            // Hold the session until the turn ends, so the measurement covers a live
            // session rather than a dropped one.
            std::hint::black_box(&session);
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }

    let elapsed = started.elapsed();
    let totals = totals.lock().expect("the lock holds");
    println!("sessions        {count}");
    println!("succeeded       {}", ok.load(Ordering::SeqCst));
    println!("failed          {}", failed.load(Ordering::SeqCst));
    println!("wall clock      {:.2} s", elapsed.as_secs_f64());
    println!(
        "slowest first token {:.0} ms",
        first_token_us.load(Ordering::SeqCst) as f64 / 1000.0
    );
    println!(
        "tokens          in {} out {} cache_read {}",
        totals.input_tokens, totals.output_tokens, totals.cache_read_tokens
    );
    match totals.cache_hit_ratio() {
        Some(ratio) => println!("cache hit rate  {:.1} %", ratio * 100.0),
        None => println!("cache hit rate  no data"),
    }
    match totals.cost_usd {
        Some(cost) => println!("cost            ${cost:.6} as charged by the provider"),
        None => println!("cost            not reported by this provider"),
    }
}
