# Budgets / 超时 / Stuck（v0.2.0 口径）

> 目标：把“烧钱失控/卡死”变成**可配置、可观测、可回放**的事实，而不是靠人盯日志。

---

## 0) 现实现状：budget/timeout → `TurnStatus::Stuck`

v0.2.0 已实现的硬规则：

- 当 agent turn 触发任一预算/超时错误时：
  - 该 turn 以 `TurnCompleted { status: Stuck, reason }` 结束（reason 为错误字符串）。
  - `thread/attention` 进入 `attention_state=stuck`，`pm watch --bell` / `pm inbox --bell` 会提醒。

对照实现（仅用于核对）：

- `crates/app-server/src/agent/core.rs`（预算与超时触发点）
- `crates/app-server/src/main/preamble.rs`（错误 → `TurnStatus` 分类）

---

## 1) 已实现的预算项（env 配置）

### 1.1 Turn 级预算（来自 agent loop）

| 预算 | env | 默认 | 触发时的 reason（示例） |
| --- | --- | --- | --- |
| steps 上限 | `CODE_PM_AGENT_MAX_STEPS` | `24` | `budget exceeded: steps` |
| tool calls 上限 | `CODE_PM_AGENT_MAX_TOOL_CALLS` | `128` | `budget exceeded: tool_calls` |
| turn 时长 | `CODE_PM_AGENT_MAX_TURN_SECONDS` | `600` | `budget exceeded: turn_seconds` |
| OpenAI 单次请求超时 | `CODE_PM_AGENT_MAX_OPENAI_REQUEST_SECONDS` | `120` | `openai request timed out` |
| token 总预算（可选） | `CODE_PM_AGENT_MAX_TOTAL_TOKENS` | `0`（禁用） | `token budget exceeded: used … > limit …` |

备注：

- `CODE_PM_AGENT_MAX_TOTAL_TOKENS=0` 表示不启用 token 预算（只靠 steps/tool_calls/time 限流）。
- 上述预算命中会直接 `Stuck`（不会悄悄继续乱改）。

### 1.2 Tool 调用内的防循环预算（固定值）

v0.2.0 还有一类“防止审批循环”的硬上限（当前不可配置）：

- 同一次 tool 调用如果连续出现 `needs_approval`，最多允许 3 次循环；超出会触发：
  - `budget exceeded: approval_cycles` → turn 进入 `Stuck`

对照实现：

- `crates/app-server/src/agent/tools/dispatch.rs`（`approval_cycles`）

---

## 2) 重要边界（避免误解）

- 预算/超时是针对 **agent 的 turn**，不是针对 **后台进程**：进程目前没有统一的超时/idle 检测（可手动 `process/inspect`、`process/tail/follow`、`process/kill`）。
- `Stuck` 的保证来自落盘事件：即使订阅掉线，也能 `thread/subscribe` 从 `since_seq` 重放补齐。

---

## 3) 已实现：补齐“没输出也没结束”的可见性

v0.2.0 现状：

- agent turn 的“没输出也没结束”已经由 `CODE_PM_AGENT_MAX_TURN_SECONDS` 兜底（超时 → `Stuck`）。
- 后台进程的“没输出也没结束”已实现：`thread/attention.stale_processes` + `CODE_PM_PROCESS_IDLE_WINDOW_SECONDS` + `pm * --bell`（见 `docs/notifications.md`）。

最小规格（写死边界）：

- 不引入 stdin/PTY 交互；进程一律非交互。
- 若要增加 idle 检测，必须落到 `thread/attention` 的可枚举字段（例如 `stale_processes`），并触发 bell（去重/节流）。

---

## 4) 已实现：解释性 artifacts（让 Stuck 可行动）

问题：只有 `TurnCompleted{status=stuck, reason}` 还不够，用户需要“下一步该干什么”。

最小规格：

- 当一个 turn 结束为 `Stuck` 时，系统自动写入一个 user artifact（建议 1 turn 生成 1 份，靠 provenance 关联 turn）：
  - `artifact_type="stuck_report"`
  - `summary` 包含：原因摘要 + 下一步建议（例如“检查 pending approvals / tail 进程输出 / 调大 budget”）
  - 内容必须走脱敏（见 `docs/redaction.md`）
- 报告至少包含可点击的定位信息：
  - `thread_id`、`turn_id`
  - 最近的 `tool_id` / `process_id`（如有）
  - 相关 artifacts 路径（stdout/stderr/user artifacts）

建议内容结构（别写成一堆废话）：

- **What happened**：`TurnStatus=Stuck` + `reason`
- **Where to look**：
  - 最近一个 `ApprovalRequested`（如果存在）
  - 最近一个 running process 的 `stdout_path/stderr_path`（如果存在）
  - 最近一个 tool 的 `tool_id/tool name`（如果存在）
- **Next actions**（给可复制命令）：
  - `pm inbox --watch` / `pm thread attention <thread_id>`
  - `pm process list --thread <thread_id>` + `pm process tail <process_id>`
  - 调整 budget env（只列出当前命中的那一项，例如 `CODE_PM_AGENT_MAX_TURN_SECONDS`）

最小模板（建议写死成 Markdown，便于人读/脚本解析）：

```md
# Stuck report

## What happened
- thread_id: ...
- turn_id: ...
- status: stuck
- reason: ...

## Where to look
- last_approval_id: ... (optional)
- last_tool: ... (optional)
- last_process_id: ... (optional)
- stdout_path: ... (optional)
- stderr_path: ... (optional)

## Next actions
- pm thread attention <thread_id>
- pm process list --thread <thread_id>
- pm process tail <process_id>
```

---

## 5) TODO：loop/cycle detection（别烧到预算才发现）

最小规格（TODO）：

- 只在单个 turn 内做检测（不跨 turn 建模/不跨 turn 记忆）。
- 检测到明显循环时直接结束 turn：`TurnCompleted{status=stuck, reason="loop_detected"}`（先别把 “cycle/loop” 拆成一堆状态机）。

建议的最小检测信号（实现可选其一，但必须稳定）：

- **连续重复**：相同的“步骤签名”连续出现 N 次（例如同一工具以相同参数反复调用）。
- **短周期**：最近 `L` 个签名与前一个 `L` 完全相同（例如 ABAB；L 取 2~4 的小常数即可）。

建议默认（别让实现发散）：

- 连续重复：`N=3`
- 短周期：`L=2`（检测 ABAB）

签名（建议）：

- 用一个小的纯值类型（例如 `u64` hash），避免把大 JSON/大文本留在内存里。
- 建议构成（按可得性从低到高选）：
  - tool call：`tool_name + redacted(params_json)`（不含 approval_id 等易变字段）
  - 或加上：`tool_status`（completed/denied/failed）
  - 或加上：assistant 输出文本的 hash（更容易误判，谨慎）

安全边界（写死）：

- loop 检测相关的落盘内容必须是**元数据**，禁止把 raw prompt / tool args / tool output 写进事件或报告（脱敏不确定就等于泄漏）。

可选产物（建议，但不强依赖）：

- 生成 `artifact_type="loop_report"`（markdown，脱敏），只包含：
  - `thread_id/turn_id`
  - 触发原因（`loop_detected` + 命中的信号类型）
  - 最近几个 tool 名称与 `tool_id`（不含参数）

---

## 6) TODO：auto compact/summary（降低 token 风险）

最小规格（TODO）：

- 目标：把“长上下文导致烧钱/卡死”变成可控行为，而不是隐式退化。
- 建议触发条件：当 `CODE_PM_AGENT_MAX_TOTAL_TOKENS>0` 且已使用量接近阈值（例如 80%）时。
- 产物：用 `artifact/write` 写入 `artifact_type="summary"`（脱敏），并在后续请求用摘要重建上下文。
- 长上下文模型切换（`long-context`）属于 Router 范畴（TODO 草案见 `docs/model_routing.md`），与 compact 二选一即可；别一口气做完。

摘要内容边界（写死，防泄漏）：

- 不落盘 reasoning（用“行动/结论/下一步”的摘要代替）。
- 不内联大 tool 输出/日志：只写引用（`process_id/tool_id/artifact_id` + 路径/摘要），需要细节让用户去看 artifacts。
- 摘要必须走脱敏（`docs/redaction.md`），并建议在写入后再做一次“哨兵扫描”（命中 token 形态则替换为 `<REDACTED>`）。

验收（未来实现时）：

- 触发阈值后生成 `artifact_type="summary"`，且 provenance 指向该 `turn_id`（见 `pm_protocol::ArtifactProvenance`）。
- 后续 turn 的上下文构建能选择“summary + 最近 K 条事件”而不是全量历史（避免继续膨胀）。

---

## 7) 快速验证（可复制）

> 需要 `OPENAI_API_KEY`（或 `CODE_PM_OPENAI_API_KEY`）。

```bash
# 1) 把 turn 超时压到 1 秒，制造 Stuck
CODE_PM_AGENT_MAX_TURN_SECONDS=1 cargo run -p pm -- ask "ping" --json

# 2) 找到 thread 并查看最后一个 turn 状态/原因
# （如果你习惯用 app-server JSON-RPC，可用 thread/list_meta + thread/attention）
cargo run -p pm -- thread list
```

验收：

- 当 turn 进入 `Stuck` 时，`pm artifact list <thread_id>` 必须能看到 `artifact_type="stuck_report"` 的产物，且 provenance 指向该 `turn_id`。
