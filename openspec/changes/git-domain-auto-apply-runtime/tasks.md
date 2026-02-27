# 任务：git-domain-auto-apply-runtime

## 1. 文档与约束

- [x] 编写本阶段 proposal。
- [ ] 编写 spec delta。
- [ ] 约束 app-server 仅保留编排与 payload 映射。

## 2. Runtime 实现

- [ ] 新增自动应用结果模型（成功/失败阶段/失败原因）。
- [ ] 新增自动应用入口 API（封装前置校验、抓取、check、apply）。
- [ ] 保留 `capture_workspace_patch` 与 `run_git_apply_with_patch_stdin` 的复用。

## 3. App-server 接入

- [ ] `subagents_runtime_artifacts.rs` 改为调用 runtime auto-apply API。
- [ ] 保持 `isolated_write_auto_apply` 字段结构兼容。
- [ ] 保持 recovery 命令组装行为兼容。

## 4. 验证

- [ ] `cargo check --workspace`
- [ ] `cargo test -p omne-thread-git-snapshot-runtime`
- [ ] `cargo test -p omne-app-server fan_out_result_writer_auto_apply`
- [ ] `cargo test -p omne-app-server fan_out_result_writer_auto_applies`
