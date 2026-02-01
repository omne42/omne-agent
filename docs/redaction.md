# 脱敏（Redaction）与子进程环境清理（v0.2.0 口径）

> 目标：把“敏感信息泄漏到事件/日志/产物”的风险压到最低；即使开启回放/订阅，也尽量不把 secrets 写进历史。
>
> 注意：脱敏是 best-effort，不是安全边界；真正的硬边界来自 `sandbox/mode/execpolicy/approval`。

---

## 1) 事件落盘前自动脱敏（写死）

v0.2.0 在写入 `events.jsonl` 之前会对 `ThreadEventKind` 做脱敏处理：

- 文本字段（如 `TurnStarted.input`、`AssistantMessage.text`、各种 `reason/error`）会经过 `redact_text()`。
- JSON 参数/结果（如 `ApprovalRequested.params`、`ToolStarted.params`、`ToolCompleted.result`）会递归脱敏：
  - key 名看起来敏感（如包含 `token/password/api_key/authorization/cookie/...`）→ 值替换为 `"<REDACTED>"`。
  - string 值会再做一次 `redact_text()`（匹配已知 token 形态）。
- `ProcessStarted.argv` 会脱敏：支持 `--token xxx`/`--token=xxx` 等常见形态。

对照实现：

- `crates/core/src/redaction.rs`
- `crates/core/src/threads.rs`（`ThreadHandle::append` 写入前调用）

---

## 2) user artifacts 自动脱敏（`artifact/write`）

`artifact/write` 会在写盘前对内容做 `redact_text()`，避免把明显的 key/token 写进产物。

对照实现：

- `crates/app-server/src/main/artifact.rs`（写入前 `omne_agent_core::redact_text(&params.text)`）

---

## 3) 子进程环境清理（env scrub）

为了避免 child process 继承模型供应商密钥，`process/start` 会移除以下环境变量：

- `OPENAI_API_KEY`
- `OMNE_AGENT_OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`
- `OPENROUTER_API_KEY`
- `GEMINI_API_KEY`

对照实现：

- `crates/app-server/src/main/preamble/hardening.rs`（`CHILD_PROCESS_ENV_SCRUB_KEYS` + `scrub_child_process_env()`）
- `crates/app-server/src/main/process_control/start.rs`（spawn 前调用）

备注：

- 这只是“默认 denylist”。如果你的 repo 里还有其它 secrets（例如 `AWS_SECRET_ACCESS_KEY`），目前不会自动 scrub（后续可以扩展为可配置 denylist）。

---

## 4) 进程交互硬约束（非交互）

v0.2.0 的进程模型是**非交互**：

- `process/start`：`stdin = null`（不提供 PTY/stdin 交互）
- 只允许只读 attach：`process/inspect`、`process/tail`、`process/follow`
- 控制操作：`process/interrupt`、`process/kill`

意义：

- 避免“进程等输入但 agent 乱填”的不可审计行为。
- 进程若确实需要交互输入，应该失败/退出或由人类改成非交互命令（否则就手动 kill）。

---

## 5) 已知限制（别自欺欺人）

- 脱敏依赖正则/启发式 key 名：可能漏掉，也可能过度替换；不要把 secrets 当普通文本传给 agent。
- network access 目前是 best-effort gate（按命令名/子命令启发式识别），不是 OS 级网络沙箱。
- 未来的 `execve-wrapper`/OS hardening（禁 ptrace/core dump 等）属于更强的安全层，但不作为 v0.2.0 正确性的前提（TODO 草案见 `docs/os_hardening.md`）。

---

## 6) 快速自检

```bash
rg -n \"redact_text\\(|REDACTED\" crates/core/src/redaction.rs
rg -n \"CHILD_PROCESS_ENV_SCRUB_KEYS\" crates/app-server/src/main/preamble/hardening.rs
```
