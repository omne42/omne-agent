# ExecPolicy（命令执行策略）（v0.2.0 口径）

> 目标：把“能不能跑这个命令”从 prompt 里的软约束，变成**可配置、可解释、可审计**的规则。
>
> v0.2.0 现实：ExecPolicy 目前只用于 `process/start`（命令执行），不覆盖 file tools。

---

## 0) 范围与合并顺序（写死）

执行判定链路顺序固定为：

`mode gate → sandbox → execpolicy → approval handling`

ExecPolicy 只负责 `process/start` 的**命令前缀规则**：

- `forbidden`：硬拒绝（不走审批）
- `prompt_strict`：需要审批，且**强制人工审批**（不能被 `auto_approve/on_request/unless_trusted` 自动放行；见 `docs/approvals.md`）
- `prompt`：需要审批（由 `ApprovalPolicy` 决定人工/自动）
- `allow`：ExecPolicy 不拦截（仍可能被 mode/sandbox 拦截或要求审批）

合并优先级：`forbidden > prompt_strict > prompt > allow`（取最大值）。

默认行为（写死，v0.2.0 实现口径）：

- 未匹配任何规则：按 `prompt` 处理（需要审批）。

---

## 1) Decision 与行为（v0.2.0）

### 1.1 `forbidden`

- `process/start` 会返回 `denied=true`，并落盘：
  - `ToolStarted { tool="process/start", params={argv,cwd} }`
  - `ToolCompleted { status=Denied, error="execpolicy forbids this command", result={decision, matched_rules, justification?} }`

### 1.2 `prompt_strict`

- 当最终决策为 `prompt_strict` 时，需要审批，且强制人工：
  - `ApprovalPolicy=manual|auto_approve|on_request|unless_trusted`：返回 `needs_approval=true` + `approval_id`，进入 `NeedApproval`（不启动进程）。
  - `ApprovalPolicy=auto_deny`：自动落盘 `ApprovalRequested + ApprovalDecided(Denied, ...)` 并拒绝执行。

### 1.3 `prompt`

- 当最终决策为 `prompt` 时，需要审批（见 `docs/approvals.md`）：
  - `ApprovalPolicy=manual`：返回 `needs_approval=true` + `approval_id`，进入 `NeedApproval`（不启动进程）。
  - `ApprovalPolicy=auto_approve|on_request`：自动落盘 `ApprovalRequested + ApprovalDecided(Approved, ...)` 并继续启动进程（不会进入 `NeedApproval`）。
  - `ApprovalPolicy=auto_deny`：自动落盘 `ApprovalRequested + ApprovalDecided(Denied, ...)` 并拒绝执行。

### 1.4 `allow`

- ExecPolicy 本身不要求审批；但如果 mode 的 `command=prompt`，仍会走审批。
- 当 `ApprovalPolicy=unless_trusted` 时：
  - 仅对 `process/start`：若 execpolicy 最终决策为 `allow`，则自动批准；否则进入人工审批。

---

## 2) 规则语言：prefix-rule（最小集）

规则文件使用 `.rules`（Starlark subset）语法，核心是 `prefix_rule(...)`：

- `pattern`：命令前缀匹配（可用 token 备选）
- `decision`：`"allow" | "prompt" | "prompt_strict" | "forbidden"`（缺省为 `allow`）
- `justification`：可选，便于审计输出
- `match` / `not_match`：可选，用例校验（像单测一样，解析时即验证）

示例（只允许 `git status`，其它一律 prompt）：

```starlark
prefix_rule(
    pattern = ["git", "status"],
    decision = "allow",
)
```

更复杂的 token 备选（只展示形式，别过度设计）：

```starlark
prefix_rule(
    pattern = [["bash", "sh"], ["-c", "-lc"]],
    decision = "forbidden",
    justification = "avoid string-eval shells; prefer argv direct exec",
)
```

安全提示（写死口径）：

- `bash -lc` / `sh -c` / `python -c` / `node -e` 这类“解释执行字符串”的入口，建议默认 `forbidden`（否则在 `ApprovalPolicy=auto_approve` 下等价于任意命令执行）。
- 真要允许，优先走“显式白名单 + 人工审批”（例如 `ApprovalPolicy=manual`，或 `prompt_strict`；见 `docs/approvals.md`）。

---

## 3) 如何加载（v0.2.0：global + per-mode）

### 3.1 global（启动参数）

ExecPolicy 可以由 app-server 启动参数全局注入：

- app-server：`pm-app-server --execpolicy-rules <PATH> [--execpolicy-rules <PATH> ...]`
- `pm` CLI：同名参数透传给 app-server：`pm --execpolicy-rules <PATH> ...`

fail-closed（写死）：

- 任何 `--execpolicy-rules` 文件缺失/不可读/解析失败：app-server 应直接启动失败（不要静默忽略或回退到“空规则”）。
  - 空规则意味着“全部命令都走 `prompt`”，在 `ApprovalPolicy=auto_approve` 下等价于 allow-all。

### 3.2 per-mode（`modes.yaml`）

v0.2.0 支持为某个 mode 单独配置额外的规则文件列表：

- 配置位置：`./.codepm_data/spec/modes.yaml` → `modes.<name>.permissions.command.execpolicy_rules: [<path>...]`
- 路径解析：绝对路径按原样使用；相对路径按 **thread cwd（workspace root）** 解析（并通过 path boundary 校验，避免 `..`/symlink 逃逸）
- 合并顺序（写死）：`global rules（启动参数） → mode rules`
- fail-closed（写死）：mode 指定的 rules 文件缺失/不可读/解析失败时，该次 `process/start` 必须直接拒绝并返回可诊断错误（不要静默忽略该层）

---

## 4) 快速验证（可复制）

检查某条命令会匹配哪些规则：

```bash
cargo run -p pm-execpolicy -- check -r /path/to/policy.rules -- git status
```

让 `pm` 在启动 app-server 时加载规则：

```bash
cargo run -p pm -- --execpolicy-rules /path/to/policy.rules thread start --json
```

---

## 5) per-mode / per-thread execpolicy rules

问题：全局 `--execpolicy-rules` 很容易“要么过宽要么过窄”。不同 mode（architect/coder/reviewer/builder）需要不同的命令白名单与提示策略。

实现状态：

- **per-mode**：已实现（见上文 3.2）。
- **per-thread**：TODO（未来通过 `thread/configure` 写入 thread config 事件实现）。

规格草案（per-thread 仍未落地）：

- **per-thread**：允许 thread config 增加 `execpolicy_rules: [<path>...]`（例如通过 `thread/configure` 写入 `ThreadConfigUpdated`），用于：
  - 临时补充 allowlist（让某些命令免审批）
  - 或临时收紧（强制 `prompt/forbidden`）
  - 路径解析同上（按 thread cwd 解析相对路径）。
- **合并顺序（写死）**：`global rules（启动参数） → mode rules → thread override（如有，TODO）`
  - 决策合并仍为 `forbidden > prompt > allow`。
  - 语义提醒（写死，避免误会）：规则是“并集叠加”。
    - 任一层命中 `forbidden` ⇒ 最终 `forbidden`（无法被其它层的 `allow` 抵消）。
    - 任一层命中 `prompt` ⇒ 最终 `prompt`（无法被其它层的 `allow` 抵消）。
    - 当所有层都 **未命中任何规则** 时 ⇒ 最终 `prompt`（v0.2.0 默认）。
  - 审计输出必须能看到：最终 decision 来自哪个规则文件/哪条规则（避免黑箱）。
  - fail-closed（写死）：
    - mode/thread 指定的 rules 文件缺失/不可读/解析失败：应直接拒绝 `process/start`（等价 forbidden）并返回可诊断错误（不要静默忽略该层）。

验收（未来实现时）：

- `pm thread config-explain <thread_id>` 能解释：当前有效的 rules 来源顺序（global/mode/thread）与每次 `process/start` 的 matched rules。
- `ApprovalPolicy=unless_trusted` 仍以 **最终 execpolicy decision** 为准（allow 才可能自动批准）。
