# OS / Process Hardening（v0.2.x 最小子集已实现）

> 目标：把“本地全权限执行”的风险压到可接受范围：减少 secrets 泄露面、降低被调试/转储/注入的概率、减少非交互场景挂死。
>
> 状态：v0.2.x 已实现 **最小子集**（`OMNE_HARDENING` 开关 + pre-main best-effort + child process 最小 env scrub/非交互）；其余仍为 TODO（见下文）。

---

## 0) 范围与原则

范围（当前 + 未来）：

- **pre-main**：进程启动早期做一次 best-effort hardening。
- **child process**：对 `process/start` 统一施加一组“安全默认值”（仍受 `mode/sandbox/execpolicy/approval` 约束）。

原则：

- 不发明“我们自己的沙箱”：hardening 是 **best-effort**，不能替代 `sandbox/approvals/execpolicy`。
- 跨平台差异要明确：Linux-only 的能力必须可选且不影响其它平台正确性。
- 可审计：启用/禁用 hardening 必须能在事件/日志里被定位（避免“我以为开了，其实没开”）。

---

## 1) 开关与可观测性（v0.2.x 已实现）

v0.2.x 已提供 **明确开关**，并把生效结果写进日志（best-effort，但不能黑箱）：

- env：`OMNE_HARDENING=off|best_effort`
  - 默认：`best_effort`
  - `off`：不做 pre-main hardening；child process hardening 仅保留最小子集（env scrub + 非交互约束；不注入 env defaults）。
- 记录位置：
  - app-server 启动日志（stderr）：打印 `hardening=<mode>` + 每个子项 `applied|enabled|skipped|failed(reason)`。
  - 不建议把“失败原因”塞进 thread 事件：它不属于单一 thread；但必须可从 server 日志定位。
  - 所有 hardening 失败都 **不阻止启动**（best-effort），但必须记录。

平台差异（v0.2.x 最小子集）：

- Linux：core dump/umask/ptrace 限制 best-effort；缺失权限则记录为 skipped/failed。
- macOS：core dump/umask best-effort；ptrace 限制不支持时记录 skipped。
- Windows：当前未支持（`omne-app-server` 依赖 `nix`）。

fail-closed（只对配置解析）：

- 若未来引入 hardening 的配置文件（例如 allowlist/denylist），解析失败/未知字段应直接报错（避免“以为开了，其实没开”）。

---

## 2) 现状（v0.2.x 最小子集）

- pre-main hardening（best-effort）：
  - 禁止 core dump（Unix）。
  - 更严格的默认文件权限：`umask(0o077)`（Unix）。
  - Linux-only：禁止被 ptrace/attach（`prctl(PR_SET_DUMPABLE, 0)`）。
- child process hardening（最小子集）：
  - `process/start` 对 child env 做 scrub（实现对照：`crates/app-server/src/main/preamble.rs`）。
  - 非交互约束：不做 stdin/PTY 交互（见 `docs/v0.2.0_parity.md` 的进程模型）。
  - 非交互 env defaults（仅当未设置且 hardening 启用时注入到 child env）：`GIT_TERMINAL_PROMPT=0`、`NO_COLOR=1`、`PAGER=cat`。
  - 支持按 env 追加 scrub 规则（默认关闭）：`OMNE_HARDENING_EXTRA_SCRUB_KEYS`（逗号分隔键名）、`OMNE_HARDENING_EXTRA_SCRUB_PATTERNS`（逗号分隔 glob，如 `*_TOKEN,*SECRET*`，大小写不敏感）。
  - 支持按 env 启用继承 env allowlist（默认关闭）：`OMNE_HARDENING_ALLOW_ENV_KEYS`（逗号分隔键名）、`OMNE_HARDENING_ALLOW_ENV_PATTERNS`（逗号分隔 glob，大小写不敏感）。
  - `process/start` 返回 `effective_env_summary`（脱敏）：`hardening_mode`、`scrubbed_keys`、`allowlist_dropped_keys`、`injected_defaults`、`configured_extra_scrub_keys`、`configured_extra_scrub_patterns`、`configured_allowed_env_keys`、`configured_allowed_env_patterns`。
- 事件落盘与 artifact 写入前脱敏（见 `docs/redaction.md`）。

---

## 3) TODO：pre-main hardening（增强项）

> 目标：减少“主进程被附加调试/转储”的机会，并把默认行为更像 CI（非交互）。

增强项（best-effort）：

- [x] 扩展环境清理清单（动态链接器/调试/注入相关变量）：启动时清理 `LD_PRELOAD/LD_LIBRARY_PATH/LD_AUDIT/LD_DEBUG/DYLD_*`，并把实际清理结果写入启动日志。
- [x] 如系统支持，启用更严格的 ptrace 限制（例如 Yama；取决于部署环境）：通过 `OMNE_HARDENING_LINUX_YAMA_PTRACE_SCOPE=0..3` 显式开启（默认关闭），best-effort 写入 `/proc/sys/kernel/yama/ptrace_scope`。
- 统一非交互环境默认：
  - [x] `CI=1`（可选；注意可能改变某些工具输出）：通过 `OMNE_HARDENING_SET_CI=true` 显式开启，默认关闭。

验收（未来实现时）：

- `omne-app-server` 启动日志中能看到：增强项是否启用、哪些项成功应用、哪些项因平台/权限失败并被记录为 warning。

---

## 4) TODO：child process hardening（增强项）

> 目标：降低 secrets 外泄与交互挂死；保证同一类命令在不同环境里更可复现。

增强项（在 `process/start` 内统一施加，best-effort）：

- env scrub 更系统化：
  - [x] 允许通过配置追加 scrub keys/patterns（例如 `*_TOKEN/*KEY*/*SECRET*` 的已知键集合），默认关闭且可审计。
  - [x] 对“允许透传的白名单”明确化（默认关闭）：`OMNE_HARDENING_ALLOW_ENV_KEYS/OMNE_HARDENING_ALLOW_ENV_PATTERNS`，并把生效结果写入 `effective_env_summary`（脱敏后）。
- 资源边界：
  - [x] 支持 `process/start.timeout_ms`（毫秒，可选）作为 child process wall-clock timeout；超时后沿用 `process/kill` 语义（`ProcessKillRequested` + `ProcessExited.reason` 可审计）。

当前最小验收（已实现）：

- `process/start` 返回值里可见 `effective_env_summary`（脱敏后的“删了哪些 key/注入了哪些默认值”，只用于审计）。
- `process/start` 接收可选 `timeout_ms`（`>=1`），并在返回值回显该字段；超时后会触发 kill 事件链路。

---

## 5) 与其它策略的关系（写死顺序）

hardening 不能改变权限裁决链路：

`allowed_tools → hard boundary / config validation → mode gate → execpolicy → approval handling → process/start (hardening applied)`

也就是说：

- `deny` 仍然 deny（hardening 不是后门）。
- `prompt` 仍然会产生审批事件（hardening 不能绕过审批）。
