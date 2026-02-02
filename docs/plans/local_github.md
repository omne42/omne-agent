# Plan: Local GitHub (Forgejo)

## Goal

在本机提供一个**真实可运行**的“GitHub 风格”代码托管与 PR 系统（repo hosting + PR UI/API），让 `omne-agent` 可以：

- 从本地 forge 拉取 repo 到临时目录
- 为每个任务创建分支、提交变更、推送到远端
- 创建 PR（可选：跑 checks、合并）
- **所有 PR 的命名都必须额外带 `omne/` 前缀**（至少覆盖：head branch；以及 PR title/显示名）

## Reality check

“GitHub”本身不是一个可直接内嵌/可自由分发的本地实现。为了满足“真实运行在本地 + 有 PR”的需求，我们选择一个成熟的开源 forge：

- **Forgejo（推荐）**：提供 Git hosting（HTTP/SSH）、PR、Web UI、API、用户/Token、Webhook。
- 备选：Gitea（Forgejo 的上游生态之一）、GitLab（更重）。

本计划把 Forgejo 当作“本地 GitHub 实现”的落地载体；`omne-agent` 只做适配与流程编排。

## Non-goals

- 不追求 GitHub API 100% 兼容（我们只需要 PR 工作流的最小子集）。
- 不内嵌整个 forge 的实现到 Rust 二进制（先外置服务；后续再评估是否需要进一步内嵌）。
- 不在第一阶段支持复杂权限模型（先 loopback-only + token）。

## Constraints

- **安全边界**：必须尊重现有 `sandbox_network_access` / `execpolicy` / approvals。禁网时仅允许 loopback（`127.0.0.1` / `localhost`）的本地 forge。
- **可审计**：关键动作（clone/branch/commit/push/create_pr/merge）必须落盘为事件与 artifacts（至少记录：repo、分支、PR URL、checks 结果）。
- **命名规范**：统一 `omne/` 前缀，避免与用户已有分支/PR 冲突。
- **最小依赖**：第一阶段允许依赖系统 `git`（用于 clone/push 等），后续再对齐 `docs/plans/embedded_git.md` 的“无系统 git”方向。
- **磁盘可控**：批量并发 task 时不得出现“每个 task 一套 `target/`”的线性爆炸；必须共享 build cache，并在过大时提醒用户。

## Architecture sketch

### External service

- Forgejo（loopback-only）
  - Git remote：HTTP(S) 或 SSH
  - API：创建/查询/合并 PR

### omne-agent internal components (to build)

- `ForgeClient`
  - `create_pull_request(base, head, title, body) -> pr_url`
  - `list_pull_requests(...)`
  - `merge_pull_request(...)`（可选）
- `GitRunner`
  - Phase 1：shell out to `git`（严格走 `process/start` + `execpolicy` + approvals）
  - Phase 2：可替换为内嵌实现（见 `docs/plans/embedded_git.md`）
- `PrNaming`
  - `head_branch = "omne/ai/<pr_name>/<session_id>/<task_id>"`
  - `pr_title = "omne/<pr_name>: <task title>"`
- `RepoCache`
  - 本地 mirror：`.omne_agent_data/repos/<repo>.git`（bare mirror）
  - 任务 workspace：临时目录（每 task 独立，便于并发与回滚）
- `BuildCache`
  - **每个 repo 只允许一个共享的 `CARGO_TARGET_DIR`**（位于 `agent_root` 下的持久化目录），所有 task worktree 复用该目录，避免并发数 × `target/` 的磁盘爆炸。
  - cargo 并发策略：task 可以 20+ 并发；但同一 repo 的 cargo 构建默认会因 target-dir lock 自动串行（或由我们显式限流）。
  - 过大提醒：当共享 `CARGO_TARGET_DIR` 超过阈值（默认阈值可调）时，向用户提示并生成清理建议 artifact（例如建议 `cargo clean` 或清理 `incremental/`）。

## Local dev setup (manual)

> 这里先定义目标形态；具体启动脚本/compose 在实现阶段补齐。

- 启动 Forgejo（loopback-only），创建一个 admin 用户并生成 API token。
- 在 Forgejo 上创建/导入 repo（或由 `omne-agent` 通过 API 自动创建）。
- 配置 `omne-agent` 连接信息（建议 env / project config 二选一）：
  - `OMNE_AGENT_FORGE_BASE_URL`（例如 `http://127.0.0.1:3000`）
  - `OMNE_AGENT_FORGE_TOKEN`
  - `OMNE_AGENT_FORGE_OWNER`
  - `OMNE_AGENT_FORGE_REPO`

## Definition of Done (DoD)

- Forgejo 本地可启动，且 `omne-agent` 可用 token 调通 API。
- `omne-agent` 端到端完成一次 PR 流程：
  - clone/fetch → 创建分支 → commit → push → create PR
  - PR 的 head branch 与 title 均带 `omne/` 前缀
  - 产出 PR URL，并落盘到 artifacts
- Rust gates 全绿：
  - `cargo fmt --all`
  - `cargo check --workspace --all-targets`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Task DAG

### t1 - 规范与配置面（spec）

- Files:
  - `docs/plans/local_github.md`
  - （如需要）`docs/development_process.md`
- Acceptance:
  - 明确 `omne/` 前缀策略覆盖面（branch/title）
  - 明确 loopback-only 的安全策略与失败语义

### t2 - Forge 配置落地（实现）

- Files:
  - `crates/core`（新增 Forge 连接配置与序列化结构）
  - `crates/app-server`（读取 project config/env 并注入到执行路径）
- Acceptance:
  - 能解析 base_url/token/owner/repo
  - token 不写入明文 logs；必要时走现有 redaction

### t3 - ForgeClient（实现 + 测试）

- Files:
  - 新增 `crates/forge/`（建议）或放在 `crates/core` 内部模块（如耦合很小）
- Acceptance:
  - `create_pull_request` 可用（最小字段：base/head/title/body）
  - 单元测试用 mock HTTP server 覆盖成功/失败/鉴权失败

### t4 - Git 工作流与并发隔离（实现）

- Acceptance:
  - 每个 task 在独立临时目录执行（并发安全）
  - 分支名按 `PrNaming` 生成，并 push 到 Forgejo remote

### t5 - PR 产物与可审计输出（实现）

- Acceptance:
  - 将 PR URL、head branch、head commit、checks 结果落盘
  - 失败时给出可复现命令与日志路径

### t6 - 可选：集成测试（带开关）

- Acceptance:
  - 本地有 Forgejo 时可跑 e2e；CI 默认跳过（避免引入重量级依赖）
