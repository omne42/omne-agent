# CodePM 端到端流程（Runbook）

> 本文描述 **当前已实现** 的 CodePM（Rust）端到端使用流程：repo 注入 → session/task 并发执行 → 本地 PR 分支生成 → 顺序合并 → hook 回调 → 查询与回溯。
>
> 现状提醒：`Coder/Merger` 目前是 **脚本化 Git 流水线**（可选 `git apply` + Rust `cargo fmt/check` + commit/push/merge），并非“真正的 AI 写码/审阅/合并”。AI 角色落地见 `docs/implementation_plan.md`。

---

## 0. 目标与边界（别自欺欺人）

- 目标：在本机用 Rust 跑通“多 task 并发 + 临时目录隔离 + git 分支流水线 + 合并 + hook”的最小闭环。
- 当前已实现：repo 注入（bare/mirror）、task 并发、任务 workspace、Rust fmt/check gate、push 分支、顺序 merge、session 存储与查询、Smart HTTP（loopback-only）。
- 当前未实现：真实 AI Coder/Reviewer/Merger、冲突自动修复、GitHub PR、分布式 worker、可配置的多语言 checks。
- 前置依赖：`git`、Rust toolchain（workspace 需要 `rustc >= 1.85`）、可选 `curl`（用于调用 HTTP API 示例）。

---

## 1. 目录与数据（你得知道东西落哪儿）

### 1.1 `.code_pm/`（持久化）

默认在“repo root”下（`git rev-parse --show-toplevel`；不在 git repo 内则为当前目录）：

```
.code_pm/
  repos/          # bare repos：{repo}.git
  data/           # sessions JSON：sessions/<id>/*.json
  locks/          # repo locks：{repo}.lock
```

覆盖方式：

- CLI：`code-pm --pm-root /path/to/.code_pm ...`（相对路径按 repo root 解析）
- 环境变量：`CODE_PM_ROOT=/path/to/.code_pm`

兼容提示：如果检测到旧目录 `.codex_pm` 且 `.code_pm` 还不存在，CLI 会输出 warning（你可以 `mv .codex_pm .code_pm` 或 `--pm-root .codex_pm`）。

### 1.2 `/tmp/<repo>_<session_id>/`（一次运行的 artifacts）

默认在 `/tmp`（或 `CODE_PM_TMP_ROOT` 覆盖）：

```
/tmp/<repo>_<session_id>/
  session.json
  result.json
  logs/                        # hook logs
  tasks/<task_id>/
    repo/                      # 独立 clone 工作区
    artifacts/                 # git/cargo steps logs（含 cargo-target/）
    task.json
  merge/
    repo/
    artifacts/                 # merge logs
```

---

## 2. 初始化 + 注入仓库（Repo Injection）

初始化本地数据目录布局：

```bash
cargo run -p code-pm -- init
```

注入一个仓库（本地路径或远端 URL）到 `.code_pm/repos/<name>.git`：

```bash
cargo run -p code-pm -- repo inject <repo_path_or_url> --name <repo_name>
```

说明：

- `--name` 可省略（会从 source 推导）。
- `<repo_name>` 允许带可选 `.git` 后缀（会被规范化掉，避免 `.git.git`）。
- 本地路径支持相对路径；会被解析为绝对路径后再传给 `git clone`/`git remote set-url`。

列出已注入仓库：

```bash
cargo run -p code-pm -- repo list
cargo run -p code-pm -- repo list --verbose
cargo run -p code-pm -- repo list --json
cargo run -p code-pm -- repo list --verbose --json
```

---

## 3. 可选：启动本地 Git Smart HTTP + 查询 API

启动服务（仅允许 loopback）：

```bash
cargo run -p code-pm -- serve --addr 127.0.0.1:9417
```

通过 HTTP clone/push bare repo：

```bash
git clone http://127.0.0.1:9417/git/<repo_name>.git
git push  http://127.0.0.1:9417/git/<repo_name>.git <branch>
```

HTTP JSON API（只读查询）：

```bash
curl -s http://127.0.0.1:9417/api/v0/repos
curl -s 'http://127.0.0.1:9417/api/v0/repos?verbose'
curl -s 'http://127.0.0.1:9417/api/v0/sessions?limit=20'
curl -s 'http://127.0.0.1:9417/api/v0/sessions?verbose&limit=20'
curl -s http://127.0.0.1:9417/api/v0/sessions/<uuid>
curl -s 'http://127.0.0.1:9417/api/v0/sessions/<uuid>?all'
curl -s http://127.0.0.1:9417/api/v0/sessions/<uuid>/meta
```

可调项：

- `CODE_PM_HTTP_MAX_BODY_BYTES`：限制 git smart-http 请求体大小（默认 `1GiB`）。

---

## 4. 运行一次 session（核心）

你需要提供：

- `--pr-name <name>`
- prompt：`--prompt <text>` 或 `--prompt-file <path>`（二选一，且内容不能为空）
- repo（3 选 1）：
  - `--repo <name>`：使用已注入的 bare repo
  - `--repo-src <path_or_url>`：以 source 自动注入/更新并运行
  - 如果当前目录在一个 git repo 内（存在 `.git`），可省略 `--repo/--repo-src`，会默认以当前 `repo_root` 作为 `--repo-src` 注入并运行（repo 名默认为目录名（去掉可选 `.git` 后缀后）sanitize）

最小示例（单 task，模板拆分）：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "your spec + requirements + goals"
```

如果你就在目标 git repo 内运行，也可以省略 `--repo/--repo-src`：

```bash
cargo run -p code-pm -- run \
  --pr-name <pr_name> \
  --prompt "your spec + requirements + goals"
```

可选：对每个 task 先应用同一个 patch（用于验证流水线）：

```bash
git diff > /tmp/change.patch
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --apply-patch /tmp/change.patch
```

对 Rust repo 可选增加 `--cargo-test`（在提交前额外执行 `cargo test --workspace --all-targets`）：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --cargo-test
```

### 4.1 task 拆分方式（3 选 1）

1. 默认：模板 `TemplateArchitect`（永远 1 个 task：`main`）。

2. `--auto-tasks`：`RuleBasedArchitect` 从 prompt 的 checklist/列表/编号列表里提取 task（上限 8）。

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt $'...\n- [ ] task A\n- [ ] task B\n' \
  --auto-tasks \
  --max-concurrency 2
```

3. 覆盖：显式指定 tasks（绕过 `Architect`）

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --task a:apply-patch \
  --task b:apply-patch \
  --max-concurrency 2
```

或使用 JSON 文件：

```json
{
  "tasks": [
    { "id": "a", "title": "apply patch A" },
    { "id": "b", "title": "apply patch B", "description": "optional" }
  ]
}
```

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --tasks-file /tmp/tasks.json \
  --max-concurrency 2
```

注意：`--auto-tasks` 不能与 `--task/--tasks-file` 同时使用。

### 4.2 并发与事件流

- 并发：`--max-concurrency N`（必须 `>= 1`）。
- 事件流（stderr）：
  - `--stream-events`：文本
  - `--stream-events-json`：NDJSON（每行带 `type` 字段）

### 4.3 输出与退出码

- `--json`：输出 `RunResult` pretty JSON（stdout）。
- `--strict`：只要有 task 失败或 merge error 就返回非零退出码（便于脚本/CI）。

---

## 5. Hooks（session 完成后的回调）

### 5.1 Command hook（本地命令）

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --hook-cmd /usr/bin/env \
  --hook-arg bash --hook-arg -lc --hook-arg 'echo "$CODE_PM_SESSION_ID $CODE_PM_RESULT_JSON"'
```

环境变量（关键）：

- `CODE_PM_SESSION_ID`
- `CODE_PM_REPO`
- `CODE_PM_PR_NAME`
- `CODE_PM_PM_ROOT`
- `CODE_PM_SESSION_DIR`（`.code_pm/data/sessions/<id>`）
- `CODE_PM_TMP_DIR`（`/tmp/<repo>_<id>`）
- `CODE_PM_RESULT_JSON`（`/tmp/.../result.json`）
- `CODE_PM_MERGED`（`1`/`0`）

hook stdout/stderr 会写入：`/tmp/<repo>_<id>/logs/hook.{stdout,stderr}.log`。

### 5.2 Webhook hook（HTTP POST JSON）

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --hook-url http://127.0.0.1:3000/webhook
```

payload 字段包含：`session_id/repo/pr_name/base_branch/pm_root/session_dir/tmp_dir/result_json/merged/merge_error`。

---

## 6. 查询与回溯（别靠猜）

列出 sessions：

```bash
cargo run -p code-pm -- session list
cargo run -p code-pm -- session list --limit 20
cargo run -p code-pm -- session list --json
cargo run -p code-pm -- session list --verbose
```

查看某个 session（默认优先输出 `result`；`--all` 输出 bundle）：

```bash
cargo run -p code-pm -- session show <uuid>
cargo run -p code-pm -- session show <uuid> --all
```

日志位置（排查失败优先看这里）：

- task：`/tmp/<repo>_<id>/tasks/<task>/artifacts/*.log`
- merge：`/tmp/<repo>_<id>/merge/artifacts/*.log`
- hook：`/tmp/<repo>_<id>/logs/hook.{stdout,stderr}.log`

---

## 7. 贡献流程（改 CodePM 自己）

启用 git hooks（强制 Conventional Commits + Rust fmt/check gate）：

```bash
./scripts/setup-githooks.sh
```

常用验证命令：

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo test --workspace
```

变更记录：

- 所有重要改动写入 `CHANGELOG.md` 的 `[Unreleased]`。

---

## 8. 下一步（按现实优先级）

- 把 `Coder/Merger/Architect` 从“脚本”替换为真实 AI agent（建议先让 AI 输出 patch，再复用现有 `git apply + checks + commit/push`）。
- 把 checks 做成可配置 pipeline（至少：Rust `clippy/test` 可选；非 Rust repo 的通用 checks/hook）。
- 增加控制面：支持通过 HTTP 触发 `run`、订阅事件流、管理 repo/session（当前 HTTP 只覆盖 git smart-http + 查询）。
- 若要走 `codex-rs` 复用路线，先把 `docs/implementation_plan.md` 的 codex fork 结构与当前 workspace 对齐，避免两套架构并行互相打脸。
