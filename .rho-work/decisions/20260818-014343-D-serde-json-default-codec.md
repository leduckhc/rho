# D-serde-json-default-codec — serde_json is the default codec, and sonic-rs is an off-by-default feature


**Question (T1 architect):** which JSON codec does the session file use?

**Decision:** `serde_json` is the default. The `fast-json` cargo feature selects
`sonic-rs`. `simd-json` is rejected. The codec is one module with a generic `encode` and
`decode`, so the feature switch changes no caller and no record type. The numbers and
the command are in `ADR-jsonl-codec`.

**Reason:** `sonic-rs` wins on encode and on a large untyped decode, but it costs a 23
percent larger binary, a dependency tree of 102 lines against 21, and a slow fallback
off `x86_64` and `aarch64`. `simd-json` lost on small records. So the fast codec is a
feature, not the default.

**Rules out:** a codec-specific attribute on a record type, because it would break the
other codec. Making `sonic-rs` the default and paying its cost for every build.
