# Changelog

本项目的所有重要变更都会记录在这个文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

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
