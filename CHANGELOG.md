# Changelog

本项目的所有重要变更都会记录在这个文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added
- Hook 执行现在会把 stdout/stderr 写入 session artifacts：`/tmp/<repo>_<session_id>/logs/hook.{stdout,stderr}.log`，便于排查失败原因。
- `--pm-root` / `CODE_PM_ROOT`：允许覆盖 `.code_pm` 数据目录位置（相对路径按 repo root 解析）。
- 兼容提示：当默认使用 `.code_pm` 且检测到旧目录 `.codex_pm` 存在（但 `.code_pm` 尚未创建）时，CLI 会输出 warning，提示手动迁移或通过 `--pm-root .codex_pm` / `CODE_PM_ROOT=.codex_pm` 复用旧数据。
- `code-pm session list/show`：从本地 `.code_pm/data/` 查询 session（默认优先输出 `result`；`--all` 输出 session/tasks/prs/merge/result）。
- `code-pm session list [--limit N] [--json]`：按 session id 顺序列出 sessions；`--json` 输出 JSON 数组。
- `code-pm session list --verbose [--limit N] [--json]`：按 `created_at`（RFC3339）倒序输出 session 元信息（不含 prompt）；`--json` 输出 JSON 数组。
- `code-pm run --json`：以 pretty JSON 输出 `RunResult`（便于脚本/集成）。
- `code-pm run --strict`：当存在 task 失败或 merge error 时返回非零退出码。
- `code-pm run --max-concurrency`：现在会校验为 `>= 1`（拒绝 `0`，避免静默回退到 `1`）。
- `code-pm run --stream-events-json`：以 JSON Lines（NDJSON）格式输出 `RunEvent` 到 stderr（每行包含 `type` 字段），便于实时消费事件流。
- `code-pm run --hook-url <url>`：完成后向 webhook `POST` JSON（`session_id/repo/pr_name/base_branch/pm_root/session_dir/tmp_dir/result_json/merged/merge_error`）。
- `pm-http GET /api/v0/sessions`：新增 `?verbose=true`（返回 `SessionMeta[]`）与 `?limit=N`（截断结果）；默认仍返回 `SessionId[]`。
- `pm-http`：query flag 支持无值形式（`?verbose` / `?all` 等价于 `=true`）。
- `pm-http GET /api/v0/sessions/:id`：默认返回 `result`；`?all=true` 返回 session/tasks/prs/merge/result bundle。
- `pm-http GET /api/v0/sessions/:id/meta`：返回 `SessionMeta`（不含 prompt）。
- `pm-http`：支持 `CODE_PM_HTTP_MAX_BODY_BYTES`（默认 1GiB）限制 git smart-http 请求体大小；超限返回 413。
- `pm-http serve`：强制 loopback-only（拒绝绑定非 `127.0.0.1`/`::1` 地址），避免无鉴权服务被意外暴露到公网。
- `pm-http git`：拒绝除 GET/POST 之外的 HTTP method（返回 405），减少非预期攻击面。
- `pm-http git`：405 响应包含 `Allow: GET, POST`，便于客户端/调试工具正确处理。
- Orchestrator：改进并发调度（以 worker-pool 方式限流，不再一次性 spawn 所有 tasks；不会再因为并发限流导致 `TaskFinished` 事件延迟），并在 `--max-concurrency 1` 场景下也会把 task 的 panic/cancel 转为 `Failed` PR（避免直接崩溃整个 session）。
- `pm-git`：identity 配置步骤现在会严格校验每一步的 `ok`（避免静默忽略 `git config --get` 的异常失败）。
- `code-pm run --task/--tasks-file`：显式空 task id 现在会被拒绝（避免意外回退到 `task` 造成重复/混乱）。
- session 列表查询改为直接枚举 `.code_pm/data/sessions/` 目录（更快，且只返回合法 UUID）。
- `Session/SessionMeta.created_at` 的 JSON 表达改为 RFC3339 字符串（读仍兼容旧 tuple/unix timestamp）。
- session 元信息新增独立存储 `sessions/<id>/meta.json`（`list_session_meta` 优先读取，避免为列表读入大 prompt）。
- `pm-http` 的 session API（`/api/v0/sessions/:id/*`）现在只接受 UUID 格式的 session id，避免非法路径段触发 storage key 校验错误导致 500。
- `CODE_PM_TMP_ROOT` 为空（例如 `export CODE_PM_TMP_ROOT=`）时将被忽略，避免 session artifacts 意外落到当前工作目录。
- `code-pm repo inject` 与 `code-pm run --repo-src` 现在支持使用相对路径引用本地仓库（不会再相对 `.code_pm/repos` 解析导致 clone 失败）。
- `code-pm repo inject` 复用已存在的注入仓库时会更新 `origin` 到本次传入的 source，避免同名重新注入仍从旧源 fetch。
- `code-pm repo inject` 复用已存在的 bare repo 时会以 mirror refspec 拉取到 `refs/*`（而不是只更新 `refs/remotes/origin/*`），避免后续 clone 看不到 `refs/heads/*`。
- storage 读取损坏 JSON 时增加错误上下文（包含具体文件路径），便于定位问题数据。
- storage 写入 JSON 失败时会尽力清理临时文件（`*.json.tmp.*`），避免脏文件堆积。
- hook 执行失败的错误现在会包含 session id，便于定位失败运行与对应 artifacts。
- `pm-http` 构造 fallback 响应时现在会正确设置 HTTP status（避免极端情况下误返回 200）。
- `code-pm run --stream-events-json` 在序列化异常时也会输出 JSON 行，避免污染 NDJSON 流。
- `code-pm run --stream-events-json` 在 consumer 落后导致事件被丢弃时会输出 error JSON 行（`event_lagged`），避免静默丢事件。

## [0.1.0] - 2026-01-20

### Added
- 初始 CodePM Rust workspace（`code-pm` CLI + `pm-core`/`pm-git`/`pm-http`），落地 Phase 1 骨架：repo 注入、/tmp session/task 目录、任务 clone/commit/push、本地顺序合并。
- `code-pm init`：初始化 `.code_pm/` 数据目录布局。
- Repo 管理：`code-pm repo inject/list`，注入本地路径或远端 URL 到 `.code_pm/repos/<name>.git` bare repo。
- Run 编排：`code-pm run` 创建 session，产出 `/tmp/<repo>_<session_id>/...` artifacts 与 `.code_pm/data/sessions/<id>/` JSON 数据（session/tasks/prs/merge/result）。
- 并发 task：每 task 独立 clone 到 `/tmp/.../tasks/<task_id>/repo`，可用 `--max-concurrency` 并行；事件流 `--stream-events`。
- 任务拆分：`--task/--tasks-file` 覆盖任务列表；`--auto-tasks` 从 prompt Markdown 列表规则化拆分。
- Git coder 流水线：clone →（可选）`git apply` →（Rust repo）`cargo fmt`/`cargo check`（`CARGO_TARGET_DIR` 指向 artifacts）→ commit → push 分支 `ai/<pr_name>/<session_id>/<task_id>`。
- Merge：顺序合并 Ready PR 分支回 base（默认 `main`），并保留每一步日志到 `merge/artifacts/*.log`。
- Smart HTTP（增强）：`code-pm serve`（loopback-only）提供 `git clone/push` over HTTP（`git http-backend`）。
- Hook：完成后可执行命令 hook，并通过 `CODE_PM_*` 环境变量传递 session 上下文；临时目录根可用 `CODE_PM_TMP_ROOT` 覆盖。
