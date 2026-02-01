# 通知与提醒（v0.2.0 口径）

> 目标：RTS 场景下用户不该“刷屏盯日志”。系统必须在关键状态变化时把人叫回来。
>
> v0.2.0 MVP：只做终端 bell（`\x07`），并提供去重/节流；其它通知渠道（系统通知/webhook）留接口但不强做。

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

## 1) 已实现：`omne-agent watch --bell`

`omne-agent watch --bell` 是单 thread 的事件流订阅：

- 从事件流推导状态变化（例如 `ApprovalRequested` → `need_approval`，`TurnCompleted{Stuck}` → `stuck`）。
- 只有当状态变为 `need_approval|failed|stuck` 才会响铃。
- 默认抑制首次 bell（避免刚 attach 就响）。
- 支持 `--debounce-ms`：相同状态在窗口内只响一次。

对照实现：

- `crates/agent-cli/src/main/watch_inbox.rs`

---

## 2) 已实现：`omne-agent inbox --watch --bell`

`omne-agent inbox --watch --bell` 轮询所有 thread meta：

- 只对 `need_approval|failed|stuck` 响铃。
- 按 `(thread_id, attention_state)` 去重/节流：相同 thread 的相同状态在 `debounce_window` 内只提醒一次；状态变化才再次提醒。
- 会在 stderr 输出一行 `attention: <thread_id> -> <state>`，并响铃（方便脚本抓取）。

对照实现：

- `crates/agent-cli/src/main/watch_inbox.rs`

---

## 3) 已实现：后台进程“需要人接管”提醒

问题：v0.2.0 的进程模型是非交互（`stdin=null`），因此“等待输入”会表现为**长时间无输出/不退出**。如果不显式提醒，用户会以为系统死了。

最小可实现规格（不引入复杂 UI）：

- 检测条件（任意满足）：
  - running process 在 `idle_window` 内无新输出（以 stdout/stderr artifacts 的 mtime 近似）
- 行为：
  - `thread/attention` 输出 `stale_processes=[{process_id, idle_seconds, last_update_at, stdout_path, stderr_path}]`
  - `omne-agent inbox --bell` / `omne-agent watch --bell` 在 `stale_processes` 从空变非空时提醒一次（节流同上）
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

- `OMNE_AGENT_PROCESS_IDLE_WINDOW_SECONDS`：
  - `0` = 禁用
  - `N>0` = idle_window 秒数（默认建议 300）

注意：`attention_state` 可能仍然是 `running`。因此提醒逻辑不能只盯 `attention_state`，必须把 `stale_processes`（或 count）当成独立触发源。

备注：不要发明“stdin 交互”。正确动作是：用户 `process/inspect`/`process/tail` 看输出，必要时 `process/kill`，然后把命令改成非交互式。

---

## 4) 快速自检

```bash
# bell 逻辑（状态推导 + debounce）
rg -n \"omne-agent inbox\" crates/agent-cli/src/main/watch_inbox.rs
rg -n \"maybe_bell\" crates/agent-cli/src/main/watch_inbox.rs
```
