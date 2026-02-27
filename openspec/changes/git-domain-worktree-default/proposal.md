# 提案：isolated_write 默认使用 git worktree

## 背景

当前 `isolated_write` 工作区主要通过目录复制实现。该模式可用，但与“专属 Git 领域”的目标不一致：
Git 相关隔离策略应优先复用 Git 自身机制（worktree），而不是以文件复制为主。

Claude Code 官方工作流与 Git 官方文档都强调可并行 worktree 会话。

## 目标

- `isolated_write` 默认后端切换为 `git worktree`。
- 当源目录非 Git 仓库或 worktree 创建失败时，自动回退 copy fallback，确保不中断。
- 将 worktree 创建能力收敛到 `omne-thread-git-snapshot-runtime`（Git 领域）。

## 非目标

- 不在本阶段实现完整 worktree 回收策略（如 thread archive 自动 remove/prune）。
- 不修改 fan-out result schema。

## 范围

- Runtime crate 新增 worktree create 原语。
- app-server `prepare_isolated_workspace` 改为“worktree first, copy fallback”。
- 补充覆盖 worktree 成功路径与 fallback 路径的测试。

## 参考依据

- Claude Code 官方教程（并行 worktree 会话）：
  https://code.claude.com/docs/en/tutorials#run-parallel-claude-code-sessions-with-git-worktrees
- Git 官方 `git-worktree` 文档：
  https://git-scm.com/docs/git-worktree
- 仓库现有方向：`docs/rts_workflow.md`、`docs/v0.2.0_parity.md`、`docs/implementation_plan.md`

## 验收标准

- `cargo check --workspace` 通过。
- `cargo test -p omne-thread-git-snapshot-runtime` 通过。
- `cargo test -p omne-app-server fan_out_result_writer_auto_applies` 通过。
- `cargo test -p omne-app-server fan_out_result_writer_auto_apply` 通过。
