# 规格增量：git-domain（worktree 默认后端）

## 新增要求

### 要求：isolated_write 默认使用 worktree

当 `subagent/spawn` 选择 `workspace_mode=isolated_write` 时，系统必须优先使用 Git worktree 创建隔离工作区。

#### 场景：Git 仓库 worktree 创建成功

- 给定父工作区是有效 Git 仓库
- 当创建隔离工作区
- 则默认执行 `git worktree add --detach`
- 且返回该 worktree 路径作为子线程 cwd

#### 场景：非 Git 仓库或 worktree 失败

- 给定父工作区不是 Git 仓库，或 worktree 创建失败
- 当创建隔离工作区
- 则自动回退 copy fallback
- 且不中断 subagent 启动流程

### 要求：worktree 实践边界

worktree 路径规划与维护需符合工程边界：路径可预测、避免污染主目录、支持后续 prune/清理。

#### 场景：路径规范

- 给定 task_id 与 parent_thread_id
- 当生成隔离路径
- 则路径位于 `.omne_data/tmp/subagents/...` 命名空间
- 且不与已有路径冲突

## 变更要求

### 要求：Git 领域归属

worktree 创建能力必须归属于 `omne-thread-git-snapshot-runtime`，
app-server 仅负责编排与 fallback 策略决策。
