# bench/jsonl-codec

This crate answers one question: which JSON library should the rho session log
use? See `docs/adr/ADR-005-jsonl-codec.md` for the decision and the numbers.

The crate is **not** a workspace member. It depends on `sonic-rs` and on
`simd-json`, and the workspace must not carry either crate for a benchmark. Its
`Cargo.toml` holds an empty `[workspace]` table, which keeps it standalone.

## Run it

Give it a real JSONL session file. A pi session file works, and so does a rho
session file.

```sh
cd bench/jsonl-codec
cargo run --release -- ~/.pi/agent/sessions/<project>/<stamp>_<uuid>.jsonl
```

It reports two corpora:

- **Corpus A** is the file you gave it, mapped onto the rho record shape. It has
  long text fields, like a real conversation.
- **Corpus B** is 50000 short synthetic records, in the tool-event shape.

Each line reports the best time of 30 rounds, and the throughput. It measures the
typed decode path, the typed encode path, and the untyped value path.

## Read the output with care

- The append path in rho is dominated by the write, not by the codec. A faster
  codec does not make `store` faster.
- The untyped path is the provider SSE reader. That path is the hottest, and it is
  where the numbers differ most.
- A number from this bench belongs in `docs/benchmarks.md` only with the platform,
  the date, and the command.
