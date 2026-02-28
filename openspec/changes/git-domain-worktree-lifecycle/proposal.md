# 提案：git-domain-worktree-lifecycle

## 相关文档

- `openspec/changes/git-domain-sequence.md`：全链路强制顺序（本变更为第 4 阶段）。
- `openspec/specs/git-domain/spec.md`：Git 领域基线要求与边界。
- `openspec/specs/git-domain/implementation-roadmap.md`：整体路线图与阶段衔接。
- `openspec/changes/git-domain-worktree-default/proposal.md`：上一阶段默认 worktree 后端的输入前提。
- `docs/rts_workflow.md`：runtime 分层原则（能力下沉，服务层编排）。
- `/root/autodl-tmp/zjj/p/wsl-docs/00-元语/git-worktree.md`：worktree 运维实践（清理与生命周期）。

## 做什么

- 为 Git 领域补齐 worktree 生命周期能力：`remove`、`prune`（`lock` 保留为可选扩展）。
- 将 thread 终态流程（`thread/archive`、`thread/delete`）与 worktree 回收动作联动。
- 建立“失败不阻断主流程、但必须可诊断”的回收策略。

## 为什么做

- 第 3 阶段把 `isolated_write` 默认切到 worktree 后，系统会持续创建短生命周期工作区。
- 若缺少回收机制，`git worktree list` 会累积悬挂条目，导致磁盘占用和行为漂移。
- 清理能力应归属 Git 领域 runtime，避免在 `app-server` 持续扩散 Git 命令实现。

## 怎么做

- 在 `omne-git-runtime` 新增 worktree 生命周期 API：
  - 识别受管的 detached worktree；
  - 执行 `worktree remove --force`；
  - 执行 `worktree prune` 清理元数据。
- 在 app-server 的 thread 终态调用 runtime API，仅做：
  - 受管路径判定；
  - 生命周期调用编排；
  - 日志/结果映射。
- 回收失败时不阻断 `archive/delete` 主流程，但必须记录失败原因并可追踪。

## 非目标

- 不在本阶段修改 fan-out result schema（观测字段放到下一阶段）。
- 不在本阶段实现跨进程全局 GC 守护进程。
- 不在本阶段引入新的仓库复制/快照后端。

## 验收标准

- 行为正确性：
  - `thread/archive`、`thread/delete` 对受管 worktree 执行回收后，worktree 目录被移除，且 `git worktree list` 无残留。
  - 非受管目录或非 worktree 目录不误删。
- 架构边界：
  - `app-server` 不新增直接 `git` 命令实现；Git 生命周期逻辑归属 `omne-git-runtime`。
- 可验证检查：
  - `cargo test -p omne-git-runtime`
  - `cargo test -p omne-app-server thread_archive_`
  - `cargo test -p omne-app-server thread_delete_`
  - `rg -n "Command::new\\(\\\"git\\\"\\)" crates/app-server/src/main/thread_manage`（应无新增命中）
