# 运行时目录结构（`pm_root` = `./.codepm_data/`）（v0.2.0 口径）

> 目标：让用户/工具在 30 秒内定位：事件落盘在哪、process log 在哪、user artifact 在哪。
>
> `pm_root` 默认是 `./.codepm_data/`：运行时数据（threads/artifacts）+ 项目级配置（`.codepm_data/config.toml`、`.codepm_data/spec/`）。

---

## 0) pm_root 的选择（写死）

`pm_root` 是 `.codepm_data/` 的根目录：

- 直接运行 app-server（`pm-app-server`）：默认 `$(pwd)/.codepm_data`
- 通过 `pm` CLI：默认会显式传 `--pm-root`（优先级：CLI flag > env `CODE_PM_ROOT` > `$(pwd)/.codepm_data`）
- 覆盖方式：
  - CLI：`pm-app-server --pm-root <path>`
  - env：`CODE_PM_ROOT=<path>`

---

## 1) threads 与事件日志（append-only）

每个 thread 一个目录：

```
<pm_root>/
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
<pm_root>/threads/<thread_id>/artifacts/
```

### 2.1 process logs（stdout/stderr）

每个 process 一个目录：

```
<pm_root>/threads/<thread_id>/artifacts/processes/<process_id>/
  stdout.log
  stdout.segment-0001.log        # 超过阈值后 rotate
  stdout.part-0001.log           # 兼容命名（如出现）
  stderr.log
  stderr.segment-0001.log
  stderr.part-0001.log           # 兼容命名（如出现）
```

要点：

- `process/start` 会在事件里落盘 `ProcessStarted{stdout_path,stderr_path}`，并在返回值里直接带路径。
- rotate 阈值默认 `8MiB`，可用 `CODE_PM_PROCESS_LOG_MAX_BYTES_PER_PART` 覆盖。

### 2.2 user artifacts（`artifact/write`）

用户可见的文档产物（markdown + metadata）：

```
<pm_root>/threads/<thread_id>/artifacts/user/
  <artifact_id>.md
  <artifact_id>.metadata.json
```

metadata 字段模型：`pm_protocol::ArtifactMetadata`（见 `crates/agent-protocol/src/lib.rs`）。

---

## 3) 如何“从 ID 定位到文件”

- 已知 `thread_id`：
  - `pm thread events <thread_id>`（或 JSON-RPC `thread/subscribe`）看 `events.jsonl` 的回放结果
  - `pm artifact list <thread_id>` 查 user artifacts（返回 metadata + 路径）
  - `pm process list --thread <thread_id>` 查 processes（返回 stdout/stderr 路径）
- 已知 `process_id`：
  - `pm process inspect <process_id>` / `pm process tail <process_id>` / `pm process follow <process_id>`
- 已知 `artifact_id`：
  - `pm artifact read <thread_id> <artifact_id>`

---

## 4) 清理行为（危险但必要）

- `thread/clear_artifacts` 会删除 `<thread_dir>/artifacts`：
  - 若存在 running process，默认拒绝；`force=true` 会先 kill 再删。
- `thread/delete` 会删除整个 `<thread_dir>`（包括 events 与 artifacts）。
