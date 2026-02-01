# 项目启动（omne-agent / Rust）

> 文档索引：`docs/README.md`  
> v0.2.0 对齐清单：`docs/v0.2.0_parity.md`  
> 目标态使用流程（RTS）：`docs/rts_workflow.md`  
> 重新设计/开发流程：`docs/development_process.md`  
> vNext 计划：`docs/implementation_plan.md`

## Quickstart（本地）

```bash
# build
cargo build -p omne-agent -p omne-agent-app-server

# init project data root
./target/debug/omne-agent init

# start TUI (thin client over JSON-RPC)
./target/debug/omne-agent tui
```

## agent_root（项目数据根）

- 默认：当前目录下 `./.omne_agent_data/`
- 覆盖：
  - CLI：`omne-agent --root <path> ...`
  - env：`OMNE_AGENT_ROOT=<path>`

`omne-agent` client 默认会优先连接 `<agent_root>/daemon.sock`（daemon 模式）；连接失败则回退到 spawn `omne-agent-app-server`（stdio JSON-RPC）。

更多目录布局见 `docs/runtime_layout.md`。

## 开发 gates

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
