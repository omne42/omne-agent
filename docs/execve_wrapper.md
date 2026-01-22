# Execve Wrapper / Shell Runner（TODO：规格草案）

> 目标：把“shell 内部的每一次 `execve`”也纳入 `mode/sandbox/execpolicy/approval` 的裁决链路，避免 `bash -lc '...'` 一次启动里偷偷跑多个高风险子命令而绕过策略。
>
> 状态：v0.2.0 **未实现**。当前仅对顶层 `process/start.argv` 做策略裁决（见 `docs/execpolicy.md`、`docs/approvals.md`）。

背景参考：

- `docs/research/codex.md`（Codex `patched bash + execve wrapper`）
- `docs/research/openai-cli-agent.md`

---

## 0) 范围与非目标

范围（未来实现）：

- 为 “shell tool / runner” 提供一种可拦截 `execve` 的执行后端，使 shell 内部的子进程启动也能被审计与拦截。
- 支持 `allow/deny/prompt/prompt_strict` 四态决策（其中 `prompt_strict` = Escalate，见 `docs/approvals.md`）。
- 与 MCP 对接：wrapper 把 `execve` 尝试上报到一个本地的决策服务（可实现为 MCP server）（见 `docs/mcp.md`）。

非目标（先别碰）：

- 取代 sandbox（Landlock/Seatbelt/容器）——wrapper 不是安全边界，只是**更早、更细粒度**的拦截点。
- 兼容所有 shell（zsh/fish/powershell…）；先只覆盖 `bash` 路径。
- 把每一次 allow 的 `execve` 都变成事件（会爆炸）：只对 **prompt/deny/escalate** 做强审计。

---

## 1) v0.2.0 现状与风险（先把边界说清楚）

现状：

- `process/start` 只接收 `argv: Vec<String>`（不接受单字符串命令），并对顶层 argv 做 `mode → sandbox → execpolicy → approvals`。
- 如果 argv 本身就是 `bash -lc ...`，那么 shell 内部实际跑了哪些子命令，v0.2.0 **看不见**。

最低防线（现在就能做）：

- 用 execpolicy 对 `bash -lc` / `sh -c` / `python -c` 这类“二次解释器入口”默认 `prompt` 或 `forbidden`（见 `docs/execpolicy.md`）。

---

## 2) 组件与数据流（最小可实现）

> 这里刻意只定义“决策语义与审计口径”。具体实现可以是 patched bash、`LD_PRELOAD`、或其它机制；别在 v0.2.0 的文档里承诺维护某个特定 fork。

### 2.1 组件

- **patched bash**：在 bash 触发 `execve` 前回调一个外部程序（约定环境变量 `BASH_EXEC_WRAPPER` 指向 wrapper）。
- **execve-wrapper**：一个小二进制；接收 `cwd/argv/pid/...`，向决策服务请求裁决后决定放行/拒绝。
- **decision service**：本地服务端（建议实现为 MCP server，transport=stdio 或 unix socket）。
  - 负责运行 `mode/sandbox/execpolicy/approval` 链路并返回决策。

### 2.2 线程/turn 绑定（避免“哪个 thread 在执行”丢失）

当 `process/start` 启动一个“启用 wrapper 的 shell”时，必须向 child 注入非敏感 env（示例，命名占位）：

- `CODE_PM_THREAD_ID`
- `CODE_PM_TURN_ID`
- `CODE_PM_EXECVE_SOCKET`（或 stdio 连接信息）
- `CODE_PM_EXECVE_TOKEN`（随机 nonce；用于本地鉴权，防止其它进程伪造请求）

wrapper 必须把上述字段回传给 decision service，保证审计与审批归属正确。

---

## 3) 决策接口（最小协议草案）

> 注意：这是 wrapper ↔ decision service 的内部协议，不等同于 app-server JSON-RPC。

请求（JSON）：

```json
{
  "version": 1,
  "thread_id": "01J...",
  "turn_id": "01J...",
  "pid": 12345,
  "cwd": "/abs/path",
  "argv": ["git", "status"]
}
```

响应（JSON）：

```json
{
  "decision": "allow | deny | prompt | prompt_strict",
  "reason": "optional human readable string",
  "approval_id": "01J... (only for prompt/prompt_strict)"
}
```

语义（写死）：

- `allow`：wrapper 继续执行（实际 `execve`）。
- `deny`：wrapper 必须拒绝执行，并把原因写入 stderr（stdout/stderr 会被 `process/start` 落盘）。
- `prompt`：进入 approval handling（见 `docs/approvals.md`）；可被 `ApprovalPolicy=auto_approve` 自动放行。
- `prompt_strict`：强制人工审批（Escalate）；不能被 auto_approve 自动放行，且不应被 remembered decision 自动复用（见 `docs/approvals.md`）。

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
  - 用 `CODE_PM_EXECVE_TOKEN` 做一次请求鉴权（否则其它本地进程可伪造 allow）
- 决策时不要只信 `argv[0]` 的“名字”：要明确 `PATH` 污染与 TOCTOU 风险；可用时优先基于解析后的绝对路径/文件元信息做裁决（细节实现不在本文承诺范围内）。
- wrapper 不应上传完整 env；需要时只上传 allowlist（并做脱敏）。
- decision service 的日志/事件不得落盘原始 payload（只记录元信息 + 脱敏视图；见 `docs/redaction.md`）。
- 对“等待审批”的阻塞必须有超时兜底（避免永远挂死）；超时建议映射为 `TurnStatus::Stuck`（见 `docs/budgets.md`）。
- gate 不可用/鉴权失败时的默认行为必须保守：建议 `deny` 或 `prompt_strict`，禁止静默放行。

---

## 6) DoD（未来实现的可验证清单）

- 在启用 wrapper 的 shell 中执行 `curl https://example.com`：
  - 若 execpolicy 对 `curl` 为 `forbidden`：必须被拒绝，stderr 含拒绝原因，且不产生审批事件。
  - 若 execpolicy 对 `curl` 为 `prompt`：必须产生 `ApprovalRequested`，`pm inbox` 可见；批准后继续执行。
- 在 `ApprovalPolicy=auto_approve` 下触发 `prompt_strict`：
  - 必须进入 `NeedApproval`，不会自动放行（对齐 `docs/approvals.md`）。
- 任何路径上不得把 `OPENAI_API_KEY` 等 secrets 写入事件/日志（可用哨兵值回归测试；见 `docs/redaction.md`）。
