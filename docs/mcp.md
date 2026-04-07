# MCP（Model Context Protocol）

> 目标：把“外部工具生态”接入做成标准接口：可配置、可审计、可回放、可收口。
>
> 状态：v0.2.x 已落地 **MCP client（stdio 最小子集）**；并提供 **实验性 MCP server（stdio）**：`omne mcp serve`（只读 allowlist + 每次调用落盘审计）。
>
> 背景参考：`docs/research/codex.md`、`docs/research/openai-cli-agent.md`、`docs/research/claude-code.md`。
>
> 另：`execve-wrapper` 的 Run/Escalate/Deny 通道可复用 MCP server 思路，见 `docs/execve_wrapper.md`。

---

## 0) 范围与非目标

范围（v1 最小子集；v0.2.x 已实现 client 侧）：

- MCP client：连接一个或多个 MCP servers，获取 tools/resources/prompts，并在 agent loop 中可调用。
- MCP server（实验）：让其它 MCP clients 把 OmneAgent 当作一个工具（暴露受限能力子集）。

非目标（先别碰）：

- marketplace/自动发现/自动安装（风险大、扩面快）。
- 复杂传输栈（HTTP/SSE/WebSocket 统统支持）；先做最小 transport。
- 把 MCP 当成“权限后门”：MCP 不能绕过 `mode/sandbox/execpolicy/approval`。
- 在 v1 承诺“动态注入远端 tool schema”并让模型像本地 tool 一样直接调用（先用 wrapper tool，简单可审计）。

---

## 1) MCP client（连接外部 servers）

### 1.1 默认关闭（写死）

MCP 属于“执行外部二进制 + 任意 side-effect”的高风险能力，v1 建议 **默认关闭**：

- `OMNE_ENABLE_MCP=true` 才允许启用 MCP client（**未启用时不读取配置文件**）。
- 未启用时：
  - 不读取配置文件（即使存在）。
  - `mcp/*` 调用一律 fail-closed 拒绝（`denied=true`），并返回 `error_code="mcp_disabled"`；且不得启动任何外部进程。

### 1.2 配置文件位置与发现顺序（建议写死）

Project config（可提交/可 review）：

- **Canonical**：`./.omne_data/spec/mcp.json`

发现顺序建议与 `docs/modes.md`/`docs/model_routing.md` 保持一致（高 → 低）：

1. env：`OMNE_MCP_FILE`（绝对或相对路径；相对路径按 thread cwd（workspace root）解析）
2. `./.omne_data/spec/mcp.json`
3. 内置默认（空配置：不启用任何 server）

fail-closed（写死）：

- env 指向文件不存在、解析失败、schema 校验失败：应直接报错（不要 silent fallback 到别的文件）。
- 文件存在但内容无效：应直接报错（避免“以为生效但其实没生效”）。

### 1.3 `mcp.json`（v1 最小 schema）

> 只定义必需字段，别发明 DSL。v1 先只支持 `stdio`。

```json
{
  "version": 1,
  "servers": {
    "ripgrep": {
      "transport": "stdio",
      "argv": ["mcp-rg", "--stdio"],
      "env": { "NO_COLOR": "1" }
    }
  }
}
```

字段语义（建议写死；fail-closed）：

- 顶层：
  - `version`：整数，当前固定 `1`。
  - `servers`：map，key 为 `server_name`（稳定标识；建议只用 `[a-zA-Z0-9_-]`）。
  - 未知字段：直接报错（避免 typo 静默）。
- `servers.<name>`：
  - `transport`：v1 只允许 `"stdio"`；其它值直接报错。
  - `argv`：非空数组；每项必须为非空字符串；禁止单字符串 shell 拼接（例如 `"bash -lc ..."`）。
    - `argv[0]` 建议使用绝对路径（减少 PATH 劫持风险）；若不是绝对路径，视为使用当前环境的 `PATH` 查找。
  - `env`：可选，map（key/value 都是 string）；用于向 MCP server 注入环境变量。
    - 仍需要对敏感 env 做 scrub/脱敏（见 `docs/redaction.md`）。
  - 未知字段：直接报错（fail-closed）。

### 1.4 生命周期（最小语义）

建议最小行为：

- thread start/resume 时按需建立连接（lazy）：只有当需要 list/call 某个 server 时才启动/连接。
- resume 不承诺复用旧连接：连接是可重建的缓存，不是权威状态。
- v1 建议每个 thread 独立管理 MCP server 进程（避免跨 thread 共享导致审计/权限边界混乱）。
- stdio MCP server 的启动边界应与 `process/start` 对齐：在真正 `spawn()` 前，按同一套 execution gateway 预处理 `argv/cwd/workspace_root`，避免 `mcp/*` 只复用审批语义却绕过本地执行边界。

事件化（建议）：

- 连接/断开必须可审计：产生 `ToolStarted/ToolCompleted`（或专门的 `Mcp*` 事件；二选一，先别扩协议）。
- MCP server 进程应纳入 Process Registry（本质是一个后台进程）：有 `process_id`，stdout/stderr 落盘（见 `docs/runtime_layout.md`）。
- 同一 server 的请求建议串行化（一个连接上不要并发写 request），避免协议/输出交错导致难以回放。
- stdout/stderr 属于“原始日志”（不保证脱敏），不应被直接注入到模型上下文；若要展示/引用，建议生成脱敏摘要（见 `docs/redaction.md`）。

### 1.5 `mcp/*` 如何进入权限链（写死）

核心约束：MCP tool call **可能产生任意 side-effect**，默认必须走审批。

建议最小落地方式：

- 把 MCP 调用建模成一组本地工具（method 为 slash，tool id 为 snake_case；口径见 `docs/tool_parallelism.md`）：
  - `mcp/list_servers` ↔ `mcp_list_servers`
  - `mcp/list_tools` ↔ `mcp_list_tools`
  - `mcp/list_resources` ↔ `mcp_list_resources`
  - `mcp/call { server, tool, arguments }` ↔ `mcp_call`
- `mcp/*` 先经过 `allowed_tools`、必要的 config/schema 校验与 hard boundary（例如 read-only、network deny、spawn 前 execution gateway 准备）；进入策略合并阶段后，再按 `mode gate → execpolicy → approval handling` 继续（见 `docs/modes.md`/`docs/approvals.md`）。
  - mode：v1 建议默认对 `mcp/*` 采取 `prompt`（或直接 `deny`），避免默认放权。
  - approval：启用 MCP 时强烈建议 `ApprovalPolicy=manual`。
  - `prompt_strict`（已实现；见 `docs/approvals.md`）：v0.2.x 中 `mcp/call` 默认写入 `approval.requirement="prompt_strict"`，并且不可被 `remember` 自动复用。
  - 被拒绝时返回稳定 `error_code`，便于客户端自动分类（例如 `allowed_tools_denied`、`mode_denied`、`sandbox_policy_denied`、`sandbox_network_denied`、`execpolicy_denied`、`execpolicy_load_denied`、`approval_denied`）。
- 当 `mcp/*` 需要 lazy 启动 stdio server 时，server 进程本身也属于这条治理链的一部分：本地侧既要判定“是否允许调用该 MCP 能力”，也要在 spawn 前执行 execution gateway 的边界检查与命令准备。

v0.2.x 实现口径（最小）：

- app-server JSON-RPC：
  - `mcp/list_servers`
  - `mcp/list_tools`
  - `mcp/list_resources`
  - `mcp/call`（默认 `prompt_strict`）
- agent tools（snake_case）：
  - `mcp_list_servers`
  - `mcp_list_tools`
  - `mcp_list_resources`
  - `mcp_call`
- CLI：
  - `omne mcp list-servers <thread_id>`
  - `omne mcp list-tools <thread_id> <server>`
  - `omne mcp list-resources <thread_id> <server>`
  - `omne mcp call <thread_id> <server> <tool> --arguments-json '<json>'`

> 注意：MCP 不能绕过 execpolicy。即使 MCP server 内部执行命令，也必须在它自己的侧做 policy；本地侧只能保证“我们何时允许调用它”。
>
> 如果未来落地 `execve-wrapper`，推荐把 “execve 决策服务”实现为一个本地 MCP server（见 `docs/execve_wrapper.md`），复用 transport 与审计口径。

### 1.6 大结果的落盘（避免把 payload 塞进事件）

建议：

- `mcp/call` 的结果如果超过阈值（例如 256KiB），写入 artifact（`artifact_type="mcp_result"`），`ToolCompleted.result` 只返回 `artifact_id/path/summary`。
- 对返回文本做脱敏（见 `docs/redaction.md`）。

---

## 2) MCP server（让外部把 OmneAgent 当工具）（实验）

目标：

- 让其它 MCP clients 调用 OmneAgent 的受限能力子集（例如：thread list/attention、artifact list/read、process tail）。

最小原则：

- 默认只绑定 loopback（或本地 stdio），不对公网暴露。
- 外部调用仍要落盘成事件（可回放/可审计）。
- 暴露的工具集合必须是显式 allowlist（不提供“任意 JSON-RPC 转发”）。
- 如未来支持网络传输：必须引入认证（例如 bearer token）与 host/port 绑定策略；默认不启用。

建议暴露的最小工具子集（占位）：

- `omne.thread.list_meta`
- `omne.thread.attention`
- `omne.thread.subscribe`
- `omne.artifact.list/read`
- `omne.process.list/inspect/tail/follow`

### 2.1 v0.2.x 现状：`omne mcp serve`（stdio）

最小用法（由外部 MCP client 启动该进程并通过 stdio 交互）：

```bash
omne mcp serve
```

审计：

- 默认启用：每次 `tools/call` 都会向一个 audit thread 写入一条 `artifact_type="mcp_server_call"`（从而产生可回放事件）。
- 可用 `--audit-thread-id <thread_id>` 复用已有 thread；或 `--no-audit` 关闭审计（不推荐）。

暴露的工具（只读 allowlist；通过 `tools/list` 返回）：

- `omne.thread.list_meta`
- `omne.thread.attention`
- `omne.thread.state`
- `omne.thread.events`
- `omne.artifact.list`
- `omne.artifact.read`
- `omne.process.list`
- `omne.process.inspect`
- `omne.process.tail`
- `omne.process.follow`

参数补充（当前实现）：

- `omne.thread.events` 支持可选 `kinds: string[]`，用于按事件类型过滤（例如 `["attention_marker_set","attention_marker_cleared"]`）。
- `omne.thread.events` 的 MCP `inputSchema` 现已给出 `kinds.items.enum`（与 Rust 协议层共享同一枚举源），便于客户端在调用前校验。
- `omne.thread.events.kinds` 中若包含未知事件类型，服务端会返回 `invalid params`，并在错误数据里附带 `supported_kinds`。
- CLI 侧 `omne thread events --kind <type>` 也使用同一份事件类型枚举源做参数校验，非法值会在本地直接报错。
- `omne.thread.attention` 响应包含 marker 布尔摘要：`has_plan_ready` / `has_diff_ready` / `has_fan_out_linkage_issue` / `has_test_failed`；并返回 `attention_markers.*` 详情（当 marker 存在时）。
- `omne.thread.list_meta` 每个 thread 行同样包含上述 `has_*` 布尔摘要；当 `include_attention_markers=true` 时还会返回 `attention_markers` 对象。

---

## 3) 验收（未来实现时）

MCP client：

- 指定 `./.omne_data/spec/mcp.json` 后，`omne` 能列出 servers 与 tools/resources。
- 调用 `mcp/call` 时必须产生审批事件（`ApprovalRequested`），且可在 `omne inbox` 中看到阻塞。
- MCP server 进程 stdout/stderr 必须落盘并可 `process/tail`。

MCP server：

- 外部 MCP client 能调用 `omne.thread.list_meta` 并得到结果。
- 所有外部调用都能在本地 thread 事件里回放（审计链完整）。
