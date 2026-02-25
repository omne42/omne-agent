# Attention / Inbox（派生视图）（v0.2.0 口径 + TODO）

> 目标：把“需要人介入”的点变成可枚举、可查询、可通知的状态，而不是让用户 grep 日志。
>
> 原则：Attention 是**派生视图**（derived view），唯一真相仍是 `events.jsonl`（见 `docs/thread_event_model.md`）。

---

## 0) 范围与非目标

范围（v0.2.0）：

- 定义 `thread/list_meta` 与 `thread/attention` 的输出里 `attention_state` 的语义与取值。
- 定义哪些状态会触发 `omne watch --bell` / `omne inbox --bell`（见 `docs/notifications.md`）。

非目标（v0.2.0）：

- 不做 UI；只定义控制面/CLI 可观察的语义。
- 不把复杂业务状态塞进事件；如需新增状态，优先用**小而明确的事件/字段**表达，而不是靠启发式猜测。

---

## 1) `attention_state`（v0.2.0：已实现）

### 1.1 值集合（当前实现可能出现）

`thread/list_meta` 与 `thread/attention` 返回的 `attention_state` 为字符串（snake_case），当前实现可能出现：

- `need_approval`：存在 pending approvals（阻塞）
- `failed`：存在失败 turn 或失败进程
- `stuck`：turn 命中 budgets/timeout（见 `docs/budgets.md`）
- `running`：存在 active turn 或 running process
- `paused`：thread 被 pause
- `archived`：thread 已归档
- `done`：最后一个 turn completed
- `interrupted`：最后一个 turn interrupted
- `cancelled`：最后一个 turn cancelled
- `idle`：没有 turn（或无状态）

注意：

- bell 的触发集合是 `need_approval|failed|stuck`（见 `docs/notifications.md`），其它状态变化只更新展示，不响铃。

### 1.2 派生优先级（当前实现口径）

派生规则来自服务端对 thread state + events 的计算（实现对照：`crates/app-server/src/main/thread_observe.rs`）。可用直觉理解为：

1. 有 pending approval → `need_approval`
2. 有 failed process/turn → `failed`
3. 有 active turn 或 running process → `running`
4. paused → `paused`
5. archived → `archived`
6. 否则根据 last turn status → `done|interrupted|cancelled|failed|stuck`，都没有则 `idle`

> 约束：Attention 是“需要人介入”的视图；因此 `need_approval/failed/stuck` 必须可稳定派生。

---

## 2) API/CLI（可复制）

### 2.1 `thread/list_meta`（inbox 的轻量快照）

```bash
omne thread list-meta
omne thread list-meta --json
omne thread list-meta --include-attention-markers --json
```

用途：

- 快速列出所有 threads 的 `attention_state`（适合 inbox 轮询）。
- 当需要 marker 详情（而不只是布尔摘要）时，可用 `--include-attention-markers` 返回 `attention_markers` 对象（结构与 `thread/attention` 对齐）。
- `omne thread list-meta` 的纯文本输出 `markers=...` 现会包含 `fan_in_dependency_blocked` / `fan_in_result_diagnostics` / `subagent_proxy_approval` 与 `token_budget_exceeded` / `token_budget_warning`（对应字段为 true 时）。
- 同一行在 `count>0` 时会输出 `subagent_pending=<count>`，用于快速查看待处理 `subagent/proxy_approval` 数量。

配套 inbox 视图（CLI）：

```bash
omne inbox
omne inbox --details
omne inbox --only-fan-out-linkage-issue
omne inbox --only-fan-out-auto-apply-error
omne inbox --only-fan-in-dependency-blocked
omne inbox --only-fan-in-result-diagnostics
omne inbox --only-token-budget-exceeded
omne inbox --only-token-budget-warning
omne inbox --only-subagent-proxy-approval
```

- `--only-fan-out-linkage-issue`：仅保留 `has_fan_out_linkage_issue=true` 的 thread，便于聚焦 fan-out 关联异常。
- `--only-fan-out-auto-apply-error`：仅保留 `has_fan_out_auto_apply_error=true` 的 thread，便于聚焦 fan-out 自动回写失败。
- `--only-fan-in-dependency-blocked`：仅保留存在 fan-in 依赖阻塞摘要的 thread，便于聚焦依赖链阻塞。
- `--only-fan-in-result-diagnostics`：仅保留存在 fan-in 结果诊断摘要的 thread，便于聚焦 fan-in 结果匹配/扫描异常。
- `--only-token-budget-exceeded`：仅保留 token budget 已超限（`token_budget_exceeded=true`）的 thread，便于优先处理硬阻塞。
- `--only-token-budget-warning`：仅保留 token budget 预警激活的 thread（`token_budget_utilization` 达阈值且未 exceeded），阈值与 bell 口径一致（默认 90%，可由 `OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT` 覆盖）。
- `--only-subagent-proxy-approval`：仅保留存在待处理 `subagent/proxy_approval` 的父 thread（由 `thread/list_meta.pending_subagent_proxy_approvals>0` 派生）。
- `--details`：除了 attention 详情，还会追加 fan-out 自动回写摘要、fan-in 依赖阻塞摘要，以及 fan-in 结果诊断摘要（若存在 `dependency_blocked/dependency_blocker_*`，并包含阻塞计数）。
  - `markers` 行会额外显示 `token_budget_exceeded` / `token_budget_warning`（后者阈值与 bell 口径一致）。
  - 当启用 token budget 时，会额外显示 `token_budget` 快照行（`remaining/limit/utilization/exceeded`）。
  - `--json` 行会包含布尔字段 `token_budget_warning_active`（由 `thread/list_meta` 下发；阈值口径与 bell 一致）。
  - 若存在 subagent 代理审批，还会追加 `subagent_pending` 行（`total` + `states` 分布），便于快速判断子线程整体进度。
  - `--json --details` 下，每个 thread 行会额外包含 `subagent_pending` 对象（`total` + `states`），可直接供看板/告警消费。
  - watch 模式下摘要按“内容变化”输出，未变化不会重复刷屏；仅变化的摘要类型会输出（含 `subagent_pending`）。
  - 摘要从存在变为不存在时会输出一次 `cleared` 信号，方便下游消费。
  - JSON 摘要会包含 `dependency_blocked_ratio` 与 `changed_fields`，可直接用于看板/告警展示。
  - 调试开关：
    - `omne watch <thread_id> --debug-summary-cache`（或 `OMNE_WATCH_SUMMARY_CACHE_DEBUG=1`）：输出 `watch_summary_refresh`，包含 `auto_apply/fan_in/fan_in_diag/subagent` 的 refresh/source；`--json` 下会输出 `kind=watch_summary_refresh_debug` 的结构化 JSON 行。
    - `omne inbox --watch --details --debug-summary-cache`（或 `OMNE_INBOX_SUMMARY_CACHE_DEBUG=1`）：输出 `inbox_summary_cache`，包含 `fan_out/fan_in/fan_in_diag/subagent` 的 cache/attention/fetch/skip 统计。
    - `omne inbox --watch --details --json --debug-summary-cache` 时，JSON 输出会额外带 `summary_cache_stats` 对象（结构化统计，与 stderr 文本统计字段一致）。

### 2.2 `thread/attention`（单 thread 详情）

```bash
omne thread attention <thread_id>
omne thread attention <thread_id> --json
```

详情里至少包含（当前实现）：

- `attention_state`
- `pending_approvals`（可用于定位阻塞点）
  - 当 action 为 `subagent/proxy_approval` 时，`pending_approvals[].summary` 会尽力补充
    - `child_attention_state`
    - `child_last_turn_status`
    便于在父 thread 直接判断子 thread 当前是否仍在运行/已完成/失败，而无需手动切换查询子 thread。
- `running_processes` / `failed_processes`
- `stale_processes`（后台进程“无输出但仍运行”的接管信号）
- `last_turn_status/last_turn_reason`
- `token_budget_limit/token_budget_remaining/token_budget_utilization/token_budget_exceeded/token_budget_warning_active`（启用 `OMNE_AGENT_MAX_TOTAL_TOKENS` 时）

---

## 3) 已实现：状态扩展（PlanReady/DiffReady/TestFailed/FanOutLinkageIssue/FanOutAutoApplyError）

`thread/attention` 已增加 marker 输出（当前实现为“显式事件优先 + 推断回退”）：

- `attention_markers.plan_ready`：
  - 来源（优先）：最新 `AttentionMarkerSet{marker=plan_ready}` 事件
  - 清除：`AttentionMarkerCleared{marker=plan_ready}`（当前在新 turn 开始时触发）
  - 回退来源：本 thread 最新 `artifact_type="plan"` artifact（兼容历史数据）
  - 字段：`set_at` + `artifact_id` + `artifact_type` + `turn_id?`
- `attention_markers.diff_ready`：
  - 来源（优先）：最新 `AttentionMarkerSet{marker=diff_ready}` 事件
  - 清除：`AttentionMarkerCleared{marker=diff_ready}`（当前在新 turn 开始时触发）
  - 回退来源：本 thread 最新 `artifact_type in {"diff","patch"}` artifact（兼容历史数据）
  - 字段同上
- `attention_markers.test_failed`：
  - 来源（优先）：`AttentionMarkerSet{marker=test_failed}` / `AttentionMarkerCleared{marker=test_failed}` 事件序列（后者会清除 marker）
  - 回退来源：本 thread 最新“测试命令失败”的 `ProcessExited(exit_code!=0)`（由 `ProcessStarted.argv` 识别 `cargo test` / `pytest` / `npm test` / `go test` 等，兼容历史数据）
  - 字段：`set_at` + `process_id` + `turn_id?` + `exit_code?` + `command?`
- `attention_markers.fan_out_linkage_issue`：
  - 来源（优先）：`AttentionMarkerSet{marker=fan_out_linkage_issue}` / `AttentionMarkerCleared{marker=fan_out_linkage_issue}`（前者由 `artifact_type="fan_out_linkage_issue"` 写入触发；后者由 `artifact_type="fan_out_linkage_issue_clear"` 或新 turn 开始触发）
  - 回退来源：本 thread 最新 `artifact_type="fan_out_linkage_issue"` artifact（兼容历史数据）
  - 字段：`set_at` + `artifact_id` + `artifact_type` + `turn_id?`
- `attention_markers.fan_out_auto_apply_error`：
  - 来源（优先）：`AttentionMarkerSet{marker=fan_out_auto_apply_error}` / `AttentionMarkerCleared{marker=fan_out_auto_apply_error}`（由 `artifact_type="fan_out_result"` 的结构化 payload 中 `isolated_write_auto_apply.error` 是否存在触发；新 turn 开始时也会清除）
  - 回退来源：本 thread 最新可解析的 `artifact_type="fan_out_result"` artifact：若 `isolated_write_auto_apply.error` 非空则置位；若最新可解析结果无 error 则视为已清除（兼容历史数据）
  - 字段：`set_at` + `artifact_id` + `artifact_type` + `turn_id?`
- `attention_markers.token_budget_warning`：
  - 来源：`AttentionMarkerSet{marker=token_budget_warning}` / `AttentionMarkerCleared{marker=token_budget_warning}`（由 token budget 利用率跨阈值上升沿/回落触发）
  - 字段：`set_at` + `turn_id?`
- `attention_markers.token_budget_exceeded`：
  - 来源：`AttentionMarkerSet{marker=token_budget_exceeded}` / `AttentionMarkerCleared{marker=token_budget_exceeded}`（由 token budget 超限上升沿/回落触发）
  - 字段：`set_at` + `turn_id?`

同时，`thread/attention` 还会输出布尔摘要：

- `has_plan_ready`
- `has_diff_ready`
- `has_fan_out_linkage_issue`
- `has_fan_out_auto_apply_error`
- `has_fan_in_dependency_blocked`
- `has_fan_in_result_diagnostics`
- `has_test_failed`

当存在 fan-out 自动回写未应用时，还会输出轻量摘要（当前实现）：

- `fan_out_auto_apply`（可选）：
  - `task_id`
  - `status`（`error` / `attempted_not_applied` / `enabled_not_attempted` / `disabled`）
  - `stage?` / `patch_artifact_id?` / `recovery_commands?` / `recovery_1?` / `error?`
  - 语义：优先使用“最新可解析 fan_out_result”的结构化内容；若最新结果已 `applied=true`，该字段会消失（而不是沿用更旧未应用记录）。

当存在 fan-in 依赖阻塞时，还会输出轻量摘要（当前实现）：

- `fan_in_dependency_blocker`（可选）：
  - `task_id`
  - `status`
  - `dependency_blocked_count` / `task_count` / `dependency_blocked_ratio`
  - `blocker_task_id?` / `blocker_status?` / `reason?`
  - 语义：优先使用“最新可解析 fan_in_summary”的结构化内容；若最新摘要已清除阻塞，该字段会消失（而不是沿用更旧阻塞记录）。

当存在 fan-in 结果诊断时，还会输出轻量摘要（当前实现）：

- `fan_in_result_diagnostics`（可选）：
  - `task_count`
  - `diagnostics_tasks`
  - `diagnostics_matched_completion_total`
  - `diagnostics_pending_matching_tool_ids_total`
  - `diagnostics_scan_last_seq_max`
  - 语义：优先使用“最新可解析 fan_in_summary”的结构化内容；若最新摘要不存在诊断信息，该字段会消失。

注意：

- `plan_ready/diff_ready/test_failed/fan_out_linkage_issue/fan_out_auto_apply_error/token_budget_warning/token_budget_exceeded` 已支持显式事件化（`AttentionMarkerSet`）；其中前五者与 `token_budget_*` 都支持显式清除（`AttentionMarkerCleared`，`token_budget_*` 由预算状态回落自动清除）；`thread/attention` 会优先使用显式事件，并对历史数据回退推断。
- 当前布尔摘要在 `thread/attention` 与 `thread/list_meta` 均可用（便于 inbox 直接按 marker 过滤）。
- `thread/list_meta` 与 `thread/attention` 均会在存在时携带 `fan_out_auto_apply` 轻量摘要，便于 inbox/watch 直接展示而不必逐线程读取 artifact。
- `thread/list_meta` 与 `thread/attention` 均会在阻塞存在时携带 `fan_in_dependency_blocker` 轻量摘要，便于 inbox/watch 直接展示而不必逐线程读取 artifact。
- `fan_out_auto_apply_error` 目前会把 `attention_state` 提升为 `failed`（便于统一失败语义），同时保留专用 marker/布尔字段用于精准筛选。
- `thread/list_meta` 与 `thread/attention` 现已携带 token budget 快照字段，供 `watch/inbox --bell` 做 `token_budget_exceeded` 与 `token_budget_warning`（高利用率）提醒（详见 `docs/notifications.md`）。
- `thread/usage` / `thread/state` / `thread/attention` / `thread/list_meta` 四个接口现已使用同一套服务端预算快照口径：
  - `token_budget_limit` 为空时，`remaining/utilization/exceeded/warning_active` 全部为空；
  - `token_budget_exceeded=true` 时，`token_budget_warning_active` 必为 `false`（不会同时为 true）。

---

## 4) 已实现：`stale_processes`（后台进程无输出提醒）

> 具体算法与通知口径见 `docs/notifications.md`，这里给 Attention 侧的字段语义。

- `thread/attention` 输出新增字段：
  - `stale_processes=[{process_id, idle_seconds, last_update_at, stdout_path, stderr_path}]`
- `idle_seconds` 由 stdout/stderr 文件 mtime 近似计算（非交互进程场景足够）。
  - 注意 stdout/stderr 可能 rotate 分片：`last_update_at` 应取所有 `*.segment-*.log` 的最大 mtime（细节见 `docs/notifications.md`）。
- 默认阈值建议 `idle_window=300s`；`0` 禁用（建议 env：`OMNE_PROCESS_IDLE_WINDOW_SECONDS`）。

注意：

- 即使 `attention_state=running`，也可能存在 `stale_processes`（这是独立信号）。
- `omne inbox --bell` / `omne watch --bell` 的提醒触发应以 `stale_processes` 从空→非空为准，并做 debounce（见 `docs/notifications.md`）。
