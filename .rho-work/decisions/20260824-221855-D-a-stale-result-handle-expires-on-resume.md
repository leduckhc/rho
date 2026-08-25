# D-a-stale-result-handle-expires-on-resume — old evidence expires, and says so

**Question:** a resumed context holds text like `<tool_result_preview handle="r-0001">`.
The store behind that handle died with the earlier run. What does the model see?

## The decision

On a resume, rho rewrites each stale preview in the **rebuilt context**. The new text says
the evidence expired, and it keeps the byte count. The model then re-runs the command
instead of calling a handle that fails.

The file on disk does not change. Only the rebuilt context does.

## Rules that hold

- `D-tool-result-handle` stands. A resumed session reads no earlier run's evidence, and
  the per-session nonce still enforces that.
- The rewrite is visible to the model, so it is never a silent hole.
- The preview head that the record already holds stays. Only the handle promise goes.
- A test asserts that no live handle survives a resume.

## Rules out

**Keeping the result store alive across a resume.** The nonce would have to go on disk,
which weakens the boundary `D-tool-result-handle` set on purpose.

**Leaving the handle in place.** The model would call `read_tool_result`, get an error, and
spend a turn learning that the evidence is gone.

**Editing the file.** The file is append-only. That is `D-append-only-jsonl`.

## Cost

One rewrite pass over the rebuilt context, on the resume path only.
