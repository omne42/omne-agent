# Approvals（审批）规范（v0.2.0 口径）

> 目标：把“需要人类放行/拒绝”的点，从 prompt 里的软约束，变成**可落盘、可回放、可审计**的事实。
>
> 这份文档描述的是**已经落地的行为** + **Escalate（prompt_strict）** 的实现口径（v0.2.0 已实现）。

---

## 0) 范围与非目标

范围（v0.2.0）：

- 覆盖所有会产生 side-effect 的工具调用（`process/start`、`file/*`、`artifact/*`…）在需要审批时的事件化与回放语义。
- 覆盖 `ApprovalPolicy` 五档策略与 `remember` 记忆规则（默认 scope=thread/session）。
- 覆盖与 `mode gate / sandbox / execpolicy` 的合并顺序。

非目标（v0.2.0）：

- OS 级强隔离（网络 namespace、seccomp、execve wrapper 等）——可以预留，但不作为正确性前提。
- UI/终端如何渲染审批（只定义控制面与事件语义）。

---

## 1) 事件模型：落盘即真相

审批以两类事件表达（`ApprovalId` 为 join key）：

- `ApprovalRequested { approval_id, turn_id?, action, params }`
- `ApprovalDecided { approval_id, decision, remember, reason? }`

关键不变量：

- **先 request 后 decided**：同一个 `approval_id` 必须先出现 `ApprovalRequested`。
- **自动决策也要落盘**：即使 `ApprovalPolicy=auto_approve/auto_deny`，也必须写入 `ApprovalRequested + ApprovalDecided`（审计/回放需要）。
- **事件按 `EventSeq` 单调递增排序**（订阅可重复投递；客户端按 `seq` 去重）。

---

## 2) ApprovalPolicy：现实现状（v0.2.0）

当上游（mode/execpolicy）产生 `prompt` 时，进入 approval handling。策略如下：

- `auto_approve`
  - 写入 `ApprovalRequested`，立即写入 `ApprovalDecided(Approved, reason="auto-approved by policy")`，不中断执行。
- `on_request`
  - v0.2.0 行为与 `auto_approve` 等价（保留语义槽位，避免未来改名/破坏兼容）。
- `manual`
  - 只写入 `ApprovalRequested`，返回 `needs_approval=true`（进入 `NeedApproval`）；等待人类追加 `ApprovalDecided`。
- `unless_trusted`
  - 仅对 `action=process/start`：当 `execpolicy` 对该 `argv` 的最终决策为 `allow` 时自动批准；否则进入人工审批。
  - 其它 `action`：一律进入人工审批。
- `auto_deny`
  - 写入 `ApprovalRequested`，立即写入 `ApprovalDecided(Denied, reason="auto-denied by policy")`，并拒绝执行。

实现对照（仅用于核对，不作为规范的一部分）：

- `crates/agent-protocol/src/lib.rs`（`ApprovalPolicy/ApprovalDecision/ThreadEventKind`）
- `crates/app-server/src/main/approval.rs`（`gate_approval()`）

---

## 3) `remember`：session 内“别再重复骚扰用户”

语义（v0.2.0）：

- 人类在 `approval/decide` 时可选 `remember=true`。
- `remember=true` 的 `ApprovalDecided` 会在 **thread/session 内**形成“记忆表”（派生状态，不是权威来源）。
- 后续遇到同一条规则 key：
  - 系统仍会追加一对新的 `ApprovalRequested + ApprovalDecided`（`remember=false`），并在 `reason` 写清楚是“remembered decision”触发。
  - 决策会自动生效：Approved 则继续，Denied 则拒绝。

key 的稳定性（v0.2.0 实现口径）：

- `file/write`：`path + create_parent_dirs`
- `file/delete`：`path + recursive`
- `fs/mkdir`：`path + recursive`
- `file/edit`：`path`
- `file/patch`：`path`
- `process/start`：使用 `params` 的完整 JSON 序列化（避免遗漏风险字段）
- 其它 action：`{action}|{params_json}`

注意：key 过宽会误放行，过窄会重复审批。**宁可重复，也别误放行**。

---

## 4) 与 mode / sandbox / execpolicy 的组合顺序（写死）

执行判定链路顺序固定为：

`mode gate → sandbox → execpolicy → approval handling`

规则：

- 任一层 `deny`（execpolicy 的 `forbidden` 视为 deny）：硬拒绝（不走审批；approval policy 不能覆盖 deny）。
- 任一层 `prompt` 且没有 deny：进入 `approval handling`（按 `ApprovalPolicy` 决定自动/人工）。
- 否则：allow，继续执行。

---

## 5) Escalate 语义（v0.2.0：prompt_strict 已实现）

问题：当前只有 `prompt`（可被 `auto_approve` 自动放行）与 `deny`（硬拒绝）。缺少一种“**即使 auto_approve 也必须停下来**”的语义槽位。

定义（建议）：`escalate = prompt_strict`

- 语义：强制人工审批（不能被 `auto_approve/on_request/unless_trusted` 自动放行）。
- 不是“绕过 deny”：如果 `mode/sandbox/execpolicy` 的结果是 deny，仍然 deny。
- 触发来源（v0.2.0）：
  - ExecPolicy 支持 decision=`prompt_strict`；当 `process/start` 命中该决策，会在 `ApprovalRequested.params` 写入 `approval.requirement="prompt_strict"`（见 `docs/execpolicy.md`）。
  - 未来的 `execve-wrapper`/MCP 等低层执行通道也可复用该槽位（参考：`docs/execve_wrapper.md`、`docs/mcp.md`）。

最小可实现表达方式（不破坏现有事件结构）：

- 在 `ApprovalRequested.params` 里加入一个稳定字段（建议命名空间化，避免污染 action params）：
  - `{"approval": {"requirement": "prompt" | "prompt_strict", "source": "execpolicy"}}`
- `approval handling` 对 `prompt_strict` 的策略矩阵：
  - `auto_approve/on_request/unless_trusted/manual`：一律进入人工审批（行为等价 `manual`）
  - `auto_deny`：仍直接拒绝（并落盘 decided）
- `remember`（写死实现口径）：当 `approval.requirement="prompt_strict"` 时，不会生成 remembered decision，也不会复用 remembered decision。

验收（现实现状）：

- 在 `approval_policy=auto_approve` 下触发 `prompt_strict`：
  - 必须进入 `NeedApproval`，且不会自动写入 `ApprovalDecided(Approved)`。
- `omne inbox` / `omne watch` 必须可见 pending approvals（含 `approval.requirement` 字段）。

---

## 6) 快速自检（实现/文档一致性）

```bash
rg -n \"enum ApprovalPolicy\" crates/agent-protocol/src/lib.rs
rg -n \"gate_approval\\(\" crates/app-server/src/main/approval.rs
rg -n \"remembered_approval_decision\" crates/app-server/src/main/approval.rs
```
