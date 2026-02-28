# 规格增量：git-domain（auto-apply runtime）

## 新增要求

### 要求：Runtime 自动应用状态机

`omne-git-runtime` 必须提供自动应用状态机 API，覆盖：
前置条件校验、patch 抓取、`git apply --check`、`git apply`。

#### 场景：前置条件失败

- 给定子线程状态不是 completed，或缺失目标工作区
- 当执行自动应用 API
- 则返回 `Precondition` 阶段失败
- 且给出可诊断的失败原因

#### 场景：检查阶段失败

- 给定 patch 在目标工作区 `git apply --check` 冲突
- 当执行自动应用 API
- 则返回 `CheckPatch` 阶段失败
- 且保留建议执行的 `check/apply` 命令参数

#### 场景：应用成功

- 给定 patch 可通过 check 且可应用
- 当执行自动应用 API
- 则返回 `applied=true`，并记录执行过的命令参数

## 变更要求

### 要求：App-Server 作为映射层

`app-server` 在 auto-apply 路径中必须调用 runtime 状态机 API，
仅负责协议字段映射、attention marker 与恢复命令组装。
