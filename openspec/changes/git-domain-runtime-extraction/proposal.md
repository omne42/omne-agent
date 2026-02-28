# 提案：Git 领域 Runtime 下沉（第二阶段）

## 背景

`thread/diff` 与 `thread/patch` 已经开始下沉到 `omne-git-runtime`，
但隔离子代理工作区的 patch 抓取与应用逻辑仍在 app-server 的 dispatch 代码中。

这会让 Git 命令细节分散在多个层级，导致领域边界不清晰。

## 目标

- 保持 Git 领域归属在 `omne-agent` 内（不迁移到 `safe-fs-tools`）。
- 延续 OpenSpec 风格的增量下沉方式。
- 将子代理隔离工作区的 Git patch 原语下沉到 runtime crate。
- 保持 app-server 现有行为与协议载荷语义不变。

## 非目标

- 不对 fan-out result 的 payload schema 做大规模重构。
- 不把完整的 subagent 编排逻辑迁出 app-server。

## 范围

- Runtime crate：补充可复用的 patch 抓取/应用原语。
- App-server：改为调用 runtime 原语，移除内联 Git 子进程实现。
- 测试：补充 runtime 单测，并保持 app-server 集成测试通过。

## 验收标准

- `cargo check --workspace` 通过。
- `cargo test -p omne-git-runtime` 通过。
- `cargo test -p omne-app-server subagents_agent_spawn_guard_tests` 中相关自动应用场景通过。
