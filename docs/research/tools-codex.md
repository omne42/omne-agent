# Codex Tooling (example/codex, Current Code Reality)

## Scope
本文件记录 `example/codex` 的工具组装、默认暴露和动态扩展机制。

Primary sources:
- `/root/autodl-tmp/zjj/p/example/codex/codex-rs/core/src/tools/spec.rs`
- `/root/autodl-tmp/zjj/p/example/codex/codex-rs/core/src/tools/handlers/plan.rs`
- `/root/autodl-tmp/zjj/p/example/codex/codex-rs/core/src/tools/handlers/apply_patch.rs`
- `/root/autodl-tmp/zjj/p/example/codex/codex-rs/core/src/models_manager/model_info.rs`
- `/root/autodl-tmp/zjj/p/example/codex/codex-rs/core/src/features.rs`

## 1) 工具不是固定清单，而是动态组装

`build_specs(...)` 会基于 `ToolsConfig` 动态加入 tool specs 和 handlers。`ToolsConfig` 输入至少包含：
- model 元数据（`ModelInfo`）
- 功能开关（`Features`）
- `web_search_mode`

这意味着 Codex 的默认工具面会随模型、特性、运行配置变化。

## 2) 工具族与启用条件

### 2.1 Shell 家族（互斥分支，四选一）

按 `ConfigShellToolType` 只会选一条主路径：
- `shell`
- `local_shell`
- `exec_command` + `write_stdin`
- `shell_command`

此外，会注册 shell alias handlers 以兼容旧 prompt。

### 2.2 常驻核心工具（builder 里总是加入）

1. `list_mcp_resources`
2. `list_mcp_resource_templates`
3. `read_mcp_resource`
4. `update_plan`
5. `view_image`

### 2.3 条件工具

1. `request_user_input`
条件：`collaboration_modes_tools=true`

2. `apply_patch`
条件：模型或 feature 指示支持（`Freeform` 或 `Function` 形态）

3. `grep_files` / `read_file` / `list_dir` / `test_sync_tool`
条件：出现在 `experimental_supported_tools`

4. `web_search`
条件：`web_search_mode` 为 `Cached` 或 `Live`

5. `spawn_agent` / `send_input` / `wait` / `close_agent`
条件：`collab_tools=true`

### 2.4 动态扩展入口（第一类能力）

1. MCP server tools：运行时把 MCP schema 转换后附加到最终工具集
2. `dynamic_tools`：运行时注入并转换成 OpenAI tool schema

这两项使 Codex 具备“静态内置 + 动态外扩”的统一工具面。

## 3) 默认行为证据（测试）

`gpt-5-codex` 默认测试集合（`test_build_specs_gpt5_codex_default`）中，工具为：
1. `shell_command`
2. `list_mcp_resources`
3. `list_mcp_resource_templates`
4. `read_mcp_resource`
5. `update_plan`
6. `request_user_input`
7. `apply_patch`
8. `web_search`
9. `view_image`

要点：默认集合明显偏小且高频，便于缓存和工具选择稳定。

## 4) 与工具相关的 feature 默认值

`features.rs` 中典型默认：
- `Collab=false`
- `CollaborationModes=true`
- `UnifiedExec=false`
- `ShellTool=true`

这些默认值直接决定是否出现协作工具、统一执行工具等。

## 5) 模型元数据驱动工具策略

`model_info.rs` 会按 model slug 指定工具偏好，例如：
- `gpt-5-codex`：`apply_patch=Freeform`，`shell_type=ShellCommand`
- `codex-mini-latest`：`shell_type=Local`
- 测试模型可打开更多实验工具（`grep_files/read_file/list_dir/test_sync_tool`）

## 6) 安全与执行约束

1. `exec_command` schema 含审批升级字段（`sandbox_permissions`、`justification`、`prefix_rule`）。
2. `apply_patch` handler 在执行前会做 patch grammar 校验。
3. `update_plan` 是状态工具，且在 Plan mode 存在保护行为。

## 7) 对 Omne 的可借鉴点

1. 工具组装与模型能力紧耦合，减少“无效曝光”。
2. 动态工具（MCP + runtime dynamic）是第一类能力，不是旁路。
3. 子代理生命周期工具是完整闭环（spawn/send/wait/close）。
