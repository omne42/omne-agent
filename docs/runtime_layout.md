# 运行时目录结构（`agent_root` = `./.omne_agent_data/`）（v0.2.0 口径）

> 目标：让用户/工具在 30 秒内定位：事件落盘在哪、process log 在哪、user artifact 在哪。
>
> `agent_root` 默认是 `./.omne_agent_data/`：运行时数据（threads/artifacts）+ 项目级配置（`.omne_agent_data/config.toml`、`.omne_agent_data/config_local.toml`、`.omne_agent_data/.env`、`.omne_agent_data/spec/`）。

---

## 0) agent_root 的选择

`agent_root` 是 `.omne_agent_data/` 的根目录：

- 直接运行 app-server（`omne-agent-app-server`）：默认 `$(pwd)/.omne_agent_data`
- 通过 `omne-agent` CLI：默认会显式传 `--root`（优先级：CLI flag > env `OMNE_AGENT_ROOT` > `$(pwd)/.omne_agent_data`）
- 覆盖方式：
  - CLI：`omne-agent-app-server --root <path>`
  - env：`OMNE_AGENT_ROOT=<path>`

---

## 0.1) daemon socket（可选）

当使用 daemon 模式（unix socket）时：

- socket：`<agent_root>/daemon.sock`
- 启动：`omne-agent-app-server --root <agent_root> --listen <agent_root>/daemon.sock`
- `omne-agent` CLI：默认会尝试连接该 socket；失败则 fallback 到 spawn `omne-agent-app-server`（保持 JSON-RPC 语义不变）。

---

## 1) threads 与事件日志（append-only）

每个 thread 一个目录：

```
<agent_root>/
  threads/
    <thread_id>/
      events.jsonl
      events.jsonl.lock
```

说明：

- `events.jsonl`：append-only 事件流（每行一个 `ThreadEvent` JSON）。
- `events.jsonl.lock`：写入锁（避免并发写坏 log）。

---

## 2) artifacts 目录（大内容都在这）

thread 下的 artifacts 根目录：

```
<agent_root>/threads/<thread_id>/artifacts/
```

### 2.1 process logs（stdout/stderr）

每个 process 一个目录：

```
<agent_root>/threads/<thread_id>/artifacts/processes/<process_id>/
  stdout.log
  stdout.segment-0001.log        # 超过阈值后 rotate
  stderr.log
  stderr.segment-0001.log
```

要点：

- `process/start` 会在事件里落盘 `ProcessStarted{stdout_path,stderr_path}`，并在返回值里直接带路径。
- rotate 阈值默认 `8MiB`，可用 `OMNE_AGENT_PROCESS_LOG_MAX_BYTES_PER_PART` 覆盖。

### 2.2 user artifacts（`artifact/write`）

用户可见的文档产物（markdown + metadata）：

```
<agent_root>/threads/<thread_id>/artifacts/user/
  <artifact_id>.md
  <artifact_id>.metadata.json
```

metadata 字段模型：`omne_agent_protocol::ArtifactMetadata`（见 `crates/agent-protocol/src/lib.rs`）。

---

## 3) 如何“从 ID 定位到文件”

- 已知 `thread_id`：
  - `omne-agent thread events <thread_id>`（或 JSON-RPC `thread/subscribe`）看 `events.jsonl` 的回放结果
  - `omne-agent artifact list <thread_id>` 查 user artifacts（返回 metadata + 路径）
  - `omne-agent process list --thread-id <thread_id>` 查 processes（返回 stdout/stderr 路径）
- 已知 `process_id`：
  - `omne-agent process inspect <process_id>` / `omne-agent process tail <process_id>` / `omne-agent process follow <process_id>`
- 已知 `artifact_id`：
  - `omne-agent artifact read <thread_id> <artifact_id>`

---

## 4) 清理行为（危险但必要）

- `thread/clear_artifacts` 会删除 `<thread_dir>/artifacts`：
  - 若存在 running process，默认拒绝；`force=true` 会先 kill 再删。
- `thread/delete` 会删除整个 `<thread_dir>`（包括 events 与 artifacts）。
