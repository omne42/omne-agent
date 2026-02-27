# 提案：git-domain-worktree-lifecycle

## 做什么

- 为 Git 领域补齐 worktree 生命周期能力：`remove`、`prune`、可选 `lock`。
- 将 thread 归档/清理流程与 worktree 回收动作联动。

## 为什么做

默认 worktree 后端如果没有生命周期管理，会累积悬挂 worktree 与脏元数据，长期运行会导致资源泄漏与行为漂移。

## 怎么做

- 在 `omne-thread-git-snapshot-runtime` 新增 worktree lifecycle API。
- 在 app-server 的 thread 终态（archive/delete）调用回收 API。
- 失败时记录可观测信息，不阻断主流程，但必须给出修复建议。

## 非目标

- 不在本阶段修改 fan-out result schema。
- 不在本阶段实现跨进程全局 worktree GC 守护进程。

## 验收标准

- 长时间回归后 `git worktree list` 无持续增长泄漏。
- `thread/archive` 与 `thread/delete` 路径具备稳定回收行为。
- `cargo check --workspace` 与相关测试通过。
