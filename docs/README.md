# Docs Index（v0.2.x）

> 约定：标题里的 “v0.2.0 口径” = 已实现；“TODO：规格草案” = 未实现但先把边界写死（避免未来跑偏）。
>
> 当前 UI 范围（强约束）：**只保留 Rust TUI（`omne tui`）**。Web GUI 暂停，不作为当前阶段交付目标。

## 1) 从哪里开始

- `docs/v0.2.0_parity.md`：v0.2.0 对齐清单（实现状态 + TODO）
- `docs/TODO.md`：新增待办项与版本计划（例如 v0.3.0 Node.js）
- `docs/implementation_plan.md`：vNext 实现计划与里程碑
- `docs/rts_workflow.md`：目标态使用流程（RTS 风格）
- `docs/development_process.md`：重新开发流程（Agent-first）
- `docs/start.md`：入口（含 v0.1.1 legacy 背景）

## 2) 运行时与协议（v0.2.0 已实现为主）

- `docs/thread_event_model.md`：Thread/Turn/Item 与 JSONL 回放口径
- `docs/omne_data.md`：`./.omne_data/` 目录约定（项目配置 + 运行时数据）
- `docs/runtime_layout.md`：`omne_root`（默认 `./.omne_data/`）目录结构与“从 ID 定位到文件”
- `docs/modes.md`：Mode（角色权限边界）与合并语义
- `docs/approvals.md`：Approvals 事件模型与 policy（含 `prompt_strict` / Escalate 口径）
- `docs/execpolicy.md`：ExecPolicy（`process/start` 命令前缀规则）
- `docs/redaction.md`：脱敏与 env scrub（避免 secrets 入日志/产物）
- `docs/artifacts.md`：Artifacts（产物）与 metadata（含 preview/history TODO）
- `docs/attention.md`：Attention/Inbox（派生视图）
- `docs/notifications.md`：通知与 bell（含去重/节流与 stale process 现状）
- `docs/budgets.md`：Budgets/timeout → `Stuck`（含 loop/summary TODO）
- `docs/tool_parallelism.md`：read-only tool 并发口径
- `docs/workspace_hooks.md`：Workspace hooks（`.omne_data/spec/workspace.yaml`）
- `docs/reference_repo.md`：Reference repo/snapshot（只读参考；v0.2.0 最小实现）

## 3) 目标态/未实现规格（TODO 草案）

- `docs/tui.md`：TUI（薄客户端；v0.2.0 P0）
- `docs/daemon.md`：Daemon（常驻 server）vs 每次启动子进程（取舍与约束）
- `docs/checkpoints.md`：checkpoint/rollback（turn 级回滚）
- `docs/hooks.md`：hooks（SessionStart/PreToolUse/PostToolUse/Stop）
- `docs/workflow_commands.md`：Workflow/Commands（Markdown + frontmatter）
- `docs/special_directives.md`：特殊指令（slash/at）的结构化表达
- `docs/subagents.md`：Subagents（fan-out/fan-in；当前仅有 `thread/fork` + `agent_spawn` 原语）
- `docs/repo_index.md`：repo/index + repo/search（把搜索结果写成 artifact）
- `docs/model_routing.md`：Model routing（router 已落地；含后续扩展 TODO）
- `docs/ditto_llm.md`：Ditto-LLM（统一 LLM SDK 方案草案；ditto-llm 为独立仓库，本仓库通过 path 依赖引用）
- `docs/presets.md`：Presets（已支持 `preset list/import/export`；导入会写入 `preset_applied` provenance，并在 `thread/config-explain` 展示 `preset` layer；完整规范仍有 TODO）
- `docs/mcp.md`：MCP client + 实验性 stdio server（含后续扩展 TODO）
- `docs/execve_wrapper.md`：execve wrapper（v0.2.x 已落地最小链路；含后续扩展 TODO）
- `docs/os_hardening.md`：OS/process hardening（TODO）

## 4) 调研索引

- `docs/research/README.md`

## 5) 命令约定（可复制）

如果你没安装 `omne` 到 PATH，用 `cargo run` 运行（所有文档里的 `omne ...` 都可按此替换）：

```bash
$ cargo run -p omne -- --help
$ cargo run -p omne-app-server -- --help

# 全屏 TUI（默认新建 thread）
$ cargo run -p omne
# 或显式：
$ cargo run -p omne -- tui

# 交互式 CLI（REPL 风格）
$ cargo run -p omne -- cli
# 兼容别名：
$ cargo run -p omne -- repl
```

快速搜索：

```bash
$ rg "<keyword>" docs
$ rg "<keyword>" crates
```

配置目录约定：

- `./.omne_data/`：项目级数据根（运行时 threads/artifacts；项目级覆盖配置 `config.toml` + secrets `.env`；项目 spec 在 `.omne_data/spec/`）
