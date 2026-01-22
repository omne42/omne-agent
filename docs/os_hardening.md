# OS / Process Hardening（TODO：规格草案）

> 目标：把“本地全权限执行”的风险压到可接受范围：减少 secrets 泄露面、降低被调试/转储/注入的概率、减少非交互场景挂死。
>
> 状态：本文是 **TODO 规格草案**（v0.2.0 未实现；当前仅有部分 env scrub/脱敏，见 `docs/redaction.md`）。

---

## 0) 范围与原则

范围（未来实现）：

- **pre-main**：进程启动早期做一次 best-effort hardening。
- **child process**：对 `process/start` 统一施加一组“安全默认值”（仍受 `mode/sandbox/execpolicy/approval` 约束）。

原则：

- 不发明“我们自己的沙箱”：hardening 是 **best-effort**，不能替代 `sandbox/approvals/execpolicy`。
- 跨平台差异要明确：Linux-only 的能力必须可选且不影响其它平台正确性。
- 可审计：启用/禁用 hardening 必须能在事件/日志里被定位（避免“我以为开了，其实没开”）。

---

## 1) TODO：开关与可观测性（建议写死）

v1 建议给 hardening 一个**明确开关**，并把生效结果写进日志（best-effort，但不能黑箱）：

- env：`CODE_PM_HARDENING=off|best_effort`
  - 默认：`best_effort`
  - `off`：完全不做 pre-main hardening；child process hardening 仅保留 v0.2.0 已实现的“必须项”（例如 secrets env scrub 与非交互约束）。
- 记录位置（建议）：
  - app-server 启动日志（stderr）：打印 `hardening=<mode>` + 每个子项 `applied|skipped|failed(reason)`。
  - 不建议把“失败原因”塞进 thread 事件：它不属于单一 thread；但必须可从 server 日志定位。

fail-closed（只对配置解析）：

- 若未来引入 hardening 的配置文件（例如 allowlist/denylist），解析失败/未知字段应直接报错（避免“以为开了，其实没开”）。

---

## 2) 现状（v0.2.0 已实现的部分）

- 事件落盘与 artifact 写入前脱敏（见 `docs/redaction.md`）。
- `process/start` 对 child env 做 scrub（实现对照：`crates/app-server/src/main/preamble.rs`）。
- 非交互约束：不做 stdin/PTY 交互（见 `docs/v0.2.0_parity.md` 的进程模型）。

---

## 3) TODO：pre-main hardening（建议项）

> 目标：减少“主进程被附加调试/转储”的机会，并把默认行为更像 CI（非交互）。

建议项（best-effort）：

- 禁止 core dump（Linux/macOS）：
  - `setrlimit(RLIMIT_CORE, 0)`
- 更严格的默认文件权限：
  - `umask(0o077)`（或等价）
- Linux-only：禁止被 ptrace（尽量）
  - `prctl(PR_SET_DUMPABLE, 0)`
  - 如系统支持，启用更严格的 ptrace 限制（例如 Yama；取决于部署环境）
- 统一非交互环境默认：
  - `GIT_TERMINAL_PROMPT=0`
  - `CI=1`（可选；注意可能改变某些工具输出）

验收（未来实现时）：

- `pm-app-server` 启动日志（或 thread 事件）中能看到：hardening 是否启用、哪些项成功应用、哪些项因平台/权限失败并被记录为 warning。

---

## 4) TODO：child process hardening（建议项）

> 目标：降低 secrets 外泄与交互挂死；保证同一类命令在不同环境里更可复现。

建议项（在 `process/start` 内统一施加，best-effort）：

- env scrub 更系统化：
  - 保持 v0.2.0 的最小 scrub 集为默认（模型供应商 key；见 `docs/redaction.md`），避免破坏用户显式依赖的凭据环境。
  - 允许通过配置追加 scrub keys/patterns（例如 `*_TOKEN/*KEY*/*SECRET*` 的已知键集合），但默认应关闭并且必须可审计（否则容易“无意中删掉凭据导致命令莫名失败”）。
  - 对 “允许透传的白名单” 明确化（宁可少透传），并把最终生效结果写入 `effective_env_summary`（脱敏后）。
- 强制非交互输入：
  - `stdin=null`（已定约束）
  - 禁用颜色/分页器（减少输出差异与 hang）：
    - `NO_COLOR=1`
    - `PAGER=cat`
- 资源边界（可选）：
  - 对 child process 设置 wall-clock timeout（与 `process/kill` 语义配合）

验收（未来实现时）：

- `process/start` 返回值里能看到 `cwd/stdout_path/stderr_path`（已实现）+ `effective_env_summary`（脱敏后的“删了哪些 key/覆盖了哪些 key”，只用于审计）。

---

## 5) 与其它策略的关系（写死顺序）

hardening 不能改变权限裁决链路：

`mode gate → sandbox → execpolicy → approval handling → process/start (hardening applied)`

也就是说：

- `deny` 仍然 deny（hardening 不是后门）。
- `prompt` 仍然会产生审批事件（hardening 不能绕过审批）。
