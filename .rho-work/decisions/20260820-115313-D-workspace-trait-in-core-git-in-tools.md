# D-workspace-trait-in-core-git-in-tools — The isolation trait lives in the core, the git code lives in the tools

**The question.** Which crate owns the isolation trait? Which crate owns the git
implementation?

**The decision.** `rho-core` owns the `Workspace` trait. `rho-tools` owns the
`GitWorktreeWorkspace` implementation. `rho-core` gains no git dependency.

**Why this split.** rho already uses it once. `rho-core` owns the `CommandRunner` trait and
holds no sandbox. `rho-tools` supplies `SandboxedRunner`. See `SPEC-agent-tasks`. The
isolation trait copies that pattern. The abstract capability lives in the core. The
implementation that needs a real dependency lives in the tools.

**Why the core stays git-free.** AGENTS.md keeps `rho-core` free of heavy dependencies. A git
dependency in the core would bind every core user to git. Most core users never isolate a
child. So the core defines the trait only.

**The extension point.** A third party writes one `impl rho_core::Workspace`. It installs the
implementation in `SpawnEnv.workspace`. It adds a container, a copy-on-write clone, or an
overlay this way. It edits nothing in rho.

**What this rules out.** The core cannot call git. A third party cannot need a fork to add an
isolation backend. The git code cannot leak into a crate that does not run tools.
