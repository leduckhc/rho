# D-a-config-file-is-read-under-a-cap — the read is bounded, not the value

Date: 20260831. Reference: `D-a-config-file-is-read-under-a-cap`.
Spec: `docs/specs/20260818-014343-SPEC-config.md`, section 6 and section 7.

## The question

`Config::read_file` read a whole file into memory:

```rust
let contents = match std::fs::read_to_string(path) { .. };
```

A project file arrives with a clone that the user did not read, so its size is chosen by
whoever wrote the repository. What bounds that read?

## The measurement

Nothing bounded it. A review drove it and measured peak resident memory:

| `.rho/config.toml` | peak RSS before | peak RSS after |
| --- | --- | --- |
| 400 MB | 428 MB | not run |
| 800 MB | 848 MB | 10 MB |

```sh
cd /tmp/bigcfg && /usr/bin/time -l rho run "hi" --provider bedrock --model x
```

Memory tracked the file, one byte for one byte. The read also happens **before** the trust
gate, because the gate reads the parsed file to know which keys to strip. So a clone can
choose rho's start-up memory, and `--trust-project` does not enter into it.

This crate already caps everything else: the credential helper at 64 KiB, the profile walk at
depth 16, the steering queue by bytes. The one uncapped read was the one an attacker picks.

## The decision

**A config file is read under a cap of 1 MiB, and the cap is on the read.**

```rust
const MAX_CONFIG_BYTES: usize = 1024 * 1024;

file.take(MAX_CONFIG_BYTES as u64 + 1).read_to_string(&mut contents)?;
if contents.len() > MAX_CONFIG_BYTES {
    return Err(ConfigError::TooLarge { path, limit: MAX_CONFIG_BYTES });
}
```

The read stops one byte over the cap. That extra byte is what tells a file at the cap from a
file over it, with no second syscall and no trust in a stated length. It is the same shape as
the credential cap in the same file.

`ConfigError::TooLarge` is a new variant and not a reuse of `Read`, so a caller can tell a
hostile file from a permission fault. That is the argument `BaseUrl` already won against
`Value`: a safety block and a typo are different answers.

## Why the cap is on the read and not on the value

A test that asserts only the refusal passes with the bound removed. This was measured, not
argued: with `take` deleted and the length check kept,
`a_file_over_the_cap_is_refused_and_names_the_limit` and `a_file_at_the_cap_is_accepted` both
stayed green, and only `a_source_with_no_end_is_refused_and_the_read_ends` failed.

That is the `bash` line cap defect exactly: the test measured the output that was kept while
the buffer grew without limit. See `D-bash-line-cap`. A cap needs a test that watches the
read, and a file cannot be that test, because a test file is as small as the cap. An endless
source can.

## What it rules out

- No cap that refuses after reading. A refusal that arrives after the allocation is not a bound.
- No clamp. A file over the cap is refused, per `D-a-limit-too-large-is-refused-not-clamped`.
- No cap test that uses only a file. One test must drive a source with no end.
- No silent truncation. rho never reads the first 1 MiB of a large file and parses that.

## Test cases

- `a_file_over_the_cap_is_refused_and_names_the_limit`
- `a_file_at_the_cap_is_accepted`
- `a_source_with_no_end_is_refused_and_the_read_ends`
