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

输入（v0.2.x，agent tool）：

- `input`（必填）
- `mode`（可选；默认 `reviewer`；受 `modes.yaml` 的 `subagent.spawn.allowed_modes` 限制）
- `workspace_mode`：`read_only | isolated_write`（可选；默认 `read_only`；`isolated_write` 当前 hard-fail）
- `model/openai_base_url`（可选）

行为（现状）：

1. `mode gate`：按当前 thread mode 的 `subagent.spawn` 决策（`deny/prompt/allow`）判定是否允许 spawn
2. `guard`：并发上限 `CODE_PM_MAX_CONCURRENT_SUBAGENTS`（默认 `4`；`0` 表示不限制）
3. 对当前 thread 执行 `thread/fork`
4. 对 forked thread 执行 `thread/configure`：默认设置 `sandbox_policy=read_only` + `mode=<child_mode>`（并可设置 `model/openai_base_url`）
5. 在 forked thread 上启动一个新的 agent turn（后台执行）

返回值（现状）：

- `thread_id`（forked）
- `turn_id`（新 turn）
- `log_path`（forked thread 的 events.jsonl 路径）
- `last_seq`（fork 时刻的 seq）

可观测性：

- forked thread 会出现在 `thread/list_meta` 与 `pm inbox --watch` 中；可用 `thread/subscribe` 重放/追踪事件（见 `docs/thread_event_model.md`）。

---

## 3) TODO：fan-out / fan-in 的最小规格占位

### 3.0 参考实现对比（我们要抄的“行为”，不是抄代码）

#### Codex（`example/codex`，重点：`rust-v0.88.0`）

- **硬限制（guard）**：`agents.max_threads` 限制“一个 user session 同时开多少 subagent threads”（对照：`example/codex/codex-rs/core/src/agent/guards.rs`、`example/codex/codex-rs/core/src/agent/control.rs` 的 `reserve_spawn_slot/commit/release_spawned_thread`）。
- **交互事件桥接**：`codex_delegate` 会把 subagent 的“需要人类介入的事件”路由回父 session：
  - `ExecApprovalRequest` / `ApplyPatchApprovalRequest`（已有）+ `RequestUserInput`（`rust-v0.88.0` 新增）
  - 对照：`example/codex/codex-rs/core/src/codex_delegate.rs`
- **`request_user_input` 工具**：subagent 通过 function tool 触发 `EventMsg::RequestUserInput`，父 session 负责展示/收集回答，再回填 `Op::UserInputAnswer`（对照：`example/codex/codex-rs/core/src/tools/handlers/request_user_input.rs`、`example/codex/codex-rs/core/src/codex.rs::request_user_input/notify_user_input_response`）。
- **错误语义**：超限直接拒绝 spawn（`AgentLimitReached`），而不是“排队等着”（对照：`example/codex/codex-rs/core/src/error.rs`）。
- **可观测性**：subagent 请求会带 `x-openai-subagent` header（`SessionSource::SubAgent(SubAgentSource::Review)`），便于后端侧分组统计（对照：`example/codex/codex-rs/codex-api/src/requests/*`）。

我们该抄的点：

- subagent **必须有并发上限**（防止递归 spawn / 资源耗尽），并且要“预留 slot → 成功后 commit → shutdown 后释放”的语义，避免失败路径泄露配额。
- subagent 的 **approval / user-input 必须可被主 thread 看到并可操作**（不一定要“事件桥接”，但用户不能被迫在 10 个 thread 之间猜哪个在等输入）。

#### Antigravity（`antigravity.google`：只采用“官方能确认”的口径）

- **Browser Subagent**：当主 agent 需要操作浏览器时，会调用 browser subagent；它使用专门模型并运行在独立 browser profile 内做隔离（官方文档：`https://antigravity.google/docs/browser-subagent`、`https://antigravity.google/docs/browser`）。
- **Task Groups**：规划模式下把复杂任务拆成多个子任务，并“经常同时推进多个部分”（官方文档：`https://antigravity.google/docs/task-groups`）。
- **Agent Manager**：强调“跨多个 workspaces 同时管理很多 agents”（官方文档：`https://antigravity.google/docs/agent-manager`）。

我们该抄的点：

- subagent 类型不需要多，但每一种都要有**清晰的隔离边界**（browser profile 就是一个现实可执行的隔离单元）。
- 对“用户可自定义 subagent”保持保守：没有可验证 spec 就别把它当成硬依赖能力。

#### OpenCode（`example/opencode`，重点：Task tool + subagent sessions）

- **Task = 创建一个 child session**（带 `parentID`），并把 `session_id` 作为结果锚点返回（对照：`example/opencode/packages/opencode/src/tool/task.ts`）。
- **subagent 类型是数据结构**：`Agent` 明确区分 `mode: subagent|primary|all`，并为每个 subagent 定义最小权限集（例如 `explore` 只允许 grep/glob/read 等）（对照：`example/opencode/packages/opencode/src/agent/agent.ts`）。
- **可恢复（resume）**：Task tool 接受 `session_id` 继续跑同一个 subagent session（对照：`example/opencode/packages/opencode/src/tool/task.ts` 的 `params.session_id`）。
- **进度摘要**：通过事件总线订阅 tool part 更新，把“子任务做了什么/卡在哪”实时汇总到 metadata（对照：同文件的 `Bus.subscribe(MessageV2.Event.PartUpdated, ...)`）。
- **并行/调度现实**：subtask 的“任务载体”是消息流里的 `SubtaskPart`，prompt loop 以 LIFO 串行消费；真正并行更多来自 `BatchTool` / `parallel_tool_calls`（对照：`example/opencode/packages/opencode/src/session/prompt.ts`、`example/opencode/packages/opencode/src/tool/batch.ts`）。

我们该抄的点：

- fan-out 的每个子任务需要一个**稳定可引用的 handle**（`task_id + thread_id/turn_id`，或 `task_id + session_id`），并且支持“继续跑/重试”。
- subagent 的“角色/权限”应该是**显式字段**（而不是纯靠 prompt 约束）。

#### Claude Code（`example/claude-code`，重点：Task + agents + hooks）

- **subagent 定义是 Markdown + YAML frontmatter**（name/description+examples/model/tools/color），便于插件化分发与版本化（对照：`example/claude-code/plugins/plugin-dev/skills/agent-development/SKILL.md`）。
- **Task tool 作为统一入口**：命令/skill 里通过 Task 调起 subagent（例如 hookify 用 Task 先跑 conversation-analyzer）（对照：`example/claude-code/plugins/hookify/commands/hookify.md`）。
- **fork context + agent 选择**：skill/command 可声明 `context: fork` 让它在 forked sub-agent context 执行，并可在 frontmatter 指定 `agent` 类型（对照：`example/claude-code/CHANGELOG.md` 的 `2.1.0` 条目）。
- **生命周期 hooks**：有 `SubagentStop`，changelog 里还提到 `SubagentStart` 与 `agent_id/agent_transcript_path`（对照：`example/claude-code/CHANGELOG.md`）。

我们该抄的点：

- subagent 生命周期必须有 hook 点（至少 `SubagentStart/SubagentStop`），否则 fan-out/fan-in 做不好可审计与自动化收口。

### 3.1 fan-out（TODO）

最小目标：

- orchestrator 能把一个 workflow 拆成多个子任务，并用 `agent_spawn`（或等价机制）并发执行。

最小约束：

- 并发任务默认只读；写操作必须在隔离 workspace 内执行（否则就是数据竞争）。
- 若请求“需要写”的子任务但系统没有隔离能力：必须直接失败（不能悄悄降级成共享写）。
- 子任务并发必须有硬上限（防止递归 spawn 把机器打爆）：建议做成 `max_concurrent_subagents`（全局/每 thread 二选一，但必须可配置且可审计）。
- 每个子任务必须能定位 provenance（至少 `thread_id/turn_id`；建议子任务写出一个 `artifact_type="fan_out_result"` 作为结果锚点）。

最小输入/输出（v1 建议写死）：

- 输入（fan-out task input）：
  - `task_id`：稳定标识（用于 fan-in 汇总）
  - `mode`：子任务角色（例如 `architect/coder/reviewer/builder`；若 v1 不支持，必须明确写死为只读角色）
  - `model`（可选）：子任务强制路由（对齐 `docs/model_routing.md` 的优先级规则）
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
  - 子任务并发上限生效：超限时必须返回可理解的错误，并在事件/日志中留下“拒绝 spawn 的原因”。

### 3.3.1 v0.2.x 务实落地建议（先只读，后隔离写）

在 workspace 隔离能力落地前，**fan-out 只能是并发只读分析**；否则就是把数据竞争包装成“功能”。

建议 DoD（按优先级）：

- [x] `agent_spawn` 支持显式 `mode`（或等价字段），并默认选择只读角色（例如 `architect/reviewer`）；并由 `modes.yaml` 提供 `subagent.spawn` gate（不允许任意模式随意 spawn）。
- [x] 引入 `max_concurrent_subagents`（全局或 per-thread 二选一即可），并在事件/错误里记录“为什么拒绝 spawn”（对齐 Codex 的 guard 思路）。
- [ ] 子任务必须写出 `artifact_type="fan_out_result"`，且内容/metadata 至少包含 `task_id + thread_id + turn_id`（作为 fan-in 的唯一锚点）。
- [x] `workspace_mode=isolated_write` 在未实现隔离前必须 hard-fail（不要隐式降级到共享写）。
- [ ] 为 fan-out/fan-in 预留 hook 点（至少 `SubagentStart/SubagentStop`），用于后续自动化收口与审计。

### 3.3.2 方案选型（A/B/C，避免“边写边发明”）

- **A：客户端编排（只读 fan-out）**：复用 `thread/fork + agent_spawn`，子任务只读并写 `fan_out_result`，fan-in 由 orchestrator 写 `fan_in_summary`；审批/输入不做桥接（依赖 `pm inbox` 聚合）。
- **B：服务端 guard + bridge**：服务端实现“预留/提交/释放 slot”的并发上限，并把子任务 `ApprovalRequested`/（未来）`UserInputRequested` 以可审计方式桥接到父 thread（类似 Codex delegate）；复杂度更高但 UX 更好。
- **C：隔离 workspace（可写 fan-out）**：每个子任务独立 worktree/clone；允许 `isolated_write`，并需要定义“如何回填 patch/如何处理冲突”。

推荐取舍：

- v0.2.x 优先做 **A**（先把结果锚点/provenance/只读边界做实），并尽早补上 **guard**（无论放客户端还是服务端，都必须是硬限制）。
- **B/C** 放到“有真实需求 + 有隔离能力”之后再做，否则只是在制造不可复现的竞态。

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
