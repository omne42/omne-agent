# 规格增量：git-domain（worktree 生命周期）

## 新增要求

### 要求：受管 worktree 在 thread 终态必须触发回收

当线程工作目录属于受管 isolated worktree 时，`thread/archive` 与 `thread/delete`
必须触发 Git 领域 runtime 的回收流程（`remove + prune`）。

#### 场景：thread/archive 触发受管 worktree 回收

- 给定线程 cwd 是受管 detached worktree
- 当执行 `thread/archive`
- 则系统调用 runtime 的 worktree 生命周期 API
- 且 worktree 路径被移除
- 且 `git worktree list` 不再包含该路径

#### 场景：thread/delete 触发受管 worktree 回收

- 给定线程 cwd 是受管 detached worktree
- 当执行 `thread/delete`
- 则系统调用 runtime 的 worktree 生命周期 API
- 且 worktree 路径被移除
- 且主流程返回成功

### 要求：非受管路径不得误删

系统必须避免将普通仓库目录或非 worktree 目录误判为可回收 worktree。

#### 场景：普通目录不触发删除

- 给定线程 cwd 不是受管 detached worktree
- 当执行 `thread/archive` 或 `thread/delete`
- 则不触发 worktree remove
- 且不影响原有线程管理行为

### 要求：失败不阻断主流程

worktree 生命周期回收失败时，系统必须保持 `archive/delete` 主流程可用，
并输出可诊断错误信息。

#### 场景：回收失败

- 给定受管 worktree 回收命令失败
- 当执行 `thread/archive` 或 `thread/delete`
- 则主流程仍返回成功
- 且记录失败原因用于后续排障

## 变更要求

### 要求：Git 生命周期能力归属 runtime

`app-server` 不得新增直接 Git 命令实现；Git 生命周期逻辑应位于
`omne-git-runtime`。
