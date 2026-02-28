# 任务：git-domain-runtime-extraction

## 1. 规格与计划

- [x] 创建 OpenSpec 目录骨架（`specs/` + `changes/`）。
- [x] 编写 git-domain 基线规格与本次提案/任务文档。

## 2. Runtime 下沉

- [x] 将隔离工作区 patch 抓取（`git diff --binary --patch`）下沉到 `omne-git-runtime`。
- [x] 将 patch 的 stdin 应用（`git apply --check` / `git apply`）下沉到 `omne-git-runtime`。
- [x] 为 runtime crate 补充 clean/dirty 抓取与 apply 行为测试。

## 3. App-server 集成

- [x] 在 `subagents_runtime_artifacts.rs` 中以 runtime API 替换内联 Git 子进程实现。
- [x] 保持现有 failure-stage 与 recovery-hint 行为不变。

## 4. 验证

- [x] `cargo check --workspace`
- [x] `cargo test -p omne-git-runtime`
- [x] `cargo test -p omne-app-server fan_out_result_writer_auto_applies`
- [x] `cargo test -p omne-app-server fan_out_result_writer_auto_apply`
