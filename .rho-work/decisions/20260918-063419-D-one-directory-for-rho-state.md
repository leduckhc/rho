# D-one-directory-for-rho-state — keep every user-scoped file under `~/.rho`

**Decision:** rho keeps every user-scoped file under one directory, `~/.rho/`,
instead of scattering them under XDG directories.

**Reason:** a user should not guess whether a config file lives under
`~/.config/rho/`, `~/.local/share/rho/`, or `~/.rho/`. One directory makes
backup, migration, and inspection obvious. The XDG base directories remain a
read-only fallback for anyone who already created a file there before this
release.

**Rule:** new files are created under `~/.rho/`. Old files under
`$XDG_CONFIG_HOME/rho/` or `~/.config/rho/` are read as a legacy fallback only
when no file exists at the new path. A migration notice names the old path
once per run.
