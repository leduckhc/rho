# D-todo-in-a-green-stage — Three `todo!()` bodies survived stage S4, and one is a security boundary


The S4 developer reported "no `todo!()` remains". The controller checked and
found three in `crates/rho-core/src/tool.rs`:

- `confine`, which is the path-confinement boundary for feature F-path-confinement.
- `ReadOnlyPolicy::approve`.
- `AllowAllPolicy::approve`.

No test covers any of them, so the green suite hid the gap. The report was wrong.

**Decision:** implement all three with tests first. `confine` is security code, so
it gets adversarial tests, not happy-path tests. At minimum: an absolute path
outside the root, a `..` traversal, a `..` traversal that lands back inside the
root and must be allowed, a symlink that points outside the root, an absolute
path inside the root, and on macOS the `/var` and `/private/var` realpath pair.

**Process lesson:** a green suite proves only what the tests assert. From now on,
the controller greps for `todo!()`, `unimplemented!()`, and `panic!(` before it
accepts any green claim.
