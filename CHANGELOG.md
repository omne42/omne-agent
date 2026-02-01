# Changelog

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added

- Agent-first Rust workspace：`omne-agent` CLI + `omne-agent-app-server`（JSON-RPC 控制面）。
- 项目数据根 `./.omne_agent_data/`：支持 `config.toml`、`config_local.toml`、`.env` 与 `spec/`。
- Thread/Turn 事件模型：append-only `events.jsonl` + `ThreadState` 事件派生。
- Approvals、modes（权限边界）、execpolicy、sandbox 策略与可解释性（`thread/config-explain`）。
- TUI（Ratatui thin client）与 REPL 风格 `omne-agent cli`。
- Reference repo（只读快照）：`omne-agent reference import/status`。
- Checkpoints（create/list/restore）：快照落盘到 thread artifacts。
- MCP：client +（可选）server 原语与审计落盘。
- 内置 role prompts：`prompt/roles/*.md`（含权限 frontmatter）编译期嵌入；每 turn 以 `@role <role>...</role>` 注入到最新 user message；当 `capabilities.prompt_cache=false` 时同时写入 system prompt。

### Changed

- Process log rotate 命名统一为 `*.segment-0001.log`。
- 部分通用能力拆为独立仓库并通过 path 依赖复用：`mcp-kit`、`safe-fs-tools`。
- Node packages（分发方向的最小落地）：`packages/omne-agent`、`packages/app-server-client`。
- Auto compact：支持 `best_context`/`max_context`（模型配置别名），并在超阈值时先 prune 老的 tool output，再做 summary 重建上下文。
- Dev：pre-commit 禁止新增/修改 `partNN.*` 风格文件名。

### Security

- 默认对事件、工具结果与进程输出做 secrets 脱敏；并默认拒绝 `.env` 的读取与写入。
- 可选 OS/process hardening：`OMNE_AGENT_HARDENING=best_effort`。
