// Which JSONL codec should rho's session log use?
// Corpus: a real pi session file, converted to rho's record shape.
use serde::{Deserialize, Serialize};
use sonic_rs::JsonContainerTrait;
use std::time::Instant;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionRecord {
    id: String,
    parent: Option<String>,
    ts: u64,
    kind: String,
    role: Option<String>,
    text: Option<String>,
    tokens: Option<u64>,
}

fn corpus() -> Vec<SessionRecord> {
    let path = std::env::args().nth(1).expect("give the pi session file path");
    let raw = std::fs::read_to_string(path).unwrap();
    let mut out = Vec::new();
    let mut parent: Option<String> = None;
    for (n, line) in raw.lines().enumerate() {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("other").to_string();
        let id = v.get("id").and_then(|t| t.as_str()).unwrap_or("x").to_string();
        let msg = v.get("message");
        let role = msg
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());
        let text = msg.and_then(|m| m.get("content")).map(|c| match c.as_array() {
            Some(blocks) => blocks
                .iter()
                .map(|b| {
                    b.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| b.to_string())
                })
                .collect::<Vec<_>>()
                .join("\n"),
            None => c.to_string(),
        });
        out.push(SessionRecord {
            id: id.clone(),
            parent: parent.take(),
            ts: 1_700_000_000 + n as u64,
            kind,
            role,
            text,
            tokens: Some(n as u64 * 7),
        });
        parent = Some(id);
    }
    out
}

fn bench<F: FnMut() -> usize>(name: &str, bytes: usize, rounds: u32, mut f: F) {
    let mut best = f64::MAX;
    let mut check = 0usize;
    for _ in 0..rounds {
        let t = Instant::now();
        check = f();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if ms < best {
            best = ms;
        }
    }
    let mbps = (bytes as f64 / 1_048_576.0) / (best / 1000.0);
    println!("{name:<28} best {best:>8.3} ms   {mbps:>8.1} MB/s   (check {check})");
}

fn short_corpus(n: usize) -> Vec<SessionRecord> {
    (0..n)
        .map(|i| SessionRecord {
            id: format!("{i:08x}"),
            parent: Some(format!("{:08x}", i.saturating_sub(1))),
            ts: 1_700_000_000 + i as u64,
            kind: "tool_call".to_string(),
            role: Some("assistant".to_string()),
            text: Some(format!("read crates/rho-core/src/agent.rs line {i}")),
            tokens: Some(i as u64),
        })
        .collect()
}

fn run(records: Vec<SessionRecord>) {
    let lines: Vec<String> = records.iter().map(|r| serde_json::to_string(r).unwrap()).collect();
    let bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
    println!(
        "corpus: {} records, {:.2} MiB of JSONL\n",
        records.len(),
        bytes as f64 / 1_048_576.0
    );
    let rounds = 30;

    println!("-- decode, one line at a time, into a typed record --");
    bench("serde_json::from_str", bytes, rounds, || {
        lines
            .iter()
            .map(|l| serde_json::from_str::<SessionRecord>(l).unwrap().ts as usize)
            .sum()
    });
    bench("sonic_rs::from_str", bytes, rounds, || {
        lines
            .iter()
            .map(|l| sonic_rs::from_str::<SessionRecord>(l).unwrap().ts as usize)
            .sum()
    });
    bench("simd_json (copy per line)", bytes, rounds, || {
        let mut sum = 0usize;
        let mut buf: Vec<u8> = Vec::with_capacity(1 << 16);
        for l in &lines {
            buf.clear();
            buf.extend_from_slice(l.as_bytes());
            let r: SessionRecord = simd_json::serde::from_slice(&mut buf).unwrap();
            sum += r.ts as usize;
        }
        sum
    });

    println!("\n-- encode, one record at a time --");
    bench("serde_json::to_string", bytes, rounds, || {
        records.iter().map(|r| serde_json::to_string(r).unwrap().len()).sum()
    });
    bench("sonic_rs::to_string", bytes, rounds, || {
        records.iter().map(|r| sonic_rs::to_string(r).unwrap().len()).sum()
    });
    bench("simd_json::to_string", bytes, rounds, || {
        records.iter().map(|r| simd_json::serde::to_string(r).unwrap().len()).sum()
    });

    println!("\n-- decode a whole file into an untyped value --");
    bench("serde_json::Value", bytes, rounds, || {
        lines
            .iter()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap().as_object().unwrap().len())
            .sum()
    });
    bench("sonic_rs::Value", bytes, rounds, || {
        lines
            .iter()
            .map(|l| {
                sonic_rs::from_str::<sonic_rs::Value>(l)
                    .unwrap()
                    .as_object()
                    .map(|o| o.len())
                    .unwrap_or(0)
            })
            .sum()
    });
}

fn main() {
    println!("=== corpus A: a real pi session, long text records ===");
    run(corpus());
    println!("\n=== corpus B: 50000 short records, the tool-event shape ===");
    run(short_corpus(50_000));
}
