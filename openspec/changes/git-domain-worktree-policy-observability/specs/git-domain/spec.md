# 规格增量：git-domain（worktree 策略与可观测性）

## 新增要求

### 要求：隔离后端策略必须显式可控

系统必须支持 `auto | worktree | copy` 三种后端策略，并对每种策略执行一致行为。

#### 场景：auto 策略

- 给定后端策略为 `auto`
- 当 worktree 创建成功
- 则使用 `worktree` 作为最终后端
- 且不写入 `fallback_reason`

#### 场景：auto 策略回退

- 给定后端策略为 `auto`
- 当 worktree 创建失败
- 则回退到 `copy`
- 且写入 `fallback_reason`

#### 场景：worktree 强制策略

- 给定后端策略为 `worktree`
- 当 worktree 创建失败
- 则返回失败
- 且不得自动回退 `copy`

#### 场景：copy 强制策略

- 给定后端策略为 `copy`
- 当创建隔离工作区
- 则直接使用 copy 路径
- 且不尝试 worktree

### 要求：isolated_write 结果必须包含后端观测字段

`isolated_write` 结果必须包含策略请求值和最终后端，便于回放与排障。

#### 场景：结构化字段存在

- 给定任一后端策略
- 当 `isolated_write` 任务完成
- 则结果包含 `requested_backend` 和 `backend`
- 且字段值可枚举（`auto|worktree|copy` / `worktree|copy`）

### 要求：Git 能力边界保持稳定

Git 命令执行能力应继续留在 runtime，app-server 仅做策略解析和结果映射。

#### 场景：边界检查

- 给定 app-server 调度实现
- 当执行边界扫描
- 则不新增 `Command::new("git")` 实现

## 变更要求

### 要求：策略非法值回退安全默认

当策略配置值非法时，系统必须回退为 `auto`，并输出可诊断信息。
