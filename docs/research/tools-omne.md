# Omne Tooling (Current Code Reality, 2026-03-02)

## Scope
本文件记录 `omne-agent` 当前真实代码口径下的工具面、裁剪策略、role/mode 关系和已知缺口。

Primary sources:
- `/root/autodl-tmp/zjj/p/omne-agent/crates/app-server/src/agent/tools/catalog.rs`
- `/root/autodl-tmp/zjj/p/omne-agent/crates/app-server/src/agent/tools/spec.rs`
- `/root/autodl-tmp/zjj/p/omne-agent/crates/app-server/src/agent/tools/dispatch/run_tool_call_once.rs`
- `/root/autodl-tmp/zjj/p/omne-agent/crates/core/src/allowed_tools.rs`
- `/root/autodl-tmp/zjj/p/omne-agent/crates/core/src/modes.rs`
- `/root/autodl-tmp/zjj/p/omne-agent/crates/app-server/src/main/thread_manage/config.rs`
- `/root/autodl-tmp/zjj/p/omne-agent/crates/app-server/src/agent/core/run_turn.rs`
- `/root/autodl-tmp/zjj/p/omne-agent/crates/app-server/src/main/fs.rs`

## Code Reality Corrections (2026-03-04)

以下为近期实现演进后的准确口径（用于覆盖旧认知）：

1. `file/patch` 不再由 `omne-app-server` 直连 `diffy` 实现。
   - 当前链路：`app-server -> omne-fs-runtime -> safe-fs-tools::apply_unified_patch`。
   - `diffy` 仅存在于 `safe-fs-tools` 更底层实现中。
   - 参考：`crates/app-server/Cargo.toml`、`crates/fs-runtime/src/lib.rs`。

2. `omne-fs-runtime` 不再只服务 `file/glob`。
   - 当前 `file/read|glob|grep|write|patch|edit|delete|fs/mkdir` 均通过 `omne-fs-runtime` 调用 `safe-fs-tools`。
   - 参考：`crates/app-server/src/main/file_read_glob_grep/read.rs`、`crates/app-server/src/main/file_read_glob_grep/grep.rs`、`crates/app-server/src/main/file_write_patch.rs`、`crates/app-server/src/main/file_edit_delete.rs`。

3. `fs/mkdir` 已下沉到 `omne-fs-runtime`。
   - 当前链路：`app-server -> omne-fs-runtime::mkdir_workspace -> safe-fs-tools::mkdir`。
   - 参考：`crates/app-server/src/main/fs.rs`、`crates/fs-runtime/src/lib.rs`。

4. `thread/diff` 不是“直接由 git-runtime 执行 diff 命令”。
   - `omne-git-runtime` 提供 recipe/limits 约束，实际通过 `process/start` 跑命令，再经 artifact 管线落盘输出。
   - 参考：`crates/app-server/src/main/thread_observe/disk_git_diff.rs`。

5. `artifact` 不能简化为“主要由 omne-artifact-store 实现”。
   - `omne-artifact-store` 提供底层存储能力，但版本、历史快照、裁剪报告等大量业务逻辑在 `app-server`。
   - 参考：`crates/app-server/src/main/artifact/write.rs`、`crates/app-server/src/main/fs.rs`。

6. `integration` 中 MCP 描述正确，但 `web/*` 主实现位置需明确。
   - `web_search/web_fetch/view_image` 主要在 `run_tool_call_once.rs` 内实现，不是独立 runtime crate。
   - 参考：`crates/app-server/src/main/mcp/runtime.rs`、`crates/app-server/src/agent/tools/dispatch/run_tool_call_once.rs`。

## 1) 全量工具面（代码定义）

### 1.1 Legacy 细粒度工具（35）

`build_tools()` 定义了 35 个 legacy tools：

1. `file_read`
2. `file_glob`
3. `file_grep`
4. `repo_search`
5. `repo_index`
6. `repo_symbols`
7. `mcp_list_servers`
8. `mcp_list_tools`
9. `mcp_list_resources`
10. `mcp_call`
11. `file_write`
12. `file_patch`
13. `file_edit`
14. `file_delete`
15. `fs_mkdir`
16. `process_start`
17. `process_inspect`
18. `process_tail`
19. `process_follow`
20. `process_kill`
21. `artifact_write`
22. `update_plan`
23. `request_user_input`
24. `web_search`
25. `webfetch`
26. `view_image`
27. `artifact_list`
28. `artifact_read`
29. `artifact_delete`
30. `thread_diff`
31. `thread_state`
32. `thread_usage`
33. `thread_events`
34. `thread_hook_run`
35. `agent_spawn`

### 1.2 Facade 聚合工具（5）

`build_facade_tools()` 定义了 5 个 facade tools：

1. `workspace`：`read/glob/grep/repo_search/repo_index/repo_symbols/write/patch/edit/delete/mkdir/help`
2. `process`：`start/inspect/tail/follow/kill/help`
3. `thread`：`diff/state/usage/events/hook_run/request_input/spawn_agent/send_input/wait/close(/close_agent)/help`
4. `artifact`：`write/update_plan/list/read/delete/help`
5. `integration`：`mcp_list_servers/mcp_list_tools/mcp_list_resources/mcp_call/web_search/web_fetch/view_image/help`

`run_tool_call_once.rs` 中 `facade_route(...)` 显式定义了 `facade op -> legacy tool -> internal action` 映射。

## 2) 默认模型暴露面（真实默认值）

默认开关组合：
- `OMNE_TOOL_FACADE_ENABLED=true`
- `OMNE_TOOL_FACADE_EXPOSE_LEGACY=false`
- `OMNE_ENABLE_MCP=false`
- `OMNE_TOOL_EXPOSE_WEB=false`
- `OMNE_TOOL_EXPOSE_SUBAGENT=false`
- `OMNE_TOOL_EXPOSE_THREAD_INTROSPECTION=false`
- `OMNE_TOOL_EXPOSE_THREAD_HOOK=false`
- `OMNE_TOOL_EXPOSE_REPO_SYMBOLS=false`

在默认组合下，模型侧通常只看到 4 个工具：
- `workspace`
- `process`
- `thread`
- `artifact`

`integration` 只有在 `mcp_enabled || expose_web` 时才出现。

实测（`tool_catalog_tests::facade_tool_surface_reduces_schema_bytes_vs_legacy_default`）：
- legacy: `count=21`, `bytes=7669`
- facade: `count=4`, `bytes=1815`
- schema bytes 降幅：约 `76.33%`

## 3) 过滤/裁剪逻辑（每轮动态）

`build_tools_for_turn(...)` 的筛选维度：

1. `allowed_tools`（按 internal action 过滤；facade 会按内部 actions 交集保留）
2. 环境开关（MCP/Web/Subagent/Thread introspection 等）
3. 模型档位 `ToolModelProfile`
4. 角色档位 `ToolRoleProfile`
5. 动态 registry（`OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED`，默认关闭）

模型档位：
- `full`：不过滤
- `compact`：隐藏 `repo_index`、`process_follow`
- 自动 compact 触发：model slug 包含 `mini` / `flash` / `haiku`
- 可通过 `OMNE_TOOL_MODEL_PROFILE=full|compact` 覆盖

角色档位：
- `Chatter`：只保留读与轻交互，不允许写文件/启动进程/删 artifact/MCP call
- `Default`：允许读、artifact 写、process inspect/tail/follow；禁止写文件和危险变更
- `Codder/Coder`：保留编码全能力
- `Legacy`：不做角色裁剪（回退行为）

注：`ToolRoleProfile::from_role` 对输入 role 名做映射，未知 role 回退到 `Legacy`。

## 4) Role 与 Mode 当前关系

当前系统中 `mode` 和 `role` 已并存于 thread 配置协议：
- `ThreadConfigureParams` 同时支持 `mode` 与 `role`
- `ThreadStateResponse` / `ThreadConfigExplainResponse` 同时返回两者

当前实现语义：
1. `mode` 主要用于权限决策与路由（例如 router 里按 `thread_mode` 选模型角色）
2. `role` 主要用于工具暴露裁剪（`build_tools_for_turn(..., role)`）
3. `thread/configure` 已移除 `role = mode` 隐式回退（仅更新 `mode` 不会覆盖 `role`）
4. role 校验已优先走独立 `RoleCatalog`，并保留 mode-name 兼容回退

结论：运行时权限、工具暴露与 explain 口径已按 `mode`/`role(permission_mode)` 显式正交；仍保留 mode-name 兼容回退路径以保证历史配置兼容。

## 5) 安全链路是否降级

没有降级。facade 不绕过 legacy 安全链：

1. facade 先映射到明确 internal action
2. 复用既有 handler 执行路径
3. 继续走 `allowed_tools -> hard boundary/config validation -> mode -> execpolicy -> approval`
4. facade wrapper 事件写入 `facade_tool/op/mapped_action`

稳定错误码：
- `facade_invalid_params`
- `facade_unsupported_op`
- `facade_policy_denied`

## 6) 相比 Codex/OpenCode 的主要缺口

1. 动态工具注册当前为 MVP（仅 read-only mapped tools，默认关闭），插件化与写操作治理未完成。
2. role/mode 仍保留 mode-name 兼容回退路径（兼容优先，不是严格拆分开关形态）。
3. provider/model 维度的裁剪策略仍较轻量（仅 full/compact + 少量启发式）。

## 7) 开关策略建议（当前建议值）

建议保持：
- `OMNE_TOOL_FACADE_ENABLED=true`（默认开）
- `OMNE_TOOL_FACADE_EXPOSE_LEGACY=false`（默认关）
- `OMNE_ENABLE_MCP=false`（默认关）
- `OMNE_TOOL_EXPOSE_WEB=false`（默认关）
- `OMNE_TOOL_EXPOSE_SUBAGENT=false`（默认关）
- `OMNE_TOOL_EXPOSE_THREAD_INTROSPECTION=false`（默认关）
- `OMNE_TOOL_EXPOSE_THREAD_HOOK=false`（默认关）
- `OMNE_TOOL_EXPOSE_REPO_SYMBOLS=false`（默认关）
- `OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED=false`（默认关，按需开启）
- `OMNE_TOOL_MODEL_PROFILE=auto(full/compact)`（默认自动）
