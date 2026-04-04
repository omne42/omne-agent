# Subagents / Fan-out（`thread/fork` + `agent_spawn`）（v0.2.0 现状 + TODO）

> 目标：支持把一个大任务拆成多个子任务并发推进（fan-out），并且每个子任务都有独立的事件流与可观测性。
>
> 现状：v0.2.0 已有 `thread/fork`（控制面）与 `agent_spawn`（agent tool）这两个原语；并已接入 `SubagentStart/SubagentStop` hooks。离“完整 scheduler + fan-in”还差一截。

---

## 0) 范围与非目标

范围：

- 明确 `thread/fork` 与 `agent_spawn` 的现实现状（复制哪些历史、跳过哪些事件）。
- 给出“安全使用”的边界（别让并发把工作区写烂）。
- 写出 fan-out/fan-in 的最小 TODO 规格占位。

非目标（v0.2.0）：

- 不做完整 scheduler（例如跨 thread 全局队列/抢占/配额）；但已实现最小的 DAG 依赖、priority 与 aging 公平调度，以及全局 LLM 并发限流（见 `OMNE_MAX_CONCURRENT_LLM_REQUESTS`/`OMNE_LLM_FOREGROUND_RESERVE`）。
- 不做自动 fan-in 汇总与 reviewer gate（先留接口）。
- 不做复杂的自动冲突修复策略（`isolated_write` 已提供 patch handoff；可选 auto-apply 仅做 `git apply --check && git apply` 的最小尝试，失败仍回落人工回填）。

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
- active turn 的筛选是“按事件类型 + turn_id + approval 关联”做的：`ApprovalRequested{turn_id=Some(active_turn_id)}` 会被跳过，且对应 `approval_id` 的 `ApprovalDecided` 也会被同步过滤，避免“有决定但缺少对应请求上下文”的残片。
  - `turn_id=None` 的审批事件不视为 active turn 事件，会随 fork 正常复制。

### 1.3 workspace 隔离（`thread/fork` 默认没有）

`thread/fork` 在未指定 `cwd` 时，fork 出来的 thread **仍然使用同一个 `cwd`**。如果两个 thread 并发写文件，会直接产生竞态与互相覆盖风险。

补充：控制面 `thread/fork` 已支持可选 `cwd` 覆盖。`agent_spawn(workspace_mode=isolated_write)` 会利用这个能力把子线程落到隔离工作区目录（默认 worktree，失败回退 copy），而不是父线程共享目录。

当前建议的安全边界：

- 把 fork/agent_spawn 当作 **并发只读分析**（read-only tools）是最稳妥默认。
- 真要并发写代码：使用 `workspace_mode=isolated_write`（已实现隔离工作区 + patch handoff，默认 worktree-first，失败回退 copy）；自动 apply/冲突处理仍是后续 TODO（见 `docs/workspace_hooks.md` 与 parity 的 scheduler TODO）。

---

## 2) `agent_spawn`（已实现：fork/new + DAG + 后台 turn）

`agent_spawn` 是 agent tool，定义在 tool catalog（实现对照：`crates/app-server/src/agent/tools/catalog.rs`、`crates/app-server/src/agent/tools/dispatch.rs`）。

补充：CLI 也提供了等价的便捷命令 `omne thread spawn <thread_id> "<input>" [--model ...] [--openai-base-url ...]`，其实现同样是 `thread/fork` +（可选）`thread/configure` + `turn/start`（对照：`crates/agent-cli/src/main/process_and_utils.rs`）。

输入（v0.2.x，agent tool）：

- `tasks[]`（必填；子任务列表）
  - `id`（必填；依赖与回溯的稳定标识）
  - `input`（必填）
  - `depends_on`（可选；依赖的 task id 列表）
  - `priority`（可选；`high | normal | low`；默认继承顶层或 `normal`）
  - `spawn_mode`（可选；`fork | new`；默认继承顶层）
  - `mode`（可选；默认继承顶层）
  - `workspace_mode`（可选；默认继承顶层）
  - `model/openai_base_url`（可选；默认继承顶层）
  - `expected_artifact_type`（可选；默认继承顶层）
- `spawn_mode`（可选；`fork | new`；默认 `new`）
- `mode`（可选；默认 `reviewer`；受 `modes.yaml` 的 `subagent.spawn.allowed_modes` 限制）
- `workspace_mode`：`read_only | isolated_write`（可选；默认 `read_only`）
- `priority`：`high | normal | low`（可选；默认 `normal`）
- `model/openai_base_url`（可选）
- `expected_artifact_type`（可选；默认 `fan_out_result`；用于结果锚点）

行为（现状）：

1. `mode gate`：按当前 thread mode 的 `subagent.spawn` 决策（`deny/prompt/allow`）判定是否允许 spawn
   - 若被 `mode gate`（含 `tool_overrides["subagent/spawn"]`）拒绝，返回体与 `ToolCompleted.result` 会带 `decision_source`（`mode_permission|tool_override`）与 `tool_override_hit`（bool）用于审计。
2. `guard`：并发上限由 `OMNE_MAX_CONCURRENT_SUBAGENTS`（默认 `4`；`0` 表示不限制）与当前 mode 的 `subagent.spawn.max_threads`（取值 `0..=64`，`0` 表示不限制）共同决定；两者都设置时取更严格值（更小者）
   - 达到上限时，返回体与 `ToolCompleted.result` 会包含 `limit_policy="min_non_zero"`、`limit_source`（`env|mode|combined|unlimited`）与 env/mode/effective 上限值，便于审计。
3. 按 `spawn_mode` 为每个 task 创建子 thread：
   - `fork`：`thread/fork`
   - `new`：`thread/start`（cwd 继承父 thread）
   - 当 `workspace_mode=isolated_write` 时，隔离目录固定为：`<omne_root>/tmp/subagents/<parent_thread_id>/<task-id>-<nonce>/repo`（实现上取 `thread_store.root()`，不再从 `server.cwd` 反推）
   - 默认策略为 `OMNE_SUBAGENT_ISOLATED_BACKEND=auto`：先尝试 `git worktree add --detach`；若失败则自动回退 copy（`backend=copy` 且记录 `fallback_reason`）
   - 可通过 `OMNE_SUBAGENT_ISOLATED_BACKEND=worktree|copy|auto` 强制策略：`worktree` 失败即报错，不回退
   - 兼容旧开关：`OMNE_SUBAGENT_ISOLATED_WORKTREE_FIRST=0` 等价于 `copy`
   - 上述流程只调用本地 `git` 命令，不启动本地 Git 服务（无 daemon/smart-http 进程）
4. 对子 thread 执行 `thread/configure`：
   - `workspace_mode=read_only` -> `sandbox_policy=read_only`
   - `workspace_mode=isolated_write` -> `sandbox_policy=workspace_write`
   - 并统一设置 `mode=<child_mode>`（可选覆盖 `model/openai_base_url`）
5. 按依赖关系启动子任务：仅当 `depends_on` 全部为 `Completed` 时才启动；在可运行任务里会优先启动更高 `priority`，并对 ready 任务维护等待轮次（aging，默认 `OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS=3`，每等待 N 轮提升一级有效优先级）以降低低优先级饥饿风险；同有效优先级按声明顺序。若任一依赖以非 `Completed` 结束，后继任务会被标记为失败（`error=blocked by dependency: ...`）且不再启动（后台执行）
   - turn priority：subagent turns 使用 `priority=background`；全局 LLM worker pool 会优先保障 foreground（见 `OMNE_MAX_CONCURRENT_LLM_REQUESTS`/`OMNE_LLM_FOREGROUND_RESERVE`）。
   - scheduler 会在启动时和 notify channel lagged 时做一次基于事件日志的 catch-up（补抓取 `TurnCompleted` 与审批事件），避免因时序窗口漏记状态。
   - 生命周期 hooks：子任务 turn 启动后触发 `subagent_start`；子任务 turn 完成后触发 `subagent_stop`（配置见 `docs/hooks.md`）。
6. 当子 thread 的该 turn 结束（`TurnCompleted`）时，写出一个 user artifact 作为结果锚点（默认 `artifact_type="fan_out_result"`）
   - `fan_out_result` 现在带结构化 payload（`schema_version=fan_out_result.v1`），`artifact/read` 会返回机器可读 `fan_out_result` 字段。
  - 当 `workspace_mode=isolated_write` 且 workspace 有改动时，会额外写 `artifact_type="patch"`（覆盖 tracked + untracked 变更），并在 `fan_out_result.isolated_write_patch` 返回 `artifact_id/read_cmd/truncated`。
  - 若 patch 捕获失败（如超时、git diff 失败、workspace 无效）不会中断结果写入，会在 `fan_out_result.isolated_write_patch.error` 返回错误上下文。
  - `fan_out_result.isolated_write_handoff` 会给出 `status_argv/diff_argv/apply_patch_hint`，并镜像 `patch` 信息，便于人工回填。
  - 启用 auto-apply 时，`fan_out_result.isolated_write_auto_apply` 会额外返回 `failure_stage/recovery_hint`，用于区分失败阶段并给出最小人工修复指引（`failure_stage`：`precondition|capture_patch|check_patch|apply_patch|unknown`）；若已产出 patch artifact，还会附带 `patch_artifact_id/patch_read_cmd` 便于直接人工回填，并通过 `recovery_commands[{label,argv}]` 给出可直接复制的恢复命令模板。

返回值（现状）：

- `tasks[]`：每个 task 的 `thread_id/turn_id/log_path/last_seq/status/workspace_cwd` 等运行句柄（`turn_id` 可能为 `null`，表示尚未启动）
  - 依赖阻塞的 task 还会带结构化字段：`dependency_blocked=true`、`dependency_blocker_task_id`、`dependency_blocker_status`（不再只靠 `error` 文本解析）。
- `priority_aging_rounds`：本次调度实际使用的 aging 参数（来自 `OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS`，已做边界钳制/默认回退）。
- `limit_policy/limit_source/env_max_concurrent_subagents/mode_max_concurrent_subagents/max_concurrent_subagents`：本次调度实际使用的并发上限信息（成功/拒绝路径都可用于审计）。

可观测性：

- forked thread 会出现在 `thread/list_meta` 与 `omne inbox --watch` 中；可用 `thread/subscribe` 重放/追踪事件（见 `docs/thread_event_model.md`）。

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

- subagent 生命周期 hook 点（`SubagentStart/SubagentStop`）已接入；下一步重点是把 fan-in 收敛策略固化为可复用模板。

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
  - `mode`：子任务角色（例如 `architect/coder/reviewer/builder/debugger/merger`；若 v1 不支持，必须明确写死为只读角色）
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

- 汇总必须带 provenance：引用哪些 threads/turns/artifacts（见 `omne_protocol::ArtifactProvenance`）。
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

- fan-out 产生的每个子任务都能在 `omne inbox` 里定位到 `thread_id/turn_id`，并可用 `thread/subscribe` 重放事件。
- fan-in 汇总 artifact 内容必须显式列出来源 `thread_id/turn_id/artifact_id`，且 metadata 的 provenance 至少包含汇总发生的 `thread_id/turn_id`。
  - 子任务并发上限生效：超限时必须返回可理解的错误，并在事件/日志中留下“拒绝 spawn 的原因”。

### 3.3.1 v0.2.x 务实落地建议（先只读，后隔离写）

在 workspace 隔离能力落地前，**fan-out 只能是并发只读分析**；否则就是把数据竞争包装成“功能”。

建议 DoD（按优先级）：

- [x] `agent_spawn` 支持显式 `mode`（或等价字段），并默认选择只读角色（例如 `architect/reviewer`）；并由 `modes.yaml` 提供 `subagent.spawn` gate（不允许任意模式随意 spawn）。
- [x] 引入 `max_concurrent_subagents`（全局或 per-thread 二选一即可），并在事件/错误里记录“为什么拒绝 spawn”（对齐 Codex 的 guard 思路）。
- [x] 子任务会写出 `artifact_type="fan_out_result"`（可用 `expected_artifact_type` 覆盖），且 summary/内容至少包含 `task_id + turn_id + status`；fan-in 汇总会额外记录该结果锚点的 `artifact_id`（连同 `thread_id/turn_id`），若写入失败则记录 `artifact_error/error_artifact_id` 并在父 thread 写 `artifact_type="fan_out_result_error"`（同时提供 quick read 命令）。
- [x] `fan_out_result` 提供结构化 payload（`schema_version=fan_out_result.v1`），`artifact/read` 可直接解析 `fan_out_result`，不再依赖文本正则。
- [x] `workspace_mode=isolated_write` 已实现隔离工作区（独立 `cwd` + `workspace_write`，默认 worktree-first，失败回退 copy），不做隐式降级到共享写。
- [x] `workspace_mode=isolated_write` 会把 patch handoff 回填到 `fan_out_result`（`isolated_write_patch` / `isolated_write_handoff`）；并支持可选 auto-apply（`OMNE_SUBAGENT_ISOLATED_AUTO_APPLY_PATCH=1`）做最小 `git apply --check && git apply` 回填，失败不阻断主流程。
- [x] fan-out/fan-in 的 hook 点：父 thread 的 `ToolStarted/ToolCompleted(tool=subagent/spawn)` + 子 thread 的 `TurnCompleted`（由返回的 `thread_id/turn_id` 关联）+ 结果锚点 artifact（默认 `fan_out_result`）。
- [x] workflow fan-out：`omne command run <name> --fan-out` 解析 `## Task:`，并在父 thread 写入 `artifact_type="fan_in_summary"` 汇总 artifact（用于主 turn 上下文/可追溯）。
- [x] workflow fan-out：支持最小依赖语义（task body 第一条非空行 `depends_on: a,b`），只在依赖任务完成后启动子任务。
- [x] workflow fan-out：支持最小优先级语义（`priority: high|normal|low`）+ aging 公平调度（`OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS`），降低低优先级任务长期饥饿风险。
- [x] workflow fan-out（CLI 编排路径）在子任务 `ApprovalRequested` 时会把 `task_id/thread_id/turn_id/approval_id/action` 写入父 thread 的 `fan_in_summary`，并附结构化审批句柄（`approve_cmd`/`deny_cmd`，例如 `omne approval decide <thread_id> <approval_id> --approve|--deny`）；`fan_in_summary` 同时包含 `Structured Data` JSON 区块供脚本消费（含稳定 `schema_version=fan_in_summary.v1`，对应共享类型 `omne-workflow-spec`）。
- [x] workflow fan-out：当依赖任务失败/中断/取消/stuck 时，后继任务会被标记 `Cancelled` + `dependency_blocked=true`，不再启动；`fan_in_summary` Structured Data 会额外给出 `dependency_blocker_task_id/dependency_blocker_status`。
- [x] `agent_spawn`（服务端调度路径）依赖语义对齐：仅 `Completed` 算满足依赖；失败依赖会把后继任务标记 `blocked by dependency` 且不再启动。
- [x] `agent_spawn`（服务端调度路径）会把子任务 `ApprovalRequested/ApprovalDecided` 镜像到父 thread，父线程 `approval/decide` 可转发到子线程真实审批。

### 3.3.2 方案选型（A/B/C，避免“边写边发明”）

- **A：客户端编排（只读 fan-out）**：复用 `thread/fork + agent_spawn`，子任务只读并写 `fan_out_result`，fan-in 由 orchestrator 写 `fan_in_summary`；CLI 路径已有“审批句柄落父 artifact”，`user-input` 仍依赖 inbox/线程级操作。
- **B：服务端 guard + bridge**：服务端实现“预留/提交/释放 slot”的并发上限，并把子任务 `ApprovalRequested/ApprovalDecided`（未来扩展 `UserInputRequested`）以可审计方式桥接到父 thread（类似 Codex delegate）；复杂度更高但 UX 更好。
- **C：隔离 workspace（可写 fan-out）**：已落地“隔离工作区执行（默认 worktree-first，失败回退 copy）+ patch handoff + 可选 auto-apply 最小回填”；后续仍需补“冲突自动修复策略”。

推荐取舍：

- v0.2.x 优先做 **A**（先把结果锚点/provenance/只读边界做实），并尽早补上 **guard**（无论放客户端还是服务端，都必须是硬限制）。
- **B/C** 放到“有真实需求 + 有隔离能力”之后再做，否则只是在制造不可复现的竞态。

### 3.4 CLI/API（未来实现占位）

CLI（v0.2.x 最小可用 + 占位）：

```bash
# fan-out + fan-in：从 command body 的 `## Task:` 段落并发执行子任务，并写入 fan_in_summary
omne command run <name> --fan-out

# 低阶原语：只 fork + 启动子 turn（不做 fan-in 汇总）
omne thread spawn <thread_id> "<input>"
```

---

## 4) 快速自检（实现/文档一致性）

```bash
rg -n \"handle_thread_fork\" crates/app-server/src/main/thread_manage/fork.rs
rg -n \"agent_spawn\" crates/app-server/src/agent/tools/dispatch.rs
```
