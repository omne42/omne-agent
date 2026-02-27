# 规格增量：git-domain（E2E 与交接硬化）

## 新增要求

### 要求：Git 领域必须有端到端主链路回归

系统必须提供可重复执行的 E2E 用例，覆盖从子代理隔离执行到线程终态清理的完整链路。

#### 场景：主链路 E2E

- 给定 `subagent/spawn` 使用 `workspace_mode=isolated_write`
- 当子线程完成并写入 `fan_out_result`
- 则系统完成 patch handoff / auto-apply 流程
- 且 `thread/archive` 或 `thread/delete` 后执行 cleanup

### 要求：策略分支必须有 E2E 覆盖

后端策略 `auto|worktree|copy` 的关键分支必须有独立 E2E 断言。

#### 场景：auto 成功

- 给定 `requested_backend=auto`
- 当 worktree 可用
- 则最终 `backend=worktree`

#### 场景：auto 回退

- 给定 `requested_backend=auto`
- 当 worktree 不可用
- 则最终 `backend=copy`
- 且存在 `fallback_reason`

#### 场景：worktree 强制失败

- 给定 `requested_backend=worktree`
- 当 worktree 不可用
- 则返回错误
- 且不得自动回退

#### 场景：copy 强制执行

- 给定 `requested_backend=copy`
- 当执行隔离工作区准备
- 则直接走 copy 路径

### 要求：交接信息必须结构化且可复跑

每个阶段结束时必须记录可供下一位直接执行的信息，不依赖口头上下文。

#### 场景：交接模板完整

- 给定阶段提交完成
- 当填写交接模板
- 则必须包含：当前状态、下一步、阻塞点、复跑命令
- 且命令可在本地直接执行

## 变更要求

### 要求：边界检查纳入阶段验收

E2E 验收必须包含 Git 领域边界扫描，防止 Git 实现回流到 app-server 非 runtime 区域。
