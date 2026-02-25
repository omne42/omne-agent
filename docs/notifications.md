# 通知与提醒（v0.2.0 口径）

> 目标：RTS 场景下用户不该“刷屏盯日志”。系统必须在关键状态变化时把人叫回来。
>
> v0.2.0 MVP：通知发送统一走 `notify-kit`（当前默认 `SoundSink`，表现为终端 bell），并提供去重/节流；其它渠道可按需追加 sink。

---

## 0) 触发源：Attention（派生视图）

通知不直接由“某条日志文本”触发，而是由 thread 的 `attention_state` 变化触发。

v0.2.0 MVP 的状态集合以 `docs/v0.2.0_parity.md` 为准，核心会触发提醒的是：

- `need_approval`：需要人类审批（阻塞）
- `failed`：turn 或 process 失败
- `stuck`：预算/超时触发（见 `docs/budgets.md`）

Attention 的派生语义与状态集合见：

- `docs/attention.md`

---

## 1) 已实现：`omne watch --bell`（经 `notify-kit`）

`omne watch --bell` 是单 thread 的事件流订阅：

- 从事件流推导状态变化（例如 `ApprovalRequested` → `need_approval`，`TurnCompleted{Stuck}` → `stuck`）。
- 只有当状态变为 `need_approval|failed|stuck` 才会发通知（默认 sound bell）。
- `--details` 模式下，会在每批非空事件后追加 fan-out auto-apply、fan-in 依赖阻塞与 fan-in 结果诊断摘要（若存在），便于在 watch 流里直接定位阻塞来源。
- `--json --details` 下会在事件 JSON 行之后追加结构化摘要行：`kind=watch_detail_summary`（`summary_type=fan_out_auto_apply|fan_in_dependency_blocker|fan_in_result_diagnostics`）。
- `fan_in_dependency_blocker` 的 JSON payload 包含 `dependency_blocked_count`、`task_count`、`dependency_blocked_ratio`，方便告警侧直接聚合展示。
- `--details` 摘要默认做去重：若摘要内容未变化，不会在后续批次重复输出（文本与 JSON 一致）；仅输出有变化的 `summary_type`。
- `--json --details` 的摘要行会额外带 `changed_fields`，表示本次相对上一快照发生变化的字段集合。
- 当某个摘要从“存在”变为“消失”时，会输出一次 `cleared`：
  - 文本：`summary: <summary_type>: cleared`
  - JSON：`{"kind":"watch_detail_summary","summary_type":"...","cleared":true,"changed_fields":["cleared"],...}`
- 当 `AttentionMarkerSet{marker=fan_out_linkage_issue|fan_out_auto_apply_error}` 进入时，会按 `failed` 语义参与提醒。
- 会额外轮询该 thread 的 `thread/attention`，对 `has_fan_out_linkage_issue` / `has_fan_out_auto_apply_error` / `has_fan_in_dependency_blocked` / `has_fan_in_result_diagnostics` 与 `stale_processes` 的 `false -> true` 上升沿各提醒一次（节流同上）。
- 会额外轮询该 thread 的 `thread/attention`，对 `token_budget_exceeded` 的 `false -> true` 上升沿提醒一次（`state=token_budget_exceeded`）。
- 当启用 token budget 且利用率达到阈值时，也会在 `false -> true` 上升沿提醒一次（`state=token_budget_warning`，阈值见下方 `OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT`）。
- 默认抑制首次 bell（避免刚 attach 就响）。
- 支持 `--debounce-ms`：相同状态在窗口内只响一次。

对照实现：

- `crates/agent-cli/src/main/watch_inbox.rs`

---

## 2) 已实现：`omne inbox --watch --bell`（经 `notify-kit`）

`omne inbox --watch --bell` 轮询所有 thread meta：

- 只对 `need_approval|failed|stuck` 发通知（默认 sound bell）。
- 当 `has_fan_out_linkage_issue` 从 `false -> true` 时也会发一次 `attention_state` 通知（`state=fan_out_linkage_issue`）。
- 当 `has_fan_out_auto_apply_error` 从 `false -> true` 时也会发一次 `attention_state` 通知（`state=fan_out_auto_apply_error`）。
- 当 `has_fan_in_dependency_blocked` 从 `false -> true` 时也会发一次 `attention_state` 通知（`state=fan_in_dependency_blocked`）。
- 当 `has_fan_in_result_diagnostics` 从 `false -> true` 时也会发一次 `attention_state` 通知（`state=fan_in_result_diagnostics`）。
- 当 `token_budget_exceeded` 从 `false -> true` 时也会发一次 `attention_state` 通知（`state=token_budget_exceeded`）。
- 当 token budget 利用率达到阈值（默认 90%）并从 `false -> true` 时会发一次 `attention_state` 通知（`state=token_budget_warning`）。
- 按 `(thread_id, attention_state)` 去重/节流：相同 thread 的相同状态在 `debounce_window` 内只提醒一次；状态变化才再次提醒。
- 会在 stderr 输出一行 `attention: <thread_id> -> <state>`，并响铃（方便脚本抓取）。

对照实现：

- `crates/agent-cli/src/main/watch_inbox.rs`

---

## 2.1 `OMNE_NOTIFY_*` 配置（`omne --bell`）

`omne watch --bell` / `omne inbox --watch --bell` 会在启动时读取以下环境变量并构造 `notify-kit` sinks：

- `OMNE_NOTIFY_SOUND`：是否启用本地 sound sink（`1/true/yes/on` 开；默认开）
- `OMNE_NOTIFY_WEBHOOK_URL`：通用 webhook URL（HTTPS）
- `OMNE_NOTIFY_WEBHOOK_FIELD`：通用 webhook payload 字段名（默认 `text`）
- `OMNE_NOTIFY_FEISHU_WEBHOOK_URL`：飞书 webhook URL
- `OMNE_NOTIFY_SLACK_WEBHOOK_URL`：Slack Incoming Webhook URL
- `OMNE_NOTIFY_TIMEOUT_MS`：sink 超时毫秒（默认 `5000`）
- `OMNE_NOTIFY_EVENTS`：可选事件 kind 白名单（逗号分隔；例如 `attention_state,stale_process`）
- `OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT`：token budget 预警阈值（百分比，`0 < value <= 100`，默认 `90`；用于 `watch/inbox --bell` 与 `omne-app-server` 的 `token_budget_warning`）
- `OMNE_NOTIFY_TOKEN_BUDGET_WARNING_DEBOUNCE_MS`：`omne-app-server` 的 token budget 预警去抖窗口（毫秒，默认 `30000`；仅影响 `token_budget_warning`）

默认行为：

- 如果不配置任何 webhook，且未显式关闭 sound，则等价于原先的 bell 行为（sound sink）。
- 如果显式关闭 sound 且未配置任何 webhook，`--bell` 会报错（无可用通知 sink）。

---

## 2.2 `OMNE_NOTIFY_*` 配置（`omne-app-server`）

`omne-app-server` 也会读取同一组 `OMNE_NOTIFY_*` 环境变量，并在关键 thread 事件上直接发通知：

- `ApprovalRequested` → `attention_state=need_approval`
- `TurnCompleted{Failed|Stuck}` → `attention_state=failed|stuck`
  - 其中 `TurnCompleted{Stuck}` 且 `reason` 含 `token budget exceeded:` 时，映射为 `attention_state=token_budget_exceeded`
- 当启用 token budget 且利用率跨过阈值（`false -> true` 上升沿）时，发 `attention_state=token_budget_warning`
  - 该提醒受 `OMNE_NOTIFY_TOKEN_BUDGET_WARNING_DEBOUNCE_MS` 去抖控制（默认 30s）
- `ProcessExited{exit_code!=0}` → `attention_state=failed`
- `AttentionMarkerSet{marker=fan_out_auto_apply_error}` → `attention_state=fan_out_auto_apply_error`
- `AttentionMarkerSet{marker=fan_out_linkage_issue}` → `attention_state=fan_out_linkage_issue`

注意：

- 服务端默认不启用 sound（`OMNE_NOTIFY_SOUND` 默认关闭），避免改变 app-server 的默认行为。
- 若未配置任何 sink（sound/webhook/feishu/slack 均未启用），服务端不会发送外部通知。

---

## 3) 已实现：后台进程“需要人接管”提醒

问题：v0.2.0 的进程模型是非交互（`stdin=null`），因此“等待输入”会表现为**长时间无输出/不退出**。如果不显式提醒，用户会以为系统死了。

最小可实现规格（不引入复杂 UI）：

- 检测条件（任意满足）：
  - running process 在 `idle_window` 内无新输出（以 stdout/stderr artifacts 的 mtime 近似）
- 行为：
  - `thread/attention` 输出 `stale_processes=[{process_id, idle_seconds, last_update_at, stdout_path, stderr_path}]`
  - `omne inbox --bell` / `omne watch --bell` 在 `stale_processes` 从空变非空时提醒一次（节流同上）
- 默认阈值建议：`idle_window=300s`；`0` 禁用

建议实现（写死一个简单、可复用的算法）：

- 对每个 running process：
  - 用文件 mtime 作为“最近输出”的近似，但必须考虑 rotate 分片：
    - 取 `stdout_path/stderr_path` 的父目录作为 process artifacts 目录
    - `last_stdout_at = max(mtime(stdout.log), mtime(stdout.segment-*.log), mtime(stdout.part-*.log))`
    - `last_stderr_at = max(mtime(stderr.log), mtime(stderr.segment-*.log), mtime(stderr.part-*.log))`
    - `last_update_at = max(last_stdout_at, last_stderr_at)`
    - 若 stdout/stderr 都找不到任何文件：`last_update_at = process.started_at`（保证“无输出也能被判 stale”）
  - `idle_seconds = now - last_update_at`
  - `idle_seconds >= idle_window` → 认为该 process stale
- 线程级别只要存在任意 stale process，就认为“需要人接管”。

配置项：

- `OMNE_PROCESS_IDLE_WINDOW_SECONDS`：
  - `0` = 禁用
  - `N>0` = idle_window 秒数（默认建议 300）

注意：`attention_state` 可能仍然是 `running`。因此提醒逻辑不能只盯 `attention_state`，必须把 `stale_processes`（或 count）当成独立触发源。

备注：不要发明“stdin 交互”。正确动作是：用户 `process/inspect`/`process/tail` 看输出，必要时 `process/kill`，然后把命令改成非交互式。

---

## 4) 快速自检

```bash
# bell 逻辑（状态推导 + debounce）
rg -n \"omne inbox\" crates/agent-cli/src/main/watch_inbox.rs
rg -n \"maybe_bell\" crates/agent-cli/src/main/watch_inbox.rs
```
