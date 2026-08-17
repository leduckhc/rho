# Verification — bash OS sandbox (SPEC-10 / F-31)

Platform tested: macOS 25.5.0, arm64 (Darwin Kernel 25.5.0). Backend:
`/usr/bin/sandbox-exec`. The Linux `bwrap` path is not verified here, because
`bwrap` does not run on macOS.

## Live boundary checks

Run:

```sh
cargo test -p rho-tools --test sandbox
```

Result: 7 passed. The tests drive the real `bash` tool under `sandbox-exec`:

- `sandbox_off_runs_a_command_unchanged` — `off` runs a command, no wrapper.
- `confined_allows_a_write_inside_the_session_root` — a write under the root works.
- `confined_refuses_a_write_outside_the_session_root` — a write to `$HOME` is
  refused; the file does not exist.
- `confined_refuses_a_write_even_after_cd` — `cd $HOME && touch ...` is refused.
- `confined_refuses_an_absolute_path_write` — an absolute path outside the root is
  refused.
- `confined_still_allows_reading_a_system_file` — a read of `/etc/hosts` works.
- `strict_refuses_a_network_call` — a connection to a local listener works under
  `confined` and fails under `strict`.

## Break-and-verify (AGENTS.md step 7)

The good file was copied to `/tmp/sandbox.rs.good` first. Never restored with git.

| Break | Tests that failed |
| --- | --- |
| `None` backend returns `plain` instead of `Err` | `confinement_fails_closed_when_no_sandbox_is_available`, `a_sandbox_failure_message_names_the_mode_and_what_to_do` |
| Profile omits `(deny file-write*)` | `confined_refuses_a_write_outside_the_session_root`, `confined_refuses_an_absolute_path_write`, `confined_refuses_a_write_even_after_cd` |
| `strict` omits `(deny network*)` | `strict_refuses_a_network_call` |

Note: the `cd` test first targeted `/`, which is not user-writable, so a broken
profile still left the file absent. The test was changed to write under `$HOME`,
which is writable, so only the sandbox can stop the write. After the change, the
broken profile fails the test.

## Measured overhead

Command (100 runs of `sh -c 'exit 0'` each, with and without the generated
profile):

```sh
root=$(mktemp -d)
prof="(version 1)
(allow default)
(deny file-write*)
(allow file-write*
    (subpath \"$(cd "$root" && pwd -P)\")
    (subpath \"/dev\"))"
# time 100x /bin/sh -c 'exit 0', then 100x sandbox-exec -p "$prof" /bin/sh -c 'exit 0'
```

| Path | Average per call |
| --- | --- |
| plain `sh -c` | 2.74 ms |
| `sandbox-exec -p ... sh -c` | 7.52 ms |
| **overhead** | **4.77 ms** |

So `sandbox-exec` adds about 4.8 ms per `bash` call on this machine. That is small
against a build or a test run, but real. It is why `off` stays the default until a
later measurement on more machines.

## What this does not stop

- `confined` allows reads. Exfiltration through a read plus a network call is still
  possible in `confined`; use `strict` for the network half.
- `strict` denies the network, not the read. A `strict` command can still write a
  secret into the session root.
- The sandbox does not replace the approval policy. `--read-only` still denies
  `bash` outright.
- `sandbox-exec` is deprecated by Apple. The macOS path is behind one function.
- The Linux `bwrap` path is unverified in this environment.
