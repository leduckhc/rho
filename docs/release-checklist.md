# Release checklist

**The repository is public as of 20260831.** It went public early, to stop the Actions
blackout that was failing every job for want of a runner. So the `## Go public` step below is
done, and the steps before it were run after it rather than before. Each one is marked with
what it found.

This page lists the steps to the first release. Do the remaining ones in order.

## Repository state

| Item | Value |
| --- | --- |
| Remote | `git@github.com:leduckhc/rho.git` |
| Visibility now | public, since 20260831 |
| Visibility at release | public |

## Before you go public

1. ~~Confirm every open track in `.rho-work/tracks/` meets its definition of done.~~ **Done
   20260831.** The one track said `open` with two stages `in-progress` while its work had
   shipped in PR #9 and its spec said `delivered`. Every stage is now closed against evidence
   that already existed, named per stage.
2. Run the gate. All four commands must exit 0.

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace --all-features
   cargo build -p rho-cli --no-default-features --features minimal
   ```

3. ~~Scan the tracked files for a secret.~~ **Done 20260831, clean.**

   ```sh
   git grep -I -n -E "(sk-[A-Za-z0-9]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|xox[baprs]-|-----BEGIN [A-Z ]*PRIVATE KEY-----)" -- . ':!web/node_modules'
   ```

4. ~~Scan every commit in the history for a secret.~~ **Done 20260831, clean over 149 commits.**

   ```sh
   git grep -I -n -E "(sk-[A-Za-z0-9]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16})" $(git rev-list --all) -- . ':!web/node_modules'
   ```

5. ~~Confirm no environment file or key file was ever committed.~~ **Done 20260831, none.**

   ```sh
   git log --all --diff-filter=A --name-only --pretty=format: | sort -u \
     | grep -E "\.env($|\.)|\.pem$|\.key$|id_rsa|\.p12$|\.netrc"
   ```

6. ~~Read `docs/benchmarks.md`. Every number needs the command that produced it.~~ **Done
   20260831, and it found four problems.** Five numbers came from a bench outside the
   repository, one line count contradicted a test, the headline binary size was two
   generations stale, and the README and the website both overclaimed. See PR #17.

## Go public

```sh
gh repo edit leduckhc/rho --visibility public --accept-visibility-change-consequences
gh repo view --json visibility
```

## After you go public

1. ~~Test the install command as an anonymous user.~~ **Done 20260831, and it works.**

   ```sh
   cargo install --git https://github.com/leduckhc/rho rho-cli
   ```

   A clean clone of `6c62b28` built in 2m05s and installed one executable, `rho`, at exit 0,
   with no credentials and no token in the environment. The installed binary reports
   `rho 0.1.0`, prints its help, and on a first run with no credential it names the variable
   to set and exits 1. It was installed to a throwaway `--root`, so it did not touch a real
   `~/.cargo/bin`.

2. ~~Test each `github.com/leduckhc/rho` link on the website.~~ **Done 20260831: all three
   return 200 anonymously.** The website uses these links in `web/`:
   - the repository home link
   - the `docs/` tree link
   - the `docs/benchmarks.md` blob link
   - the `cargo install --git` command
3. Publish the website for `getrho.dev`. **No workflow does this.** There is no pages or
   deploy job in `.github/workflows/`, so this is a manual step and it needs the hosting
   credentials. `cd web && npm run build` succeeds and writes `web/dist`.
4. Tag the release. The tag starts the `release.yml` workflow, which builds a binary per
   target and attaches it to a GitHub release.

   **Check before you tag: everything meant for the release is pushed.** A tag names a
   commit, so an unpushed fix is a fix the release does not carry. On 20260831 this tree
   held four unpushed commits on a local `main` that had diverged from `origin/main`,
   including a fix for a failed run that leaked a task and hung a consumer for ever. A
   release cut then would have shipped without it.

## Notes

- A private repository meters GitHub Actions minutes. A public repository does
  not. Watch the CI minutes until release.
- The website links and the install command failed for an anonymous visitor while the
  repository was private. Both are now verified: the three `github.com/leduckhc/rho` links in
  `web/` each return 200 anonymously, and `cargo install --git https://github.com/leduckhc/rho
  rho-cli` builds and installs from a clean clone with no credentials. See the step list above.
