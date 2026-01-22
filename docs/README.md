# Docs Index（v0.2.x）

> 约定：标题里的 “v0.2.0 口径” = 已实现；“TODO：规格草案” = 未实现但先把边界写死（避免未来跑偏）。

## 1) 从哪里开始

- `docs/v0.2.0_parity.md`：v0.2.0 对齐清单（实现状态 + TODO）
- `docs/implementation_plan.md`：vNext 实现计划与里程碑
- `docs/rts_workflow.md`：目标态使用流程（RTS 风格）
- `docs/development_process.md`：重新开发流程（Agent-first）
- `docs/start.md`：入口（含 v0.1.1 legacy 背景）

## 2) 运行时与协议（v0.2.0 已实现为主）

- `docs/thread_event_model.md`：Thread/Turn/Item 与 JSONL 回放口径
- `docs/runtime_layout.md`：`.code_pm/` 目录结构与“从 ID 定位到文件”
- `docs/modes.md`：Mode（角色权限边界）与合并语义
- `docs/approvals.md`：Approvals 事件模型与 policy（含 Escalate TODO）
- `docs/execpolicy.md`：ExecPolicy（`process/start` 命令前缀规则）
- `docs/redaction.md`：脱敏与 env scrub（避免 secrets 入日志/产物）
- `docs/artifacts.md`：Artifacts（产物）与 metadata（含 preview/history TODO）
- `docs/attention.md`：Attention/Inbox（派生视图）
- `docs/notifications.md`：通知与 bell（含 stale process TODO）
- `docs/budgets.md`：Budgets/timeout → `Stuck`（含 loop/summary TODO）
- `docs/tool_parallelism.md`：read-only tool 并发口径
- `docs/workspace_hooks.md`：Workspace hooks（`.codepm/workspace.yaml`）

## 3) 目标态/未实现规格（TODO 草案）

- `docs/checkpoints.md`：checkpoint/rollback（turn 级回滚）
- `docs/hooks.md`：hooks（SessionStart/PreToolUse/PostToolUse/Stop）
- `docs/workflow_commands.md`：Workflow/Commands（Markdown + frontmatter）
- `docs/special_directives.md`：特殊指令（slash/at）的结构化表达
- `docs/subagents.md`：Subagents（fan-out/fan-in；当前仅有 `thread/fork` + `agent_spawn` 原语）
- `docs/repo_index.md`：repo/index + repo/search（把搜索结果写成 artifact）
- `docs/model_routing.md`：Model routing（Router TODO；现状含 config explain）
- `docs/presets.md`：Presets（导入/导出 TODO；现状可手工用 `thread/configure` 达成）
- `docs/mcp.md`：MCP client/server（TODO）
- `docs/reference_repo.md`：Reference repo/snapshot（只读参考；TODO）
- `docs/execve_wrapper.md`：execve wrapper（TODO）
- `docs/os_hardening.md`：OS/process hardening（TODO）

## 4) 调研索引

- `docs/research/README.md`

## 5) 命令约定（可复制）

如果你没安装 `pm` 到 PATH，用 `cargo run` 运行（所有文档里的 `pm ...` 都可按此替换）：

```bash
$ cargo run -p pm -- --help
$ cargo run -p pm-app-server -- --help

# 交互式对话/执行环境（REPL）
$ cargo run -p pm
# 或显式：
$ cargo run -p pm -- repl
```

快速搜索：

```bash
$ rg "<keyword>" docs
$ rg "<keyword>" crates
```

配置目录约定：

- `./.codepm/`：项目可提交配置（modes/workspace/hooks/presets/…）
- `./.code_pm/`：运行时数据目录（threads/artifacts/state；不要提交）
