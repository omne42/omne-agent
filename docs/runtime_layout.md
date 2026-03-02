# 运行时目录结构（`omne_root` = `./.omne_data/`）（v0.2.0 口径）

> 目标：让用户/工具在 30 秒内定位：事件落盘在哪、process log 在哪、user artifact 在哪。
>
> `omne_root` 默认是 `./.omne_data/`：运行时数据（threads/artifacts）+ 项目级配置（`.omne_data/config.toml`、`.omne_data/config_local.toml`、`.omne_data/.env`、`.omne_data/spec/`）。

---

## 0) omne_root 的选择（写死）

`omne_root` 是 `.omne_data/` 的根目录：

- 直接运行 app-server（`omne-app-server`）：默认 `$(pwd)/.omne_data`
- 通过 `omne` CLI：默认会显式传 `--omne-root`（优先级：CLI flag > env `OMNE_ROOT` > `$(pwd)/.omne_data`）
- 覆盖方式：
  - CLI：`omne-app-server --omne-root <path>`
  - env：`OMNE_ROOT=<path>`

---

## 0.1) daemon socket（可选）

当使用 daemon 模式（unix socket）时：

- socket：`<omne_root>/daemon.sock`
- 启动：`omne-app-server --omne-root <omne_root> --listen <omne_root>/daemon.sock`
- `omne` CLI：默认会尝试连接该 socket；若连接失败：
  - 默认会**自动启动** daemon（可用 `OMNE_RPC_AUTOSTART_DAEMON=0` 关闭）
  - 启动后会重试连接（超时可用 `OMNE_RPC_DAEMON_START_TIMEOUT_MS` 覆盖）

daemon 自动启动时的日志落盘（用于排障）：

- `<omne_root>/logs/daemon.log`

> 重要：对 OpenAI-compatible gateway 来说，prompt cache 命中率往往依赖 **TCP connection stickiness**（per-instance cache）。
> 使用 daemon 能保持 app-server 进程长期存活，从而复用 HTTP connection pool，显著降低 `cached_tokens=0` 的概率。
>
> 另见：`docs/prompt_cache.md`（cached_tokens 的观测、验证与限制）

---

## 1) threads 与事件日志（append-only）

每个 thread 一个目录：

```
<omne_root>/
  threads/
    <thread_id>/
      events.jsonl
      events.jsonl.lock
      readable_history.jsonl
```

说明：

- `events.jsonl`：append-only 事件流（每行一个 `ThreadEvent` JSON）。
- `events.jsonl.lock`：写入锁（避免并发写坏 log）。
- `readable_history.jsonl`：用户可读对话（仅 `user/assistant` 文本行；由事件派生写入，可删除重建）。

---

## 2) artifacts 目录（用户可见产物）

thread 下的 artifacts 根目录：

```
<omne_root>/threads/<thread_id>/artifacts/
```

### 2.1 user artifacts（`artifact/write`）

用户可见的文档产物（markdown + metadata）：

```
<omne_root>/threads/<thread_id>/artifacts/user/
  <artifact_id>.md
  <artifact_id>.metadata.json
```

metadata 字段模型：`omne_protocol::ArtifactMetadata`（见 `crates/agent-protocol/src/lib.rs`）。

---

## 3) runtime 目录（内部运行时落盘）

thread 下的 runtime 根目录：

```
<omne_root>/threads/<thread_id>/runtime/
```

### 3.1 process logs（stdout/stderr）

每个 process 一个目录：

```
<omne_root>/threads/<thread_id>/runtime/processes/<process_id>/
  stdout.log
  stdout.segment-0001.log        # 超过阈值后 rotate
  stdout.part-0001.log           # 兼容命名（如出现）
  stderr.log
  stderr.segment-0001.log
  stderr.part-0001.log           # 兼容命名（如出现）
```

要点：

- `process/start` 会在事件里落盘 `ProcessStarted{stdout_path,stderr_path}`，并在返回值里直接带路径。
- rotate 阈值默认 `8MiB`，可用 `OMNE_PROCESS_LOG_MAX_BYTES_PER_PART` 覆盖。

### 3.2 LLM stream debug（可选）

当开启 `OMNE_DEBUG_LLM_STREAM=1` 时：

```
<omne_root>/threads/<thread_id>/runtime/llm_stream/<turn_id>.jsonl
<omne_root>/threads/<thread_id>/runtime/llm_stream/<turn_id>.request_body.json
```

---

## 4) 如何“从 ID 定位到文件”

- 已知 `thread_id`：
  - `omne thread events <thread_id>`（或 JSON-RPC `thread/subscribe`）看 `events.jsonl` 的回放结果
  - `omne artifact list <thread_id>` 查 user artifacts（返回 metadata + 路径）
  - `omne process list --thread <thread_id>` 查 processes（返回 stdout/stderr 路径）
- 已知 `process_id`：
  - `omne process inspect <process_id>` / `omne process tail <process_id>` / `omne process follow <process_id>`
- 已知 `artifact_id`：
  - `omne artifact read <thread_id> <artifact_id>`

---

## 5) 清理行为（危险但必要）

- `thread/clear_artifacts` 会删除 `<thread_dir>/artifacts`（不影响 running processes）。
- `thread/delete` 会删除整个 `<thread_dir>`（包括 events 与 artifacts）。
