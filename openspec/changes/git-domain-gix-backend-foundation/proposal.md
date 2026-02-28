# 提案：git-domain-gix-backend-foundation

## 做什么

- 在 `omne-git-runtime` 引入 `gix` 后端作为 Git 领域默认实现方向，逐步减少对系统 `git` CLI 的硬依赖。
- 明确本阶段能力边界：
  - 支持：`fetch`、`pull` 主链路（以 `fetch + 更新工作区到已获取状态` 的能力组合为准）。
  - 暂不纳入本阶段验收：`push`。
- 保持架构边界不变：`app-server` 仍只做编排，不新增 Git 过程实现。

## 为什么做

- 当前 `omne-git-runtime` 仍以 `Command::new("git")` 为主，导致运行时依赖用户机器预装 Git。
- 项目目标是“单个可执行程序尽可能自洽”，核心 Git 能力应优先内建在 Rust 领域实现中。
- `gitoxide/gix` 的公开能力列表已明确 `fetch` 可用、`push` 尚未完成，适合作为分阶段迁移起点。

## 怎么做

- 在 `crates/git-runtime` 新增后端抽象：`gix` / `cli`。
- 迁移顺序采用“低风险优先”：
  1. 仓库发现与基础读操作。
  2. `fetch/pull` 相关能力。
  3. `diff/patch/worktree` 的进一步替换或等价实现。
- 对尚未迁移的能力保留受控 fallback（由 runtime 统一管理，而非 app-server 直调 git）。
- 所有迁移步骤必须补单测与 e2e，并更新 OpenSpec 任务状态。

## 非目标

- 本阶段不承诺完整覆盖 `git` CLI 全部子命令。
- 本阶段不引入 `push` 到对外承诺能力（后续视 `gix` 能力与稳定性再评估）。
- 不把 Git 领域迁移到 `safe-fs-tools`。

## 相关文档

- `openspec/specs/git-domain/spec.md`：Git 领域当前生效基线。
- `openspec/specs/git-domain/implementation-roadmap.md`：全链路阶段路线。
- `openspec/changes/git-domain-sequence.md`：变更顺序约束。
- `crates/git-runtime/src/lib.rs`：当前主要待迁移实现。

## 外部依据

- gitoxide README（功能清单）
  - https://github.com/GitoxideLabs/gitoxide/blob/main/README.md
  - 其中高层能力清单明确包含 `fetch`，`push` 仍未勾选。
- gitoxide crate-status（gix remotes/worktrees）
  - https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md

## 验收标准

- 文档层：
  - OpenSpec 明确记录 `fetch/pull` 支持边界与 `push` 非本阶段承诺。
- 架构层：
  - 不新增 `app-server` 内部 Git 过程实现。
- 实现层（本变更完成时）：
  - `omne-git-runtime` 存在可配置后端抽象。
  - 至少一条 Git 主链路由 `gix` 后端执行并有测试覆盖。
- 验证层：
  - `cargo test -p omne-git-runtime` 通过。
  - 边界扫描：`rg -n "Command::new\(\"git\"\)" crates/app-server/src` 不新增命中。
