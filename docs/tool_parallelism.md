# Tool Parallelism（read-only 并发）（v0.2.0 口径）

> 目标：在不引入 side effects 风险的前提下，加速“多条只读查询”类工具调用（read/glob/grep/inspect/tail…）。
>
> 约束：并发只发生在 **read-only tools**，并且默认关闭。

---

## 0) 何时会并发（已实现）

当满足全部条件时，app-server 会在一个 turn 内并发执行 tool calls：

1. `CODE_PM_AGENT_PARALLEL_TOOL_CALLS=true`
2. 同一次模型响应里包含 **2 个或以上** tool calls
3. 这些 tool 都被判定为 read-only（见下文列表）

否则：按 tool call 顺序串行执行。

实现对照：`crates/app-server/src/agent/core.rs`。

---

## 1) read-only tool 列表（当前实现，保守）

当前实现把以下 tool name 视为 read-only（其它一律不并发）：

备注（命名口径）：

- 这里的 tool name 是 agent tool id（snake_case），用于模型 function call；与 JSON-RPC 方法名一一对应（slash 形式），例如 `file_read` ↔ `file/read`、`process_inspect` ↔ `process/inspect`、`thread_events` ↔ `thread/events`。

- `file_read`
- `file_glob`
- `file_grep`
- `process_inspect`
- `process_tail`
- `process_follow`
- `artifact_list`
- `artifact_read`
- `thread_state`
- `thread_events`

备注：

- 列表是**保守**的：不确定是否只读的工具不要并发（避免引入隐式竞态）。

---

## 2) 输出与事件顺序（重要边界）

- 并发执行时，tool 的实际完成顺序不固定（`buffer_unordered`）。
- 但返回给模型的 `FunctionCallOutput` 会按原始 tool call 的顺序拼接回去（保证“输入输出对齐”）。
- 落盘事件（`ToolStarted/ToolCompleted`）可能交错出现；正确性来源是 `EventSeq` 单调递增（见 `docs/thread_event_model.md`）。

---

## 3) 配置（env）

- `CODE_PM_AGENT_PARALLEL_TOOL_CALLS`：默认 `false`
- `CODE_PM_AGENT_MAX_PARALLEL_TOOL_CALLS`：默认 `8`，最大 `128`

建议：

- 只在“明显 I/O 查询瓶颈”的场景打开；默认保持关闭以降低并发带来的可观测噪声。
