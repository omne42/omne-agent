# 任务：git-domain-auto-apply-runtime

## 1. 文档与约束

- [x] 编写本阶段 proposal。
- [x] 编写 spec delta。
- [x] 约束 app-server 仅保留编排与 payload 映射。

## 2. Runtime 实现

- [x] 新增自动应用结果模型（成功/失败阶段/失败原因）。
- [x] 新增自动应用入口 API（封装前置校验、抓取、check、apply）。
- [x] 保留 `capture_workspace_patch` 与 `run_git_apply_with_patch_stdin` 的复用。

## 3. App-server 接入

- [x] `subagents_runtime_artifacts.rs` 改为调用 runtime auto-apply API。
- [x] 保持 `isolated_write_auto_apply` 字段结构兼容。
- [x] 保持 recovery 命令组装行为兼容。

## 4. 验证

- [x] `cargo check --workspace`
- [x] `cargo test -p omne-thread-git-snapshot-runtime`
- [x] `cargo test -p omne-app-server fan_out_result_writer_auto_apply`
- [x] `cargo test -p omne-app-server fan_out_result_writer_auto_applies`
