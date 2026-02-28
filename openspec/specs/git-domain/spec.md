# Git 领域规格

## 目标

在 `omne-agent` 内定义专属 Git runtime 领域，承接 thread/subagent 流程中可复用的 Git 操作。
该领域归属必须保留在 `omne-agent` 内（不迁移到 `safe-fs-tools`）。

## 要求

### 要求：Thread Snapshot Runtime 归属

`thread/diff` 与 `thread/patch` 必须使用 `omne-git-runtime` 提供的 recipe 与 limits。

#### 场景：snapshot recipe 标识稳定

- 给定 `SnapshotKind::Diff`
- 当 runtime 解析 recipe
- 则 argv 包含无颜色、无 textconv 的 git diff 基线参数
- 且 artifact 类型为 `diff`

- 给定 `SnapshotKind::Patch`
- 当 runtime 解析 recipe
- 则 argv 包含 `--binary --patch`
- 且 artifact 类型为 `patch`

### 要求：隔离工作区 Patch 抓取与应用复用

隔离子代理工作区交接所需的 Git patch 抓取/应用原语，必须实现为可复用 runtime 函数，不能在 app-server 流程代码中重复实现。

#### 场景：干净工作区抓取

- 给定隔离工作区没有文件变更
- 当执行 patch 抓取
- 则 runtime 返回无 patch

#### 场景：脏工作区抓取

- 给定隔离工作区存在已跟踪或未跟踪变更
- 当执行 patch 抓取
- 则 runtime 返回 patch 文本
- 且当超出字节预算时返回截断标记

#### 场景：通过 stdin 应用 patch

- 给定 patch 文本与目标工作区 cwd
- 当 runtime 执行 `git apply --check` 或 `git apply`
- 则通过 stdin 写入 patch 内容
- 且失败时返回可行动的错误上下文

### 要求：App-Server 作为编排层

`crates/app-server` 可以继续保留 policy/env/attention 编排逻辑，但 Git 子进程操作必须调用 Git runtime 函数。

#### 场景：编排边界

- 给定 fan-out result auto-apply 流程
- 当 app-server 执行前置条件检查、marker 更新与 payload 组装
- 则 Git 命令执行本身委托给 runtime crate API
