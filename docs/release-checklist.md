# Release checklist

The repository is private until the first release. This page lists the steps
that make it public. Do the steps in order.

## Repository state

| Item | Value |
| --- | --- |
| Remote | `git@github.com:leduckhc/rho.git` |
| Visibility now | private |
| Visibility at release | public |

## Before you go public

1. Confirm every open track in `.rho-work/tracks/` meets its definition of done.
   `agentic-workflow.yaml` states what a track must prove.
2. Run the gate. All four commands must exit 0.

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace --all-features
   cargo build -p rho-cli --no-default-features --features minimal
   ```

3. Scan the tracked files for a secret.

   ```sh
   git grep -I -n -E "(sk-[A-Za-z0-9]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|xox[baprs]-|-----BEGIN [A-Z ]*PRIVATE KEY-----)" -- . ':!web/node_modules'
   ```

4. Scan every commit in the history for a secret.

   ```sh
   git grep -I -n -E "(sk-[A-Za-z0-9]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16})" $(git rev-list --all) -- . ':!web/node_modules'
   ```

5. Confirm no environment file or key file was ever committed.

   ```sh
   git log --all --diff-filter=A --name-only --pretty=format: | sort -u \
     | grep -E "\.env($|\.)|\.pem$|\.key$|id_rsa|\.p12$|\.netrc"
   ```

6. Read `docs/benchmarks.md`. Every number needs the command that produced it.

## Go public

```sh
gh repo edit leduckhc/rho --visibility public --accept-visibility-change-consequences
gh repo view --json visibility
```

## After you go public

1. Test the install command as an anonymous user.

   ```sh
   cargo install --git https://github.com/leduckhc/rho rho-cli
   ```

2. Test each `github.com/leduckhc/rho` link on the website. The website uses
   these links in `web/`:
   - the repository home link
   - the `docs/` tree link
   - the `docs/benchmarks.md` blob link
   - the `cargo install --git` command
3. Publish the website for `getrho.dev`.
4. Tag the release. The tag starts the `release.yml` workflow.

## Notes

- A private repository meters GitHub Actions minutes. A public repository does
  not. Watch the CI minutes until release.
- The website links and the install command fail for an anonymous visitor while
  the repository is private. This is expected. Do not publish the website
  before the repository is public.
