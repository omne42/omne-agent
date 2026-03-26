# 参考资料：tool-surface-consolidation

本文件用于未来接力开发者快速定位事实依据，避免重复调研。

## A. 代码事实（当前实现）

### A1. 工具目录与动作映射

- `crates/app-server/src/agent/tools/spec.rs`
  - 事实：当前 agent tool 名称与 action 的静态映射包含 facade + legacy 两层；facade 为
    `facade/workspace`、`facade/process`、`facade/thread`、`facade/artifact`、`facade/integration`。
  - 事实：`thread` facade 已补齐子代理生命周期 action：
    `subagent/send_input`、`subagent/wait`、`subagent/close`。
- `crates/core/src/allowed_tools.rs`
  - 事实：`allowed_tools` 允许动作全集与 mode 决策映射已覆盖 facade action 与 legacy action。
  - 事实：新增生命周期 action 的 mode 映射已落地（spawn/read/spawn∩process.kill）。

### A2. 模型侧工具暴露裁剪

- `crates/app-server/src/agent/tools/catalog.rs`
  - 事实：新增 facade schema 与开关：
    - `OMNE_TOOL_FACADE_ENABLED`（默认 `true`）
    - `OMNE_TOOL_FACADE_EXPOSE_LEGACY`（默认 `false`）
  - 事实：默认仅暴露 facade tool surface（`<=5`），legacy 默认隐藏，可按开关回退。
- `crates/app-server/src/agent/core/run_turn.rs`
  - 事实：每轮记录 `tool_count` 和 `tool_schema_bytes`。
- `crates/app-server/src/agent/tools/dispatch/run_tool_call_once.rs`
  - 事实：facade `op -> mapped_action` 路由已落地；`op=help` 返回 quickstart + advanced；
    facade wrapper 事件含 `facade_tool/op/mapped_action`。
  - 事实：`thread` facade 新增 `send_input/wait/close`（含 `close_agent` 别名）并接入
    `allowed_tools -> mode -> approval`。
  - 事实：生命周期 handler 已落地：`handle_subagent_send_input_tool` /
    `handle_subagent_wait_tool` / `handle_subagent_close_tool`。

### A2.2 生命周期补齐验证（2026-03-02）

- 命令：
  - `cargo test -p omne-app-server reference_repo_file_tools_tests::facade_ -- --nocapture`
  - `cargo test -p omne-app-server facade_tool_tests:: -- --nocapture`
  - `cargo test -p omne-core allowed_tools:: -- --nocapture`
  - `cargo fmt --all --check`
  - `cargo check --workspace --all-targets`
- 输出摘要：
  - facade 生命周期新增测试通过（含 `allowed_tools/mode/approval` 拒绝路径与
    `send_input -> wait -> close` 链路）。
  - `allowed_tools` 全量映射断言通过。
  - `fmt/check` 通过（保留既有 `agent-cli` 未使用函数 warning）。

### A2.3 CLI/TUI 映射摘要展示验证（2026-03-02）

- 代码位置：
  - `crates/agent-cli/src/main/ask_exec.rs`
  - `crates/agent-cli/src/main/process_and_utils/event_render.rs`
  - `crates/agent-cli/src/main/tui/tool_format.rs`
  - `crates/agent-cli/src/main/tui/ui_state_core.rs`
- 事实：
  - `ToolStarted/ToolCompleted` 在 CLI/TUI 输出中会显示
    `facade_tool/op/mapped_action` 映射摘要。
  - TUI 在 `denied/failed` 状态下同样保留映射摘要，便于定位真实 internal action。
- 命令：
  - `cargo test -p omne ask_exec_tests:: -- --nocapture`
  - `cargo test -p omne event_render_tests:: -- --nocapture`
  - `cargo test -p omne tool_format_tests:: -- --nocapture`
  - `cargo check -p omne`
  - `cargo fmt --all --check`

### A2.1 成本对比基线（已实测）

- 命令：
  - `cargo test -p omne-app-server tool_catalog_tests::facade_tool_surface_reduces_schema_bytes_vs_legacy_default -- --nocapture`
- 输出：
  - `legacy_count=21 legacy_bytes=7669`
  - `facade_count=4 facade_bytes=1815`
- 结论：
  - 默认工具数从 21 降到 4（`<=5` 达成）
  - `tool_schema_bytes` 降幅约 `76.33%`

### A2.4 动态工具注册 MVP（2026-03-02）

- `crates/app-server/src/agent/tools/dynamic_registry.rs`
  - 事实：支持本地 registry 文件加载，默认路径为
    `<thread_root>/.omne_data/spec/tool_registry.json`。
  - 事实：开关 `OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED` 默认 `false`；
    可选路径覆盖 `OMNE_TOOL_DYNAMIC_REGISTRY_PATH`。
  - 事实：MVP 仅允许 read-only mapped tool，非只读/未知映射 fail-closed。
- `crates/app-server/src/agent/tools/catalog.rs`
  - 事实：动态工具在 turn 构建阶段注入 model-facing schema，并继续受
    `allowed_tools` 与暴露策略过滤。
- `crates/app-server/src/agent/tools/dispatch/run_tool_call_once.rs`
  - 事实：未知 tool 名会尝试 dynamic registry 映射，并复用既有 internal handler。
  - 事实：返回结构含 `dynamic_tool/mapped_tool/mapped_action`；参数校验失败错误码为
    `dynamic_invalid_params`。
- 命令：
  - `cargo test -p omne-app-server tool_catalog_tests::dynamic_registry_ -- --nocapture`
  - `cargo test -p omne-app-server reference_repo_file_tools_tests::dynamic_registry_tool_ -- --nocapture`

### A3. 运行时硬约束链路

- `crates/app-server/src/main/approval.rs`
  - 事实：`enforce_thread_allowed_tools` 运行时硬拒绝。
- `crates/app-server/src/main/process_control/start.rs`
  - 事实：`process/start` 链路顺序为 allowlist -> sandbox -> mode -> execpolicy -> approval。
- `crates/process-runtime/src/lib.rs`
  - 事实：网络命令启发式识别（network deny 依赖）。

### A4. 渐进披露现有基础

- `crates/app-server/src/agent/core/auto_compact_and_config.rs`
  - 事实：`$skill` 按需加载 `SKILL.md`（progressive disclosure 基础能力）。
- `crates/agent-cli/src/main/repl.rs`
  - 事实：`/help` 已补充 facade help-first 指引（默认最简 + `op=help` + `topic` 进阶）。

### A5. Tool 执行归属（2026-03-04 对齐）

- `crates/app-server/Cargo.toml`
  - 事实：`omne-app-server` 已移除 `diffy` 直依赖；`file/patch` 不再在 app-server 内直接处理 patch 算法。
- `crates/fs-runtime/src/lib.rs`
  - 事实：`file/read|glob|grep|write|patch|edit|delete|fs/mkdir` 均有 `omne-fs-runtime` 封装并委托到 `safe-fs-tools`。
  - 事实：`file/patch` 链路为 `safe-fs-tools::apply_unified_patch`（`diffy` 仅位于更底层依赖）。
- `crates/app-server/src/main/file_read_glob_grep/read.rs`
  - 事实：`file/read` 通过 `spawn_blocking -> omne_fs_runtime::read_text_read_only` 执行。
- `crates/app-server/src/main/file_read_glob_grep/grep.rs`
  - 事实：`file/grep` 通过 `spawn_blocking -> omne_fs_runtime::grep_read_only_paths` 执行。
- `crates/app-server/src/main/file_write_patch.rs`
  - 事实：`file/write`、`file/patch` 分别通过
    `omne_fs_runtime::write_text_workspace`、`omne_fs_runtime::patch_text_workspace` 执行。
- `crates/app-server/src/main/file_edit_delete.rs`
  - 事实：`file/edit`、`file/delete` 分别通过
    `omne_fs_runtime::edit_replace_workspace`、`omne_fs_runtime::delete_path_workspace` 执行。
- `crates/app-server/src/main/fs.rs`
  - 事实：`fs/mkdir` 已下沉到 `spawn_blocking -> omne_fs_runtime::mkdir_workspace`（不再直接 `tokio::fs::create_dir`）。
- `crates/app-server/src/main/thread_observe/disk_git_diff.rs`
  - 事实：`thread/diff` 不是“直接 git-runtime 执行 diff”；`git-runtime` 提供 recipe/limits，实际通过 `process/start` 跑命令并走 artifact 写入管线。
- `crates/app-server/src/main/artifact/write.rs`
  - 事实：artifact 业务逻辑（版本、历史快照、裁剪报告）在 app-server 层显著存在；`omne-artifact-store` 不是唯一实现主体。
- `crates/app-server/src/agent/tools/dispatch/run_tool_call_once.rs`
  - 事实：`web_search/web_fetch/view_image` 主要实现位于 dispatch 层；不是独立 runtime crate。
- `crates/app-server/src/main/mcp/runtime.rs`
  - 事实：MCP 连接管理/调用在该模块实现，与 `web/*` 的 dispatch 实现位置不同。

## B. 内部规范文档

- `docs/research/tools-alignment-todo.md`
  - 事实：已明确“减少无效 tool token、按上下文裁剪”的目标。
- `docs/research/tools-omne.md`
  - 事实：Omne 当前 facade/legacy 工具面、role/mode 关系、默认开关与缺口盘点。
- `docs/tool_parallelism.md`
  - 事实：只读工具并发边界（默认关闭，保守名单）。
- `docs/execpolicy.md`
  - 事实：命令前缀规则、强审批语义、fail-closed。
- `docs/execve_wrapper.md`
  - 事实：当前 execve wrapper 主要覆盖 unix + bash 路径。
- `docs/mcp.md`
  - 事实：MCP 默认关闭、审批与权限链要求。
- `docs/modes.md`
  - 事实：mode gate 与组合顺序写死。
- `docs/workflow_commands.md`
  - 事实：`allowed_tools` 语义是“再收紧”，未知工具 fail-closed。

## C. 对比研究

- `docs/research/tools-codex.md`
  - 事实：Codex 默认 model-facing 工具集更小、按模型/特性动态组装。
- `docs/research/tools-opencode.md`
  - 事实：OpenCode 工具注册与 provider/model 过滤策略可借鉴。

## C1. Role/Mode 共存相关代码事实

- `crates/app-server-protocol/src/lib/thread.rs`
  - 事实：thread configure/state/explain 协议中 `mode` 与 `role` 已并存。
- `crates/core/src/roles.rs`
  - 事实：已新增独立 role catalog（并保留 mode-name 兼容映射）。
- `crates/app-server/src/main/thread_manage/config.rs`
  - 事实：已移除 `role = mode` 隐式回退；role 校验已优先走 role catalog，并保留 mode-name 兼容回退。
  - 事实：`thread/config_explain` 已增加 `role_catalog` 层，输出 `effective_role/permission_mode/resolution_source`。
- `crates/app-server/src/agent/core/run_turn.rs`
  - 事实：router 使用 `thread_mode`，tool schema 裁剪使用 `thread_role`。
  - 事实：每轮先计算 `effective_permissions = mode ∩ role(permission_mode) ∩ allowed_tools`，
    再用于附件解析与 tool schema 过滤。
- `crates/app-server/src/agent/tools/catalog.rs`
  - 事实：`ToolRoleProfile` 已接入工具面裁剪（`Chatter/Default/Codder/Legacy`）。

## D. 外部参考（策略层）

- Git worktree 官方文档（与本提案无直接实现耦合，仅流程参考）：
  - https://git-scm.com/docs/git-worktree
- Claude Code 并行会话 worktree 教程（并发工程实践参考）：
  - https://code.claude.com/docs/en/tutorials#run-parallel-claude-code-sessions-with-git-worktrees

## E. 术语约定（本变更）

- `facade tool`：模型可见的聚合入口工具。
- `internal action`：路由后的既有细粒度动作（如 `file/read`、`process/start`）。
- `help-first`：默认只给最简用法，进阶通过 `op=help` 请求。
- `mainline`：直接在 `main` 持续小步迭代，不维护长期特性分支。

## F. 本轮回归结果（2026-03-02）

- 命令：
  - `cargo test -p omne-app-server -- --nocapture`
- 结果：
  - `448 passed; 0 failed`
