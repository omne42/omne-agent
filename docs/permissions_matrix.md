# 用户可选权限与实际生效特征（v0.2.x）

这份文档只描述一件事：用户能设置哪些权限，以及这些配置在当前实现里到底会怎样生效。

## 0) 最外层（简版）到底暴露什么

面向最终用户，最外层只建议暴露 5 个选项（其中 `mode` 必填）：

| 选项 | 可选值 | 必填 | 作用（用户视角） |
|---|---|---|---|
| `mode` | 内置/项目自定义 mode 名 | 是 | 选择“场景能力边界”（代码/评审/文档等）。 |
| `sandbox_policy` | `read_only` / `workspace_write` / `full_access` | 否 | 控制文件系统写入范围。`full_access` = 最大权限。 |
| `approval_policy` | `auto_approve` / `manual` / `unless_trusted` / `auto_deny` | 否 | 控制高风险动作是否自动执行或必须审批。 |
| `sandbox_network_access` | `deny` / `allow` | 否 | 控制命令执行链路里的“潜在联网命令”检测与阻断。当前实现会 best-effort 拦截已知网络客户端、`git` 的明确网络子命令，以及常见包管理器/工具链入口（如 `npm install`、`pip install`、`cargo install`、`go get`）和显式 eval/shell 包装形式；`mcp` stdio server 启动会额外对 generic launcher 和路径执行做更保守的 fail-closed 处理，并与 `process/start` 共用 execution gateway 的命令边界准备。它不是内核级网络沙箱。 |
| `role` | role catalog 名称 | 否 | 身份语义，只做降权，不会放大 `mode` 权限。 |

最外层不建议直接暴露的高级选项：

- `allowed_tools`
- `execpolicy_rules`
- `sandbox_writable_roots`

它们主要用于高级/调试场景，默认应由 `mode + role + 系统策略` 自动决定。

### 0.1 推荐给用户的三档安全预设

| 预设 | `sandbox_policy` | `approval_policy` | `sandbox_network_access` | 适用场景 |
|---|---|---|---|---|
| 保守 | `read_only` | `manual` | `deny` | 只读分析、审计代码。 |
| 默认 | `workspace_write` | `unless_trusted` | `deny` | 日常开发，风险与效率平衡。 |
| YOLO | `full_access` | `auto_approve` | `allow` | 本地全开，完全信任执行。 |

## 0.2 顶层与高级（实现口径）

参考 `policy-meta-spec`，当前实现中的顶层安全字段为：

- `mode`
- `sandbox_policy`
- `approval_policy`
- `sandbox_network_access`

高级字段（可选）：

- `role`
- `allowed_tools`
- `execpolicy_rules`
- `sandbox_writable_roots`

## 1) 配置入口

以下入口最终都落到 `thread/configure`（同一套语义）：

- RPC：`thread/configure`
- CLI：`omne thread configure ...`
- CLI REPL/TUI 内联配置（本质也是调用 `thread/configure`）
- workflow/preset 导入时写入 thread 配置

建议用 `thread/config-explain` 观察“最终生效值 + 来源分层”。

最小可复制配置示例：

```toml
# 保守（推荐给新用户）
mode = "code"
sandbox_policy = "read_only"
approval_policy = "manual"
sandbox_network_access = "deny"
```

```toml
# 默认（日常开发）
mode = "code"
sandbox_policy = "workspace_write"
approval_policy = "unless_trusted"
sandbox_network_access = "deny"
```

```toml
# YOLO（全开）
mode = "code"
sandbox_policy = "full_access"
approval_policy = "auto_approve"
sandbox_network_access = "allow"
```

## 2) 权限字段总表

> 默认值来自 `thread/config-explain` 当前实现。

| 字段 | 可选值 | 默认值 | 实际生效特征 |
|---|---|---|---|
| `approval_policy` | `auto_approve` / `manual` / `unless_trusted` / `auto_deny` | `auto_approve` | 控制 `prompt`/`prompt_strict` 场景下是否自动放行、阻塞等待、或自动拒绝。所有决策都会落盘 approval 事件。 |
| `sandbox_policy` | `read_only` / `workspace_write` / `full_access` | `workspace_write` | 控制路径边界与写入能力。`full_access` 放宽文件系统边界，但不绕过命令执行治理。 |
| `sandbox_writable_roots` | 路径数组 | `[]` | 在非 `full_access` 时，为“绝对路径写入”增加允许根目录。相对路径写入仍按 workspace 解析。 |
| `sandbox_network_access` | `deny` / `allow` | `deny` | 控制命令类网络访问闸门（`process/start`、`execve gate`、`mcp/call`）。当前实现会 best-effort 拒绝已知网络客户端、`git` 的明确网络子命令，以及常见包管理器/工具链入口（如 `npm install`、`pip install`、`cargo install`、`go get`）和显式 eval/shell 包装形式；`mcp` stdio server 启动额外对 generic launcher 和路径执行做更保守的 fail-closed 处理，并与 `process/start` 共用 execution gateway 的命令边界准备。它不是内核级网络命名空间隔离，也不直接限制 `web/*` 工具。 |
| `mode` | mode 名称（builtin + `.omne_data/spec/modes.yaml`） | `code` | 主权限边界：`read/edit/command/process/artifact/browser/subagent` + `tool_overrides`。`deny` 是硬拒绝。运行时始终有 mode。 |
| `role` | role 名称（仅 role catalog） | `coder` | 可选降权层。只会收紧 mode（不放大）。不再把自定义 mode 名当作 role。 |
| `allowed_tools` | `null`（清空）/ `[]` / 工具列表 | `null`（不额外收口） | 线程级工具白名单。设置后，不在列表里的工具直接拒绝。`[]` 等价 deny-all。 |
| `execpolicy_rules` | 规则文件路径列表 | `[]` | 命令前缀策略，和 global/mode 规则叠加；可给出 `allow/prompt/prompt_strict/forbidden`。 |

## 3) 关键语义细节

### 3.1 `allowed_tools` 是“再收口”

- `allowed_tools = null`：不启用白名单收口（保持 mode/sandbox/execpolicy 规则）。
- `allowed_tools = []`：显式拒绝所有工具。
- 配置时会 fail-closed：
  - 包含未知工具名会报错。
  - 工具若被当前 `mode` 或 `role(permission_mode)` 判定为 `deny`，也会报错。

### 3.2 `role` 不是独立工具闸门

- `role` 与 `mode` 解耦：role 只从 role catalog 解析，不走 mode-compat。
- `role` 只做降权（downscope）：语义上只能收紧 mode。
- 运行时主链路仍是 `mode + allowed_tools + sandbox + execpolicy + approval`；
  `role` 主要通过“permission_mode”参与可用权限计算与收口。

### 3.3 `sandbox_writable_roots` 的生效边界

- 只在非 `full_access` 且写入场景生效。
- 只扩展绝对路径写入根；读取和相对路径写入不走这条扩展路径。
- 配置时会做路径解析与去重，非法路径直接拒绝。

### 3.4 `execpolicy_rules` 的叠加来源

命令策略最终由三层叠加得到：

1. server 全局 `--execpolicy-rules`
2. mode 的 `command.execpolicy_rules`
3. thread 的 `execpolicy_rules`

`forbidden` / `prompt_strict` 会优先触发拒绝或强审批。

## 4) `full_access` 的边界

`sandbox_policy = full_access` 只影响文件系统边界，不会让命令执行链路跳过治理。

这意味着：

- `process/start` 仍受 `allowed_tools`、`sandbox_network_access`、mode、execpolicy、approval 约束。
- `process/execve gate` 仍按同一套治理链路裁决 `run/deny/escalate`。
- `mcp/*` 仍受各自原有门禁；如果 lazy 启动 stdio server，启动命令本身也要经过 execution gateway 边界准备。
- 其他工具（如 `file/*`、`artifact/*`、`repo/*`）也继续走各自原有门禁。

## 5) 推荐排障方式

- 查看最终生效配置：`omne thread config-explain <thread_id>`
- 关注返回中的：
  - `effective.*`（最终值）
  - `layers[]`（来源分层）
  - `permission_mode` 与 `effective_permissions`
  - 工具拒绝的 `error_code`（`allowed_tools_denied` / `mode_denied` / `sandbox_*` / `execpolicy_*` / `approval_denied` 等）

## 6) `allowed_tools` 可选值（当前内置清单）

```text
facade/workspace
facade/process
facade/thread
facade/artifact
facade/integration
file/read
file/glob
file/grep
file/write
file/patch
file/edit
file/delete
fs/mkdir
repo/search
repo/index
repo/symbols
mcp/list_servers
mcp/list_tools
mcp/list_resources
mcp/call
artifact/write
artifact/list
artifact/read
artifact/delete
thread/request_user_input
thread/diff
thread/state
thread/usage
thread/events
thread/hook_run
web/search
web/fetch
web/view_image
subagent/spawn
subagent/send_input
subagent/wait
subagent/close
process/start
process/list
process/inspect
process/kill
process/interrupt
process/tail
process/follow
```
