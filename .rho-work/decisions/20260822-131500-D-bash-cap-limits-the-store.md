# D-bash-cap-limits-the-store — bash still loses its own tail, and this says so


**Question (controller, from a live run):** with a result store configured, how much of a
large `bash` output can the model reach?

**Answer, measured:** at most `MAX_OUTPUT_BYTES`, which is 100,000 bytes. Not more.

`bash` bounds its own output before it returns, so the harness never sees the rest. The store
then holds only what `bash` kept. A `cat` of a 2.6 MB file gives the model a preview plus a
handle to 100 KB, and the other 2.5 MB is gone. It was gone before this feature existed too,
and this feature does not recover it.

**How this was found.** A live run asked the model to find a marker at byte 1,308,924 of a
2.6 MB command output. The model answered correctly, and the answer proved nothing: it had
run `grep` on the file instead of reading the stored result. The store never held the marker.
A second run used a 74 KB output with its marker at byte 70,890, and that run really did call
`read_tool_result`. See `docs/verification/tool-result-handle.md`.

The first run is the more useful record. It nearly became a false proof, and the only reason
it did not is that the byte arithmetic was checked afterwards.

**Decision:** Record the limit and do not paper over it. `SPEC-tool-result-handle` states the
bound. `docs/features.md` carries `F-bash-streams-to-the-store` as `planned` for the fix.
Nothing in this delivery claims that a store makes a whole `bash` output reachable.

**Why not fix it now.** The fix is for `bash` to stream its output into the store instead of
holding it in memory, and to cap only what it keeps in memory. That is a real change to the
tool that D-bash-line-cap constrains, because the memory bound exists to stop
`head -c 400000000 /dev/zero` from exhausting the host. Streaming to a store keeps that
protection and changes where the bytes land, so it needs its own spec and its own live
verification.

**Rules out:** raising `MAX_OUTPUT_BYTES` because a store exists, which would trade a bounded
memory cost for an unbounded one. Claiming in any document that the store makes a whole
command output readable. Leaving the limit unrecorded, which would let the next reader assume
the store solves a problem it only half solves.
