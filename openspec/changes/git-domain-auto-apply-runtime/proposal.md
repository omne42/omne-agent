# 提案：Git 自动应用状态机下沉到 Runtime

## 背景

当前 `subagents` 的隔离工作区自动应用流程，虽然底层 `git diff` / `git apply` 命令已下沉到
`omne-thread-git-snapshot-runtime`，但状态机逻辑（前置条件、失败阶段判定、命令参数回传）仍在
`app-server` 中。

这会导致 Git 领域逻辑跨层分散，影响边界清晰度与后续复用。

## 目标

- 将自动应用状态机下沉到 `omne-thread-git-snapshot-runtime`。
- 让 `app-server` 只保留编排与协议 payload 组装职责。
- 保持现有 `fan_out_result` 字段兼容，不引入破坏性变更。

## 非目标

- 不修改 `fan_out_result` schema 版本。
- 不重构子代理调度流程。
- 不迁移到 `safe-fs-tools`。

## 范围

- Runtime crate 新增 auto-apply 领域 API 与失败原因枚举。
- `app-server` 改为调用 runtime API 并映射为原有 JSON payload。
- 补充 runtime 单元测试与 app-server 回归测试。

## 验收标准

- `cargo check --workspace` 通过。
- `cargo test -p omne-thread-git-snapshot-runtime` 通过。
- `cargo test -p omne-app-server fan_out_result_writer_auto_apply` 通过。
- `cargo test -p omne-app-server fan_out_result_writer_auto_applies` 通过。
