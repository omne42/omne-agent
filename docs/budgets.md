# Budgets / 超时 / Stuck（v0.2.0 口径）

> 目标：把“烧钱失控/卡死”变成**可配置、可观测、可回放**的事实，而不是靠人盯日志。

---

## 0) 现实现状：budget/timeout → `TurnStatus::Stuck`

v0.2.0 已实现的硬规则：

- 当 agent turn 触发任一预算/超时错误时：
  - 该 turn 以 `TurnCompleted { status: Stuck, reason }` 结束（reason 为错误字符串）。
  - `thread/attention` 进入 `attention_state=stuck`，`omne-agent watch --bell` / `omne-agent inbox --bell` 会提醒。

对照实现（仅用于核对）：

- `crates/app-server/src/agent/core.rs`（预算与超时触发点）
- `crates/app-server/src/main/preamble/server.rs`（错误 → `TurnStatus` 分类）

---

## 1) 已实现的预算项（env 配置）

### 1.1 Turn 级预算（来自 agent loop）

| 预算 | env | 默认 | 触发时的 reason（示例） |
| --- | --- | --- | --- |
| steps 上限 | `OMNE_AGENT_MAX_STEPS` | `24` | `budget exceeded: steps` |
| tool calls 上限 | `OMNE_AGENT_MAX_TOOL_CALLS` | `128` | `budget exceeded: tool_calls` |
| turn 时长 | `OMNE_AGENT_MAX_TURN_SECONDS` | `600` | `budget exceeded: turn_seconds` |
| OpenAI 单次请求超时 | `OMNE_AGENT_MAX_OPENAI_REQUEST_SECONDS` | `120` | `openai request timed out` |
| token 总预算（可选） | `OMNE_AGENT_MAX_TOTAL_TOKENS` | `0`（禁用） | `token budget exceeded: used … > limit …` |

备注：

- `OMNE_AGENT_MAX_TOTAL_TOKENS=0` 表示不启用 token 预算（只靠 steps/tool_calls/time 限流）。
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

- agent turn 的“没输出也没结束”已经由 `OMNE_AGENT_MAX_TURN_SECONDS` 兜底（超时 → `Stuck`）。
- 后台进程的“没输出也没结束”已实现：`thread/attention.stale_processes` + `OMNE_AGENT_PROCESS_IDLE_WINDOW_SECONDS` + `omne-agent * --bell`（见 `docs/notifications.md`）。

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
  - `omne-agent inbox --watch` / `omne-agent thread attention <thread_id>`
  - `omne-agent process list --thread-id <thread_id>` + `omne-agent process tail <process_id>`
  - 调整 budget env（只列出当前命中的那一项，例如 `OMNE_AGENT_MAX_TURN_SECONDS`）

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
- omne-agent thread attention <thread_id>
- omne-agent process list --thread-id <thread_id>
- omne-agent process tail <process_id>
```

---

## 5) 已实现：loop/cycle detection（别烧到预算才发现）

v0.2.0 最小实现（写死边界）：

- 只在单个 turn 内做检测（不跨 turn 建模/不跨 turn 记忆）。
- 检测信号（固定）：
  - **连续重复**：相同 tool call 签名连续出现 `N=3` 次；
  - **短周期**：检测 `L=2` 的短周期（ABAB）。
- 触发后直接结束 turn：`TurnStatus::Stuck`，reason 以 `loop_detected:` 开头（例如 `loop_detected: consecutive` / `loop_detected: cycle`）。

签名（实现口径）：

- 使用 `u64` hash（只保留元数据），避免把大 JSON/大文本留在内存里。
- 构成：`tool_name + args_json` 的稳定 hash（不包含 `approval_id` 等易变字段；approval id 属于 app-server 内部 gate，不会出现在 model 提供的 args 里）。

对照实现：

- `crates/app-server/src/agent/core.rs`（`LoopDetector` + `tool_call_signature`）

---

## 6) 已实现：auto compact/summary（降低 token 风险）

v0.2.0 最小实现（写死边界）：

- 触发条件（优先）：当**当前请求上下文**（instructions + in-memory items）估算 token 达到模型的 `best_context` 时开始压缩（别名：`auto_compact_token_limit`；未配置 `best_context` 时，默认≈`max_context * 90%`，别名：`context_window`）。
- 触发条件（fallback）：当模型 context window 未知时，才会回退到 `OMNE_AGENT_MAX_TOTAL_TOKENS>0` 且 turn 内累计 token 使用量达到阈值时（默认 `90%`；可用 `OMNE_AGENT_AUTO_SUMMARY_THRESHOLD_PCT` 覆盖）。
- 产物：用 `artifact/write` 写入 `artifact_type="summary"`（文本会自动脱敏），且 provenance 指向触发的 `turn_id`。
- 上下文重建：当 thread 存在 `summary` artifact 时，后续 turn 构建上下文会优先使用：
  - `user` summary（带免责声明）+ summary 之后最近 `K` 条事件（默认 `200`；可用 `OMNE_AGENT_SUMMARY_CONTEXT_EVENT_LIMIT` 覆盖）
  - 避免每个 turn 都把全量历史塞回模型导致继续膨胀。
- 本次 turn 的后续请求：触发 summary 后，会把当前 in-memory 上下文压缩为 `user` summary（带免责声明）+ 最近少量 tail items（默认 `20`；可用 `OMNE_AGENT_AUTO_SUMMARY_TAIL_ITEMS` 覆盖；同时受 token 预算约束）。
- tool output 处理（参考 opencode）：触发阈值时会先 prune 老的 `function_call_output`（保留结构/`call_id`，但清掉大 output），再尝试 summary compact。

相关参数（可选）：

- `.omne_agent_data/config.toml`（`[openai.models]`）：
  - `max_context`（别名：`context_window`）：模型最大上下文（tokens）。
  - `best_context`（别名：`auto_compact_token_limit`）：超过后触发压缩（tokens；可不配）。
- `OMNE_AGENT_AUTO_SUMMARY_SOURCE_MAX_CHARS`：生成 summary 时用于拼接 transcript 的最大字符数（默认 `50000`）。

仍是 TODO：

- “切 long-context 模型”（Router 的上下文阈值路由；草案见 `docs/model_routing.md`）。

---

## 7) 快速验证（可复制）

> 需要 `OPENAI_API_KEY`（或 `OMNE_AGENT_OPENAI_API_KEY`）。

```bash
# 1) 把 turn 超时压到 1 秒，制造 Stuck
OMNE_AGENT_MAX_TURN_SECONDS=1 cargo run -p omne-agent -- ask "ping" --json

# 2) 找到 thread 并查看最后一个 turn 状态/原因
# （如果你习惯用 app-server JSON-RPC，可用 thread/list_meta + thread/attention）
cargo run -p omne-agent -- thread list
```

验收：

- 当 turn 进入 `Stuck` 时，`omne-agent artifact list <thread_id>` 必须能看到 `artifact_type="stuck_report"` 的产物，且 provenance 指向该 `turn_id`。
