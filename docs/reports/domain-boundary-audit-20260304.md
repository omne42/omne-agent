# omne-agent 领域边界审计（2026-03-04）

## 状态更新（2026-03-26）

本报告成文后，`omne-agent` 已有一部分跨仓收口落地。阅读本报告时请结合以下现状：

- 本报告原文中的 `agent-exec-gateway` 现在统一按当前仓内名称理解为 `omne-execution-gateway`。
- `config-kit` 已落地到项目配置与 router 配置边界：
  - `crates/app-server/src/project_config.rs`
  - `crates/core/src/router.rs`
  这意味着 `omne-agent` 已不再手写 project config / router config 的底层文件读取与格式识别。
- `omne-execution-gateway` 已接入 `process/start` 路径：
  - `crates/app-server/Cargo.toml`
  - `crates/app-server/src/main/process_stream/common.rs`
  - `crates/app-server/src/main/process_control/start.rs`
  因此，下文“app-server 依赖中未接入执行网关”的证据已过时。当前剩余问题是 `execve_gate` 与 app 层治理链仍未完全收口。
- `mcp-kit` 目前已复用 `Config::load(...)` 处理 `mcp.json` 配置输入边界，但连接缓存、spawn、initialize、request/notify 仍在 `crates/app-server/src/main/mcp/runtime.rs` 本地实现，因此 MCP 结论仍然成立。
- `policy-meta-spec` 的复用已不止 `WriteScope`；当前还通过 `policy_meta::ExecutionIsolation` 接入执行隔离语义，但整体边界判断没有变化。

## 范围
本次审计检查 `omne-agent` 是否超出自身编排层职责，侵占或重复以下组件领域：

- `omne-execution-gateway`
- `ditto-llm`
- `mcp-kit`
- `notify-kit`
- `safe-fs-tools`
- `policy-meta-spec`

审计维度：`领域侵占`、`职责重复`、`抽象重复`、`代码重复`。

## 结论总览

| 领域 | 结论 | 严重性 | 类型 |
|---|---|---|---|
| `omne-execution-gateway` | 已接入网关预检与命令准备，但仍存在部分重复实现（app 层治理链与 `execve_gate` 未完全收口） | 中-高 | 领域侵占 + 职责重复 |
| `mcp-kit` | 已复用配置加载，但 `Manager`/`Session` 层仍存在明显重复实现（连接缓存、spawn、initialize、request） | 高 | 职责重复 + 抽象重复 |
| `ditto-llm` | 仍有 provider/协议细节残留在 `omne-agent` | 中 | 领域边界偏厚 |
| `safe-fs-tools` | `.env` 安全规则双重实现且有语义漂移风险 | 中 | 代码重复 + 规则重复 |
| `notify-kit` | 边界基本清晰（以调用为主） | 低/无 | 无明显侵占 |
| `policy-meta-spec` | 当前复用 `WriteScope` 与部分 `ExecutionIsolation` 语义，边界基本清晰 | 低/无 | 无明显侵占 |

---

## 1) `omne-execution-gateway`：已接入但仍未完全收口

### 观察
`omne-execution-gateway` 的核心定位是统一命令执行边界与 fail-closed 策略。当前 `omne-agent` 已在 `process/start` 路径接入网关的 `preflight` / `prepare_command`，但 `process/start` 与 `execve_gate` 仍保留了一层并行的 app 侧治理与协议循环，因此边界还没有完全收口。

### 证据
- `app-server` 当前已显式依赖 `omne-execution-gateway`：  
  `crates/app-server/Cargo.toml:12-20`
- `omne-agent` 已在 `process_stream/common.rs` 内构建 `ExecGateway`，并通过 `preflight` / `prepare_command` 执行网关边界检查：  
  `crates/app-server/src/main/process_stream/common.rs:26-90`
- `process/start` 启动真实进程前会调用 `prepare_process_exec_gateway_command(...)`：  
  `crates/app-server/src/main/process_control/start.rs:300-323`
- `omne-agent` 在 `process/start` 内部自行执行 sandbox/network/mode/execpolicy/approval 决策链：  
  `crates/app-server/src/main/process_control/start.rs:83-113`  
  `crates/app-server/src/main/process_control/start.rs:127-230`  
  `crates/app-server/src/main/process_control/start.rs:245-284`
- `omne-agent` 还在 `execve_gate` 里再次实现一套 exec 决策逻辑（含 mode/execpolicy/approval）：  
  `crates/app-server/src/main/process_control/execve_gate.rs:378-543`
- 且 `execve_gate` 自建了 JSON-RPC/MCP 协议处理循环：  
  `crates/app-server/src/main/process_control/execve_gate.rs:88-172`

### 风险
- 执行边界已经不再是“完全双栈”，但 app 层治理链与 gateway 语义仍可能继续漂移。
- bug 修复/策略升级仍需要同时检查 gateway 接入层与 `execve_gate` 本地逻辑。
- 如果后续继续在 app 层堆命令治理逻辑，会重新把已收口的执行边界拉回分散状态。

---

## 2) `mcp-kit`：高风险重复

### 观察
`mcp-kit` 已经接管了 `mcp.json` 配置输入边界，但 `Manager + Session` 这层仍未收口。也就是说，配置加载已经共享，连接缓存、spawn/connect、initialize、request/notify 仍主要由 `omne-agent` 自己维护。

### 证据
- `omne-agent` 当前已通过 `omne_mcp_kit::Config::load(...)` 复用 `mcp-kit` 的配置加载：  
  `crates/app-server/src/main/mcp/runtime.rs:8-10`  
  `crates/app-server/src/main/mcp/runtime.rs:65-69`
- `mcp-kit` 对外职责说明：  
  `mcp-kit/crates/mcp-kit/src/lib.rs:6-9`
- `Manager` 持有连接与初始化结果缓存：  
  `mcp-kit/crates/mcp-kit/src/manager/mod.rs:171-174`
- `Manager` 覆盖 stdio/unix/streamable_http 建连：  
  `mcp-kit/crates/mcp-kit/src/manager/mod.rs:699-788`
- `Manager` 内置 initialize + notifications/initialized：  
  `mcp-kit/crates/mcp-kit/src/manager/mod.rs:1900-1985`
- `omne-agent` 自定义 `McpManager/McpConnection`：  
  `crates/app-server/src/main/mcp/runtime.rs:77-85`
- `omne-agent` 自建 spawn + initialize + notify：  
  `crates/app-server/src/main/mcp/runtime.rs:112-257`
- `omne-agent` 自建连接缓存：  
  `crates/app-server/src/main/mcp/runtime.rs:259-295`
- 且将 transport 限制为 stdio：  
  `crates/app-server/src/main/mcp/runtime.rs:121-123`

### 风险
- 与 `mcp-kit` 的安全策略（如 trust mode）和协议细节更新产生偏差。
- 重复维护连接生命周期、超时、初始化兼容逻辑。
- MCP 相关 bug/优化不能自然复用 `mcp-kit`。

---

## 3) `ditto-llm`：中风险边界偏厚

### 观察
`omne-agent` 已大量使用 `ditto-llm`，但仍保留了一些 provider/协议层细节拼装，尚未完全收敛为“编排层只组装通用请求”。

### 证据
- `omne-agent` 侧按 capability 分支构造不同客户端（OpenAI vs OpenAICompatible）：  
  `crates/app-server/src/agent/core/preamble.rs:1092-1110`
- `omne-agent` 在工具循环中手动拼 `ProviderOptions`（含 `prompt_cache_key`、`parallel_tool_calls` 等）：  
  `crates/app-server/src/agent/tool_loop/core.rs:339-349`
- `omne-agent` 手动设置 `user` 以影响 cache stickiness：  
  `crates/app-server/src/agent/tool_loop/core.rs:373-377`
- Codex parity 分支直接构造 `OpenAIResponsesRawRequest`（`include`、`prompt_cache_key` 等）：  
  `crates/app-server/src/agent/tool_loop/openai_responses_loop.rs:594-609`
- 与之对应，`ditto-llm` 已有 typed `ProviderOptions` 与 bucket 机制：  
  `ditto-llm/src/types/mod.rs:258-267`  
  `ditto-llm/src/types/mod.rs:276-320`
- `ditto-llm` 也已承担 provider 配置/quirks 路由：  
  `ditto-llm/src/profile/openai_providers.rs:238-252`  
  `ditto-llm/src/profile/openai_providers.rs:254-367`

### 风险
- provider 细节回流到 `omne-agent`，长期会重新出现 if/else 扩散。
- 兼容修复点分散在两层，定位复杂。

---

## 4) `safe-fs-tools`：中风险重复与规则漂移

### 观察
`.env` 读取/写入保护同时在 `safe-fs-tools` 与 `omne-agent` 预检中出现，且规则颗粒度不完全一致。

### 证据
- `safe-fs-tools` 的 read 明确拒绝敏感 `.env`：  
  `safe-fs-tools/src/ops/read.rs:54-58`  
  `safe-fs-tools/src/ops/read.rs:96-115`
- `omne-agent` 的 `file/read` 也做了同类拒绝：  
  `crates/app-server/src/main/file_read_glob_grep/read.rs:65-90`  
  `crates/app-server/src/main/file_read_glob_grep/read.rs:151-158`
- `safe-fs-tools` 默认 secret deny 含 `.env.*`：  
  `safe-fs-tools/src/policy.rs:286-294`
- 但 `omne-agent` 的 `is_secret_rel_path` 仅匹配精确 `.env`：  
  `crates/fs-policy/src/lib.rs:5-7`
- `file/write` / `file/edit` / `file/delete` 预检依赖该较窄规则：  
  `crates/app-server/src/main/file_write_patch.rs:51-67`  
  `crates/app-server/src/main/file_edit_delete.rs:61-77`  
  `crates/app-server/src/main/file_edit_delete.rs:272-288`

### 风险
- 规则不一致导致同类路径在不同层返回不同结果（Denied/Failed 等）。
- 维护时容易只改一处，出现行为漂移。

---

## 5) `notify-kit`：当前边界基本健康

### 观察
`omne-agent` 主要做事件映射与节流控制，发送层仍调用 `notify-kit`，未发现 sink 传输层重写。

### 证据
- `notify-kit` 作为 Hub + Sinks 对外导出：  
  `notify-kit/crates/notify-kit/src/lib.rs:12-21`
- `omne-agent` 通过标准入口构建 Hub：  
  `crates/app-server/src/main/preamble/server.rs:186-193`
- 其余逻辑主要是本域事件到通知事件的映射：  
  `crates/app-server/src/main/preamble/server.rs:195-212`  
  `crates/app-server/src/main/preamble/server.rs:335-381`

### 结论
当前无明显 `notify-kit` 领域侵占。

---

## 6) `policy-meta-spec`：当前边界基本健康

### 观察
`omne-agent` 当前主要复用 `WriteScope`，并已在执行网关接入路径中使用 `ExecutionIsolation`；没有重写 `policy-meta-spec` 的核心 schema，整体边界仍然健康。

### 证据
- `omne-protocol` 直接复用：  
  `crates/agent-protocol/src/lib.rs:590-614`
- `process_stream/common.rs` 已使用 `policy_meta::ExecutionIsolation` 对接执行网关：  
  `crates/app-server/src/main/process_stream/common.rs:39-42`

### 结论
当前无明显 `policy-meta-spec` 领域侵占。

---

## 建议（按优先级）

1. 先收口执行边界（高优先级）  
把 `process/start` 与 `execve_gate` 的“命令可执行性判定”统一委托给单一网关层（建议对齐 `agent-exec-gateway`），`omne-agent` 保留审批/事件编排。

2. 再收口 MCP 连接层（高优先级）  
`main/mcp/runtime.rs` 逐步替换为 `mcp-kit::Manager` 适配层，`omne-agent` 仅保留 mode/approval/event 相关策略。

3. LLM 边界继续瘦身（中优先级）  
将 raw OpenAI request 细节（如 `OpenAIResponsesRawRequest` 组装、cache 字段拼装策略）继续下沉到 `ditto-llm`，`omne-agent` 只传通用 `GenerateRequest` 与编排参数。

4. 文件安全规则单一化（中优先级）  
将 `.env`/secret 路径判定收敛为单一实现源（建议以 `safe-fs-tools`/共享策略库为准），避免 app-server 预检与 runtime 规则不一致。

5. 增加跨层一致性测试（中优先级）  
为“相同输入在不同入口一致判定”建立回归（process start vs execve gate；file precheck vs runtime；mcp runtime vs mcp-kit manager）。
