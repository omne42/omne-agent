# Attention / Inbox（派生视图）（v0.2.0 口径 + TODO）

> 目标：把“需要人介入”的点变成可枚举、可查询、可通知的状态，而不是让用户 grep 日志。
>
> 原则：Attention 是**派生视图**（derived view），唯一真相仍是 `events.jsonl`（见 `docs/thread_event_model.md`）。

---

## 0) 范围与非目标

范围（v0.2.0）：

- 定义 `thread/list_meta` 与 `thread/attention` 的输出里 `attention_state` 的语义与取值。
- 定义哪些状态会触发 `omne-agent watch --bell` / `omne-agent inbox --bell`（见 `docs/notifications.md`）。

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
omne-agent thread list-meta
omne-agent thread list-meta --json
```

用途：

- 快速列出所有 threads 的 `attention_state`（适合 inbox 轮询）。

### 2.2 `thread/attention`（单 thread 详情）

```bash
omne-agent thread attention <thread_id>
omne-agent thread attention <thread_id> --json
```

详情里至少包含（当前实现）：

- `attention_state`
- `pending_approvals`（可用于定位阻塞点）
- `running_processes` / `failed_processes`
- `last_turn_status/last_turn_reason`

---

## 3) TODO：状态扩展（PlanReady/DiffReady/TestFailed）

> 这是目标态扩展，**v0.2.0 未实现**（见 `docs/v0.2.0_parity.md`、`docs/rts_workflow.md`）。

需求动机：

- 计划草案/差异产物/测试失败是“语义状态”，不该要求用户自己解析 stdout 或 grep。

最小可行方向（两条路，先别发明 DSL）：

- **Option A（推断，不推荐）**：从 artifacts / process argv 推断（容易误判；只作为临时路线）：
  - `plan_ready`：检测到最新 artifact `artifact_type="plan"`（或约定类型）
  - `diff_ready`：检测到最新 artifact `artifact_type="diff"`（或约定类型）
  - `test_failed`：检测到特定流程产生的失败进程/产物（容易误判）
- **Option B（推荐：显式标注，派生稳定；v1 建议写死）**：
  - 引入明确的标记（事件或 thread state 字段），Attention 只读这些标记，不靠猜。
  - v1 推荐：新增轻量事件（占位名）：
    - `AttentionMarkerUpdated { kind, status: "set"|"cleared", turn_id?, artifact_id?, process_id?, summary? }`
  - 允许的 `kind`（写死，snake_case）：
    - `plan_ready`：存在可审阅的 plan artifact（建议 `artifact_type="plan"`）
    - `diff_ready`：存在可审阅的 diff/patch artifact（建议 `artifact_type="diff"` 或 `artifact_type="patch"`）
    - `test_failed`：测试失败（建议同时生成 `artifact_type="test_report"` 或复用 `artifact_type="stuck_report"` 的模板思路）
  - 校验（fail-closed）：
    - unknown `kind`：直接报错（避免“看起来标了，实际 silently ignore”）。
    - `artifact_id/process_id` 必须能在本 thread 内定位；否则报错。
  - 清理语义（写死一个简单规则）：
    - 默认在下一次 `TurnStarted` 时自动清掉 `plan_ready/diff_ready/test_failed`（因为用户已经开始推进下一步）。
    - 允许显式 clear（例如 UI 点击“已读/已处理”）：
      - `AttentionMarkerUpdated { kind, status: "cleared" }`
  - 派生与输出（建议写死，避免把 `attention_state` 搞成万能枚举）：
    - `attention_state` 保持 v0.2.0 口径与优先级（仍是 bell 的主要触发源；见 `docs/notifications.md`）。
    - `thread/attention` 额外输出：
      - `attention_markers={ plan_ready?, diff_ready?, test_failed? }`
        - 每个 marker 至少包含 `set_at`（时间戳）+ `turn_id?` + `artifact_id?/process_id?`（用于定位可行动对象）。
    - `thread/list_meta`（inbox 轮询）至少输出：
      - `has_plan_ready/has_diff_ready/has_test_failed`（或等价的 count/marker 摘要）

验收（未来实现时）：

- `omne-agent inbox --watch --json` 能直接看到 `has_plan_ready/has_diff_ready/has_test_failed`（或等价字段），不依赖解析 stdout。

---

## 4) TODO：`stale_processes`（后台进程无输出提醒）

> 规格草案见 `docs/notifications.md`，这里把它落到 Attention 的字段化口径上。

建议扩展（TODO）：

- `thread/attention` 输出新增字段：
  - `stale_processes=[{process_id, idle_seconds, last_update_at, stdout_path, stderr_path}]`
- `idle_seconds` 由 stdout/stderr 文件 mtime 近似计算（非交互进程场景足够）。
  - 注意 stdout/stderr 可能 rotate 分片：`last_update_at` 应取所有 `*.segment-*.log` 的最大 mtime（细节见 `docs/notifications.md`）。
- 默认阈值建议 `idle_window=300s`；`0` 禁用（建议 env：`OMNE_AGENT_PROCESS_IDLE_WINDOW_SECONDS`）。

注意：

- 即使 `attention_state=running`，也可能存在 `stale_processes`（这是独立信号）。
- `omne-agent inbox --bell` / `omne-agent watch --bell` 的提醒触发应以 `stale_processes` 从空→非空为准，并做 debounce（见 `docs/notifications.md`）。
