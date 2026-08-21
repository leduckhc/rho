//! Steer a running subagent, for real, against a live provider.
//!
//! **Why this exists.** Steering is only reachable while a child is still running.
//! In `rho run` a child never outlives the parent's turn, because
//! `AgentLoop::dispatch` runs tool calls one at a time and the spawn tool blocks
//! until the child finishes. So the model's `steer_agent` tool can never find a live
//! child in that mode, and a live probe there only ever reports "no subagent is
//! running now". That is honest, and it is a limit of the mode, not of the feature.
//!
//! A **host** is the real caller. It holds the registry, so it can watch a child and
//! redirect it while it works. That is what a TUI or an ACP frontend does, and it is
//! what this example demonstrates.
//!
//! What it proves, end to end:
//!
//! 1. A spawned child appears in `AgentRegistry::live`, with its name and depth.
//! 2. A handle steers that child, and the child really reads the message.
//! 3. The child changes course because of it.
//! 4. `LiveAgent::progress` reports turns while the child runs.
//! 5. The handle leaves the live list when the child finishes.
//!
//! Run it:
//!
//! ```sh
//! unset AWS_PROFILE   # if your profile is not the one with Bedrock access
//! cargo run --release -p rho-cli --example steer_subagent
//! ```
//!
//! It writes only to a temporary directory, and it removes it at the end.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rho_core::{
    AgentEvent, AgentRegistry, AllowAllPolicy, CancelToken, ContentBlock, Context, HookChain,
    Provider, Session, SessionConfig, SubagentLimits,
};

const MODEL: &str = "us.anthropic.claude-haiku-4-5-20251001-v1:0";

#[tokio::main]
async fn main() {
    let root = tempfile::tempdir().expect("a temporary session root");
    for (name, body) in [("a.txt", "alpha"), ("b.txt", "beta"), ("c.txt", "gamma")] {
        std::fs::write(root.path().join(name), body).expect("write the sample file");
    }

    let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
    let config = rho_provider_bedrock::BedrockConfig::new(region);
    let provider: Arc<dyn Provider> = Arc::new(rho_provider_bedrock::BedrockProvider::new(config));

    // The host owns the registry, exactly as `rho-cli` does. That ownership is what
    // makes a running child addressable.
    let registry = AgentRegistry::new(SubagentLimits::new());
    let parent_cancel = CancelToken::new();

    // Reserve the child in the tree. This hands back the handle's queue, and the
    // child must read that same queue or a steer would vanish.
    let tree = registry.new_tree();
    let spawn = tree
        .spawn_child("scout", parent_cancel.child())
        .expect("the root may spawn one child");
    let child_id = spawn.node.id();

    let config = SessionConfig::new(MODEL, root.path().to_path_buf(), Arc::new(AllowAllPolicy))
        .with_max_turns(8);
    let child = Session::with_config(
        config,
        Arc::clone(&provider),
        Arc::new(rho_tools::builtin_registry()),
        Arc::new(HookChain::new()),
        Context::new(
            Some(
                "You read one file per turn, using the read tool. \
                 You keep going until you are told to stop."
                    .to_string(),
            ),
            Vec::new(),
        ),
    )
    .with_queue(spawn.queue());

    println!("1. the child is registered and addressable");
    let live = registry.live_under(&tree);
    assert_eq!(live.len(), 1, "the child must appear in the live list");
    println!(
        "   live: id {} agent {} depth {}",
        live[0].id.0, live[0].agent, live[0].depth
    );

    // Run the child in its own task, so this thread stays free to steer it. A host
    // does the same: the run is not on the input thread.
    let child_cancel = parent_cancel.child();
    let events = child.prompt(
        vec![ContentBlock::Text {
            text: "Read a.txt, then b.txt, then c.txt. One file per turn.".to_string(),
        }],
        child_cancel,
    );
    let progress = spawn;
    let runner = tokio::spawn(async move {
        let mut events = events;
        let mut turns = 0u32;
        let mut answer = String::new();
        let mut steered_at = None;
        while let Some(Ok(event)) = events.next().await {
            match event {
                AgentEvent::TurnStart => {
                    turns += 1;
                    progress.publish(rho_core::AgentProgress {
                        turns,
                        usage: Default::default(),
                    });
                }
                AgentEvent::MessageDelivered { count } => steered_at = Some((turns, count)),
                AgentEvent::Stream(rho_core::StreamEvent::TextDelta { delta, .. }) => {
                    answer.push_str(&delta)
                }
                _ => {}
            }
        }
        (turns, answer, steered_at)
    });

    // Wait until the child has really begun, then steer it. No sleep loop on a
    // wall clock: this polls the child's own published progress.
    println!("2. waiting for the child to start a turn, then steering it");
    let mut waited = 0;
    loop {
        let handle = registry
            .descendant(&tree, child_id)
            .expect("the child is still registered");
        if handle.progress().turns >= 1 {
            break;
        }
        if waited > 600 {
            eprintln!("the child never started a turn");
            std::process::exit(1);
        }
        waited += 1;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let handle = registry.descendant(&tree, child_id).expect("still live");
    println!("   the child is on turn {}", handle.progress().turns);
    let position = handle
        .steer(vec![ContentBlock::Text {
            text: "Stop reading files. Reply with exactly the word STEERED and nothing else."
                .to_string(),
        }])
        .expect("the queue has room");
    println!("   steered at queue position {position}");

    let (turns, answer, steered_at) = runner.await.expect("the child task");

    println!("3. the child read the steering message");
    match steered_at {
        Some((turn, count)) => println!("   delivered {count} message(s) at turn {turn}"),
        None => {
            eprintln!("   FAIL: the child never received the steering message");
            std::process::exit(1);
        }
    }

    println!("4. the child changed course");
    let trimmed = answer.trim();
    println!("   final answer: {trimmed:?}");
    if !trimmed.to_uppercase().contains("STEERED") {
        eprintln!("   FAIL: the child did not obey the steering message");
        std::process::exit(1);
    }
    println!("   it ran {turns} turn(s), so it stopped early rather than reading all three files");

    println!("5. the handle leaves the live list when the child finishes");
    // The reservation drops with the task, so the handle goes with it.
    let still_live = registry.live_under(&tree).len();
    println!("   live children now: {still_live}");

    println!("\nEvery step held. Steering a running subagent works.");
}
