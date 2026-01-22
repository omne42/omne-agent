# Subagents / Fan-out（`thread/fork` + `agent_spawn`）（v0.2.0 现状 + TODO）

> 目标：支持把一个大任务拆成多个子任务并发推进（fan-out），并且每个子任务都有独立的事件流与可观测性。
>
> 现状：v0.2.0 已有 `thread/fork`（控制面）与 `agent_spawn`（agent tool）这两个原语，但离“完整 scheduler + fan-in”还差一截。

---

## 0) 范围与非目标

范围：

- 明确 `thread/fork` 与 `agent_spawn` 的现实现状（复制哪些历史、跳过哪些事件）。
- 给出“安全使用”的边界（别让并发把工作区写烂）。
- 写出 fan-out/fan-in 的最小 TODO 规格占位。

非目标（v0.2.0）：

- 不做完整 scheduler（队列/优先级/worker pool/公平性）。
- 不做自动 fan-in 汇总与 reviewer gate（先留接口）。
- 不保证 workspace 隔离（这是后续需要补的硬能力）。

---

## 1) `thread/fork`（已实现，重要边界）

JSON-RPC 方法：`thread/fork`（实现对照：`crates/app-server/src/main/thread_manage/fork.rs`）。

### 1.1 fork 会复制什么

fork 会在同一个 `cwd` 下创建一个新 thread，并把部分事件复制到新 thread 的事件流：

- ✅ `ThreadConfigUpdated`（配置层）
- ✅ `TurnStarted/TurnCompleted/TurnInterruptRequested`
- ✅ `ApprovalRequested/ApprovalDecided`
- ✅ `AssistantMessage`

### 1.2 fork 不会复制什么（危险点）

- ❌ 不复制 `ToolStarted/ToolCompleted`
- ❌ 不复制 `ProcessStarted/Exited/...`
- ❌ 不复制 `ThreadArchived/ThreadPaused/...`（forked thread 会以“未归档/未暂停”的初始状态出现）
- ❌ 不复制 active turn（正在运行的 turn 的事件会被跳过）

结论：

- fork 的语义更像“复制对话/审批/结论”，不是“复制完整可回放上下文”（工具与进程历史会缺失）。
- active turn 的筛选是“按事件类型 + turn_id”做的：`ApprovalRequested{turn_id=Some(active_turn_id)}` 会被跳过，但 `ApprovalDecided` 目前不会随 active turn 一起过滤，可能出现“有决定但缺少对应请求上下文”的残片。

### 1.3 workspace 隔离（当前没有）

fork 出来的 thread **仍然使用同一个 `cwd`**。如果两个 thread 并发写文件，会直接产生竞态与互相覆盖风险。

当前建议的安全边界：

- 把 fork/agent_spawn 当作 **并发只读分析**（read-only tools）使用。
- 真要并发写代码：必须先做 workspace 隔离（worktree/tmp dir），这属于后续 TODO（见 `docs/workspace_hooks.md` 与 parity 的 scheduler TODO）。

---

## 2) `agent_spawn`（已实现：fork + 后台 turn）

`agent_spawn` 是 agent tool，定义在 tool catalog（实现对照：`crates/app-server/src/agent/tools/catalog.rs`、`crates/app-server/src/agent/tools/dispatch.rs`）。

补充：CLI 也提供了等价的便捷命令 `pm thread spawn <thread_id> "<input>" [--model ...] [--openai-base-url ...]`，其实现同样是 `thread/fork` +（可选）`thread/configure` + `turn/start`（对照：`crates/agent-cli/src/main/process_and_utils.rs`）。

行为（现状）：

1. 对当前 thread 执行 `thread/fork`
2. （可选）对 forked thread 执行 `thread/configure` 设置 `model/openai_base_url`
3. 在 forked thread 上启动一个新的 agent turn（后台执行）

返回值（现状）：

- `thread_id`（forked）
- `turn_id`（新 turn）
- `log_path`（forked thread 的 events.jsonl 路径）
- `last_seq`（fork 时刻的 seq）

可观测性：

- forked thread 会出现在 `thread/list_meta` 与 `pm inbox --watch` 中；可用 `thread/subscribe` 重放/追踪事件（见 `docs/thread_event_model.md`）。

---

## 3) TODO：fan-out / fan-in 的最小规格占位

### 3.1 fan-out（TODO）

最小目标：

- orchestrator 能把一个 workflow 拆成多个子任务，并用 `agent_spawn`（或等价机制）并发执行。

最小约束：

- 并发任务默认只读；写操作必须在隔离 workspace 内执行（否则就是数据竞争）。
- 若请求“需要写”的子任务但系统没有隔离能力：必须直接失败（不能悄悄降级成共享写）。
- 每个子任务必须能定位 provenance（至少 `thread_id/turn_id`；建议子任务写出一个 `artifact_type="fan_out_result"` 作为结果锚点）。

最小输入/输出（v1 建议写死）：

- 输入（fan-out task input）：
  - `task_id`：稳定标识（用于 fan-in 汇总）
  - `instruction`：子任务描述（纯文本）
  - `workspace_mode`：`read_only | isolated_write`
  - `expected_artifact_type`：建议为 `fan_out_result`
- 输出（spawn handle）：
  - `task_id`
  - `thread_id`（forked）
  - `turn_id`（新 turn）
  - `log_path`（events.jsonl 路径）
  - `last_seq`（fork 时刻 seq）

### 3.2 fan-in（TODO）

最小目标：

- 把多个子任务的结果汇总成一个 user artifact（例如 `artifact_type="fan_in_summary"`）。

最小约束：

- 汇总必须带 provenance：引用哪些 threads/turns/artifacts（见 `pm_protocol::ArtifactProvenance`）。
- 汇总不塞进事件；写成 artifact（见 `docs/artifacts.md`）。

fan-in artifact 模板（v1 建议写死成 Markdown，便于人读/脚本解析）：

```md
# Fan-in Summary

Summary: ...

Sources:
| task_id | thread_id | turn_id | artifact_id |
| --- | --- | --- | --- |
| T1 | th_... | tu_... | ar_... |

Findings:
- ...
```

### 3.3 v1 推荐路径（避免过度设计）

推荐 v1 先走“客户端编排”（不引入 scheduler）：

- fan-out：复用 `thread/fork` + `agent_spawn`。
- 每个子任务完成时：写出 `artifact_type="fan_out_result"`（作为可定位的结果锚点）。
- fan-in：由 orchestrator 收集这些 `artifact_id`，写一个 `artifact_type="fan_in_summary"` 的汇总 artifact。

DoD（未来实现时）：

- fan-out 产生的每个子任务都能在 `pm inbox` 里定位到 `thread_id/turn_id`，并可用 `thread/subscribe` 重放事件。
- fan-in 汇总 artifact 内容必须显式列出来源 `thread_id/turn_id/artifact_id`，且 metadata 的 provenance 至少包含汇总发生的 `thread_id/turn_id`。

### 3.4 CLI/API（未来实现占位）

CLI（占位）：

```bash
# fan-out：启动一个子任务（等价 thread spawn + 约定写出 fan_out_result）
pm thread spawn <thread_id> "<input>" --json

# fan-in：收集多个 fan_out_result 并写汇总
# （v1 可先由 orchestrator 手工调用 artifact/write 完成）
```

---

## 4) 快速自检（实现/文档一致性）

```bash
rg -n \"handle_thread_fork\" crates/app-server/src/main/thread_manage/fork.rs
rg -n \"agent_spawn\" crates/app-server/src/agent/tools/dispatch.rs
```
