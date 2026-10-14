# shephrd

Adversarial review agent and risk classifier for Claude Code permission requests. 

shephrd is essentially just a long running daemon that takes in Claude Code `PermissionRequest` hook events. 

When your claude sessions wants to run a tool that needs your approval, shephrd dispatches a readonly review agentic using `claude -p` with your configured model and an adversarial reviewer prompt, scoped within the same cwd and conversations as the working agents so the reviewer can judge intent.

> [!WARNING]
> shephrd and its verdicts shouldn't be trusted in isolation.
> The reviewer is an LLM: its verdicts aren't deterministic, it can misread
> intent, and a prompt injection in a file it reads can fool it. It is meant to
> cut down on routine approval prompts, not to replace your own, stronger controls.
>
> Before using this tool, you should take time to build out and test your own user and project level permission controls, Claude Code's own `permissions.allow` and
> `permissions.deny` lists in `settings.json` are invaluable for this. 

## How it works

1. Claude Code prompts for your approval, which hits our custom hook in your "settings.json".
2. That hook runs `shephrd hook` to forward the request to the local daemon (`POST /review` on `127.0.0.1`).
3. The daemon builds a prompt from your recent messages, the agent's plan, and
   the request, then runs `claude -p` with only `Read`, `Grep` and `Glob`
   available, scoped to the requesting instance's cwd.
4. The adversarial agent returns one of three verdicts:

| Verdict    | Meaning                                              | What happens                    |
|------------|------------------------------------------------------|---------------------------------|
| `ALLOW`    | Routine or clearly in scope, and fully verifiable    | Approved automatically          |
| `ESCALATE` | Couldn't verify (opaque code, ambiguous intent)      | Your normal approval prompt, with reason |
| `DENY`     | Concrete risk (destructive, exfiltration, off-scope) | Your normal approval prompt, with reason |

Only `ALLOW` acts on its own. shephrd never denies anything outright, it only approves or
decides whether it thinks a decision is worth you taking a look.

## Install

You need the [`claude` CLI](https://docs.claude.com/en/docs/claude-code) on your
`PATH`, signed in. shephrd runs it to do each review.

Prebuilt downloads aren't published yet, so for now install from source with
[Rust](https://rustup.rs):

```sh
git clone https://github.com/BlaiseMoses01/shephrd.git
cd shephrd
cargo install --path .
```

This puts a `shephrd` binary in `~/.cargo/bin`, which rustup adds to your `PATH`.
After this you can delete the clone and run `shephrd` from any directory.

## Quick start

```sh
shephrd config --init   # write an editable config and prompt to ~/.config/shephrd/
shephrd enable          # add the hook to ~/.claude/settings.json
shephrd serve           # start the daemon; leave it running
```

Then use Claude Code as usual. `shephrd status` shows whether the daemon is up
and which files it's using.

## Turning it on and off

`shephrd enable` adds shephrd's hook to Claude Code's settings, and
`shephrd disable` removes it. Neither installs or removes the program itself.

- They change `~/.claude/settings.json` by default, or `./.claude/settings.json`
  with `--project` to cover only one project.
- Every other setting and hook in the file is kept. The previous version is saved
  as `settings.json.bak`.
- Running `enable` again (for example after moving the binary) updates the
  entry rather than adding a second one.

To pause shephrd for a while, either `shephrd disable`, or just stop the daemon.
While the daemon isn't running the hook steps aside, and every request goes to
your normal permission prompt.

## Running the daemon

You start `shephrd serve` yourself and leave it running, for example in a spare
terminal tab or tmux pane. Stop it with Ctrl-C.

One daemon serves every Claude Code session on your machine at once. Each
session is locked to the project folder it started in. Up to `max_concurrent`
reviews run at a time; the rest wait their turn. Restart the daemon after
changing the config or the prompt.

## Commands

| Command                        | What it does                                                         |
|--------------------------------|----------------------------------------------------------------------|
| `shephrd serve [--port]`       | Run the review daemon                                                |
| `shephrd status`               | Is the daemon running? Which config, prompt and log are in use?      |
| `shephrd enable [--project]`   | Turn shephrd on: add its hook to Claude Code's settings              |
| `shephrd disable [--project]`  | Turn shephrd off: remove its hook                                    |
| `shephrd config [--init]`      | Show the settings in effect, or write the editable config and prompt |
| `shephrd judge`                | One-shot, no daemon: judge a request from stdin and print the review |
| `shephrd hook [--url]`         | What Claude Code runs: forward a request to the daemon               |

All commands accept `--config <path>`.

## Configuration

`shephrd config --init` writes two files you can edit. It never overwrites
either one; if one is missing, it writes just that one.

| File                                  | What it is                                    |
|---------------------------------------|-----------------------------------------------|
| `~/.config/shephrd/config.toml`       | Settings (below)                              |
| `~/.config/shephrd/reviewer.md`       | The reviewer prompt (see [Tuning](#tuning-the-reviewer-prompt)) |
| `~/.local/state/shephrd/decisions.jsonl` | Log of every verdict, written by the daemon |


Within the config, e000very setting is optional; without a config file, shephrd uses the defaults
and the built-in prompt. See [`shephrd.example.toml`](shephrd.example.toml) for
the commented version.

| Key              | Default             | Description                                          |
|------------------|---------------------|------------------------------------------------------|
| `model`          | `"sonnet"`          | Model the judge runs as (alias or full model ID)     |
| `timeout_secs`   | `90`                | Judge time limit before falling back to you          |
| `user_turns`     | `5`                 | How many of your recent prompts the judge sees       |
| `port`           | `7878`              | Daemon port (`127.0.0.1` only)                       |
| `max_concurrent` | `4`                 | Reviews that may run at once; extras wait their turn |
| `decision_log`   | `~/.local/state/shephrd/decisions.jsonl` | Append-only log of every verdict, one JSON per line |
| `prompt`         | `reviewer.md` next to the config, else built in | The reviewer prompt file        |

Relative paths are relative to the config file. If you change `timeout_secs`,
run `shephrd enable` again so the hook's own timeout stays longer than the judge's.
Restart the daemon after any change.

## Updating

From your clone, pull and reinstall, then restart the daemon:

```sh
git pull
cargo install --path . --force
```

Your config and prompt are left as they are. A new version can ship changes to
the built-in prompt; if you've made your own `reviewer.md`, compare it with
[`prompts/reviewer.md`](prompts/reviewer.md) to pick those up.

## Uninstalling

Do it in this order, so Claude Code isn't left pointing at a program that's gone:

```sh
# 1. Stop the daemon (Ctrl-C wherever `shephrd serve` is running)
shephrd disable             # 2. remove the hook (add --project for any projects you enabled)
cargo uninstall shephrd     # 3. remove the program
rm -rf ~/.config/shephrd ~/.local/state/shephrd   # 4. optional: config, prompt and log
```

## Tuning the reviewer agent's prompt

If shephrd approves things you'd rather see yourself, or asks about things it
should just approve, edit the reviewer prompt at `~/.config/shephrd/reviewer.md` (`shephrd config --init`
writes it; `shephrd status` shows which prompt is in use). It holds the rules and
the definitions of `ALLOW`, `ESCALATE` and `DENY`. Without that file, shephrd uses
the built-in copy of [`prompts/reviewer.md`](prompts/reviewer.md).

- **Too permissive:** add the cases you want brought to you to the list of
  things it must not approve, or narrow the `ALLOW` definition. NOTE: if there are specific commands and things you see it miss
  regularly, consider adding those as programmatic denies in your claude rules as well.

- **Too strict:** say explicitly which routine actions are fine to approve
  (for example, your project's build and test commands). similar to above, adding these to claude rules as explicit allows forgoes the need for review
  and may have more consistent results. 

Keep every `{{placeholder}}` in the file: that's where shephrd inserts your
messages and the request. shephrd refuses to start with a prompt that's missing
one. After editing, restart `shephrd serve`.

If you'd like to benchmark your edits against a few generics scenarios before giving it a spin for real, you can run the golden
set, which runs the adversarial loop against a few mock requests.

```sh
cargo test --test golden -- --ignored --nocapture
```

On this note, if shephrd gets a verdict you really care about wrong, add a case for it in
`tests/golden/cases/<name>/` and run it so you can see if your prompt edits correct the adverse behavior.
Obviously theres non-determinism at play here, but it tends to be a decent gauge. 

## License

MIT. See [LICENSE](LICENSE).
