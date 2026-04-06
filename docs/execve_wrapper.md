# Execve Wrapper / Shell Runner（v0.2.x：实验性实现 + 规格）

> 目标：把“shell 内部的每一次 `execve`”也纳入 `mode/sandbox/execpolicy/approval` 的裁决链路，避免 `bash -lc '...'` 一次启动里偷偷跑多个高风险子命令而绕过策略。
>
> 状态：v0.2.x **已实现（实验性；unix + bash）**。实现落点：
>
> - execve gate（MCP server over unix socket）：`crates/app-server/src/main/process_control/execve_gate.rs`
> - execve wrapper 二进制：`crates/execve-wrapper`（`omne-execve-wrapper`）
> - 启用方式（最小）：`OMNE_EXECVE_WRAPPER=omne-execve-wrapper` + 用 patched bash 运行 `process/start`（见下文）。

背景参考：

- `docs/research/codex.md`（Codex `patched bash + execve wrapper`）
- `docs/research/openai-cli-agent.md`

---

## 0) 范围与非目标

范围（v0.2.x 已实现）：

- 为 “shell tool / runner” 提供一种可拦截 `execve` 的执行后端，使 shell 内部的子进程启动也能被审计与拦截。
- 支持 `run/deny/escalate` 三态（wrapper ↔ gate），其中 `escalate` 通过 approvals 复用 `prompt/prompt_strict`（见 `docs/approvals.md`）。
- 与 MCP 对接：execve gate 以 MCP server（JSON-RPC）方式在本机 unix socket 上提供 `tools/call`。

非目标（先别碰）：

- 取代 sandbox（Landlock/Seatbelt/容器）——wrapper 不是安全边界，只是**更早、更细粒度**的拦截点。
- 兼容所有 shell（zsh/fish/powershell…）；先只覆盖 `bash` 路径。
- 把每一次 allow 的 `execve` 都变成事件（会爆炸）：只对 **prompt/deny/escalate** 做强审计。

---

## 1) v0.2.x 现状与风险（先把边界说清楚）

现状（启用前）：

- `process/start` 只接收 `argv: Vec<String>`（不接受单字符串命令），并对顶层 argv 做 `mode → sandbox → execpolicy → approvals`。
- 如果 argv 本身就是 `bash -lc ...`，那么 shell 内部实际跑了哪些子命令，v0.2.0/0.2.x **默认看不见**。

现状（启用后）：

- 当 `process/start` 启动 `bash` 且设置了 `OMNE_EXECVE_WRAPPER` 时，server 会启动一个 per-process 的 execve gate（unix socket, `0600`），并向 bash 注入：
  - `BASH_EXEC_WRAPPER=<OMNE_EXECVE_WRAPPER>`
  - `OMNE_EXECVE_SOCKET=<.../execve-gate.sock>`
  - `OMNE_EXECVE_TOKEN=<random nonce>`
  - `OMNE_THREAD_ID=<thread_id>`
  - `OMNE_TURN_ID=<turn_id>`（如有）
- patched bash 在每次 `execve` 前会改写 argv：`[wrapper, orig_command, <original argv...>]`；`omne-execve-wrapper` 通过 gate 做裁决后再 `execve` 原命令。

最低防线（现在就能做）：

- 用 execpolicy 对 `bash -lc` / `sh -c` / `python -c` 这类“二次解释器入口”默认 `prompt` 或 `forbidden`（见 `docs/execpolicy.md`）。

---

## 2) 组件与数据流（v0.2.x 最小可用）

> 这里刻意只定义“决策语义与审计口径”。具体实现可以是 patched bash、`LD_PRELOAD`、或其它机制；别在 v0.2.0 的文档里承诺维护某个特定 fork。

### 2.1 组件

- **patched bash**：在 bash 触发 `execve` 前回调一个外部程序（约定环境变量 `BASH_EXEC_WRAPPER` 指向 wrapper）。
- **execve-wrapper**：一个小二进制；接收 `cwd/argv/pid/...`，向决策服务请求裁决后决定放行/拒绝。
- **execve gate（decision service）**：本地服务端（v0.2.x 实现为 MCP server，transport=unix socket）。
  - 负责运行 `sandbox_network_access` 的 best-effort argv 检测，以及 `execpolicy + approvals` 链路，并返回 `run/deny/escalate`。
  - execpolicy 的“无匹配”默认按 `process/start` 同口径处理：进入 `prompt` fallback，而不是静默 `allow`，避免 `bash -lc` 这类二次解释入口绕过顶层审批语义。

### 2.2 线程/turn 绑定（避免“哪个 thread 在执行”丢失）

当 `process/start` 启动一个“启用 wrapper 的 shell”时，必须向 child 注入非敏感 env（示例，命名占位）：

- `OMNE_THREAD_ID`
- `OMNE_TURN_ID`
- `OMNE_EXECVE_SOCKET`（或 stdio 连接信息）
- `OMNE_EXECVE_TOKEN`（随机 nonce；用于本地鉴权，防止其它进程伪造请求）

wrapper 必须把上述字段回传给 decision service，保证审计与审批归属正确。

---

## 3) 决策接口（v0.2.x：MCP tools）

> 注意：这是 wrapper ↔ execve gate 的内部 MCP 通道，不等同于 app-server JSON-RPC。

### 3.1 `omne.execve.decide`

输入（`tools/call.arguments`）：

```json
{
  "token": "random nonce",
  "cwd": "/abs/path (optional)",
  "argv": ["git", "status"]
}
```

输出（`tools/call.result.content[0].text` 的 JSON）：

```json
{
  "decision": "run | deny | escalate",
  "reason": "optional human readable string (deny)",
  "approval_id": "01J... (only for escalate)"
}
```

语义（v0.2.x）：

- `run`：wrapper 继续执行（实际 `execve`）。
- `deny`：wrapper 必须拒绝执行，并把原因写入 stderr（stdout/stderr 会被 `process/start` 落盘）。
- `escalate`：必须进入 approvals（`ApprovalRequested` 落盘）；wrapper 需调用 `omne.execve.wait` 等待最终 `run/deny`。

### 3.2 `omne.execve.wait`

输入（`tools/call.arguments`）：

```json
{
  "token": "random nonce",
  "approval_id": "01J...",
  "timeout_ms": 900000
}
```

输出（`tools/call.result.content[0].text` 的 JSON）：

```json
{
  "decision": "run | deny",
  "reason": "optional",
  "remember": false
}
```

---

## 4) 审计与落盘（最小要求）

为避免事件爆炸，最小落盘要求建议：

- `deny/prompt/prompt_strict` 必须落盘成事件（通过 approvals 或专门事件二选一，先复用 approvals）：
  - `prompt/prompt_strict`：必须产生 `ApprovalRequested`，并在决定后产生 `ApprovalDecided`。
    - 建议在 `ApprovalRequested.params` 写入：`{"approval": {"requirement": "...", "source": "execve-wrapper", "reason": "..."}}`（对齐 `docs/approvals.md` 的 `prompt_strict` 草案）。
  - `deny`：至少要在进程 stderr 有一条可检索的拒绝原因（并走脱敏；见 `docs/redaction.md`）。
- `allow`：默认不落盘每一次 execve（只由顶层 `ProcessStarted/Exited` 表达），避免噪声。

---

## 5) 安全约束（最低标准）

- wrapper 与 decision service 的连接必须是本机可信通道：
  - unix socket 文件权限 `0600`（或等价）
  - 用 `OMNE_EXECVE_TOKEN` 做一次请求鉴权（否则其它本地进程可伪造 allow）
- 决策时不要只信 `argv[0]` 的“名字”：要明确 `PATH` 污染与 TOCTOU 风险；可用时优先基于解析后的绝对路径/文件元信息做裁决（细节实现不在本文承诺范围内）。
- wrapper 不应上传完整 env；需要时只上传 allowlist（并做脱敏）。
- decision service 的日志/事件不得落盘原始 payload（只记录元信息 + 脱敏视图；见 `docs/redaction.md`）。
- 对“等待审批”的阻塞必须有超时兜底（避免永远挂死）；超时建议映射为 `TurnStatus::Stuck`（见 `docs/budgets.md`）。
- gate 不可用/鉴权失败时的默认行为必须保守：建议 `deny` 或 `prompt_strict`，禁止静默放行。

---

## 6) DoD（v0.2.x 可验证清单）

- 在启用 wrapper 的 bash 中执行 `curl https://example.com`（默认 `sandbox_network_access=deny`）：
  - 必须被基于 argv 的网络命令检测拒绝，stderr 含拒绝原因，且不产生审批事件。
- execpolicy 对 `git` 设为 `prompt_strict`（或等价规则）后，在 bash 中执行 `git status`：
  - 必须产生 `ApprovalRequested`，`omne inbox` 可见；批准后继续执行。
- 任何路径上不得把 `OPENAI_API_KEY` 等 secrets 写入事件/日志（可用哨兵值回归测试；见 `docs/redaction.md`）。
