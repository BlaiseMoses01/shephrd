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

## Layout

```
src/
  main.rs            CLI
  server.rs          daemon, root pinning, decision log
  client.rs          hook client and verdict → hook output
  judges/cli_judge.rs  runs `claude -p` as the judge
  prompt.rs          fills in prompts/reviewer.md
  transcript.rs      extracts your recent prompts and the plan
  config.rs          config loading
prompts/reviewer.md  the judge prompt
tests/               integration tests and the golden set