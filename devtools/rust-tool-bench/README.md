# rust-tool-bench (dev only)

`rust-tool-bench` is a development-only benchmark runner.

- It calls the real Rust CLI binary (`target/debug/omne`).
- It runs real turn/tool execution via `omne exec` and `omne thread events`.
- It does **not** modify main product command surface.

## Run

```bash
cargo run --manifest-path devtools/rust-tool-bench/Cargo.toml -- \
  --repo-root /root/autodl-tmp/zjj/p/omne-agent \
  --cases scripts/tool_suite/cases.facade.full.v1.json \
  --out-dir docs/reports/dev-rust-tool-bench-run
```

Optional:

- `--model <model>`
- `--openai-base-url <url>`
- `--mode <mode>`
- `--max-cases <n>`
- `--fail-fast`

## Output

- `raw_results.json`: full benchmark data
- `report.md`: quick summary table
