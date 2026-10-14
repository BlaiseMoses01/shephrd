# Contributing

## Before you push

CI runs these on every push to `main` and every pull request. Run them locally
first so CI doesn't fail on something you could have caught:

```sh
cargo fmt --all                                     # CI checks formatting with --check
cargo clippy --all-targets --locked -- -D warnings  # any warning fails CI
cargo test --locked
cargo deny check                                    # cargo install cargo-deny --locked
gitleaks git .                                      # brew install gitleaks
```

| Check       | Workflow        | What fails it                                                                       |
|-------------|-----------------|-------------------------------------------------------------------------------------|
| rustfmt     | `ci.yml`        | Code that isn't `cargo fmt`-formatted                                               |
| clippy      | `ci.yml`        | Any warning                                                                         |
| test        | `ci.yml`        | A failing test, on Ubuntu or macOS                                                  |
| cargo-deny  | `ci.yml`        | A dependency with a security advisory, a license not in `deny.toml`, or a crate from outside crates.io |
| gitleaks    | `gitleaks.yml`  | Anything that looks like a secret, anywhere in the git history (also runs weekly)   |

Test fixtures must not contain real data. When you capture a real hook event for
`tests/fixtures/`, replace your home directory, session IDs, repo names and any
prompt or plan text with generic values before committing it.

## Testing

```sh
# Unit and integration tests
cargo test

# Golden set: real judge runs against hand-built scenarios (costs money, ~1 min)
cargo test --test golden -- --ignored --nocapture

# Run a subset, or compare a different model
GOLDEN_FILTER=hidden_rm cargo test --test golden -- --ignored --nocapture
GOLDEN_MODEL=haiku cargo test --test golden -- --ignored --nocapture
```

Golden cases live in `tests/golden/cases/<name>/`, each with a `case.json`
(intent, request, expected verdicts), a `project/` that becomes the session
root, and an optional `outside/` the judge must not read. Each run writes an
audit log to `target/golden/`.

The golden set loads your shephrd config the same way the daemon does, so if
`~/.config/shephrd/reviewer.md` exists it tests that prompt, not
`prompts/reviewer.md`. Point `SHEPHRD_CONFIG` at an empty directory's
`config.toml` to test the repo's prompt. CI never runs the golden set.

## Releasing

One version tag publishes everything:

1. Bump `version` in `Cargo.toml`, run `cargo build` so `Cargo.lock` updates,
   and commit.
2. Run the golden set, and try the build in a real Claude Code session.
3. Tag and push:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

The tag must match the `Cargo.toml` version (`0.2.0` ↔ `v0.2.0`); both release
workflows check this. A tag with a suffix such as `v0.2.0-beta.1` is published
as a prerelease.

The tag starts two workflows:

| Workflow            | What it does                                                                         |
|---------------------|--------------------------------------------------------------------------------------|
| `release.yml`       | Builds macOS and Linux binaries, creates the GitHub Release with the binaries, checksums and `shephrd-installer.sh`, and pushes `Formula/shephrd.rb` to [`BlaiseMoses01/homebrew-tap`](https://github.com/BlaiseMoses01/homebrew-tap) |
| `publish-crate.yml` | Publishes the crate to crates.io (skipped if that version is already there)          |

Check the Actions tab until both are green. If one fails after the other has
published, fix the problem and re-run the failed workflow from the Actions tab;
don't move the tag. A version that's on crates.io can't be published again,
only yanked (`cargo yank --version 0.2.0`), so if the crate itself is broken,
release a new patch version.

### How publishing is set up

- **Binaries and installers** come from [cargo-dist](https://github.com/axodotdev/cargo-dist).
  Its settings (targets, installers, tap) are in `dist-workspace.toml`, and it
  generated `.github/workflows/release.yml`.
- **Homebrew** needs the `HOMEBREW_TAP_TOKEN` repository secret: a fine-grained
  token with Contents read and write on `homebrew-tap` only. Fine-grained tokens
  expire, so when it does, the release's Homebrew step fails. Make a new token,
  update the secret, and re-run the job.
- **crates.io** uses trusted publishing. There's no stored token: crates.io
  trusts `publish-crate.yml` in this repo, run in the `crates-io` environment
  (set under the crate's Settings → Trusted Publishing on crates.io). Renaming
  the workflow file or the environment breaks publishing until that setting is
  updated to match.
- **The crate package** contains only the files listed in `include` in
  `Cargo.toml`. If you add a file that the code reads at build time (with
  `include_str!` or similar), add it there, and check with
  `cargo package --list`.

### Changing the release workflow

`release.yml` is generated, so change `dist-workspace.toml` rather than editing
it by hand, then regenerate it. `allow-dirty = ["ci"]` is set so that
Dependabot's action bumps don't stop releases, but it also makes
`dist generate` refuse to run, so turn it off while regenerating:

1. Comment out `allow-dirty = ["ci"]` in `dist-workspace.toml`.
2. Copy the current action SHAs from `release.yml` into
   `[dist.github-action-commits]`, so regenerating doesn't undo Dependabot's
   updates.
3. Run `dist generate --mode ci`, then `dist plan` to check the release contents.
4. Add the `# vX.Y.Z` comments back after each pinned SHA in `release.yml`.
   Dependabot uses them to know which version is pinned.
5. Uncomment `allow-dirty`, and commit.

## Dependencies and GitHub Actions

Dependabot (`.github/dependabot.yml`) opens weekly PRs. Minor and patch Cargo
updates come grouped in one PR, majors separately, and all action updates
together. CI has to pass before merging; cargo-deny catches new advisories and
license changes.

Every action in `.github/workflows/` is pinned to a full commit SHA, with the
release it belongs to in a comment:

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
```

Tags can be moved to point at different code; a commit SHA can't. When adding
an action, pin it the same way. Look up the SHA for a release with:

```sh
gh api repos/OWNER/REPO/commits/TAG --jq .sha
```

`dtolnay/rust-toolchain` has no releases, so Dependabot can't update it. Bump
its SHA by hand now and then
(`gh api repos/dtolnay/rust-toolchain/commits/master --jq .sha`). Run
`actionlint` (`brew install actionlint`) after changing a workflow.

## Layout

```
src/
  main.rs              CLI
  server.rs            daemon, root pinning, decision log
  client.rs            hook client and verdict → hook output
  judges/cli_judge.rs  runs `claude -p` as the judge
  prompt.rs            fills in the reviewer prompt; loads and checks custom ones
  transcript.rs        extracts your recent prompts and the plan
  config.rs            config loading
  settings.rs          `enable` / `disable`: the hook in Claude Code's settings.json
prompts/reviewer.md    the built-in judge prompt
tests/                 integration tests and the golden set
.github/workflows/     CI, gitleaks, release (cargo-dist) and crates.io publishing
deny.toml              cargo-deny policy
dist-workspace.toml    cargo-dist release settings
```
