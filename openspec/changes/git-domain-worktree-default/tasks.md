# 任务：git-domain-worktree-default

## 相关文档与用途

- `openspec/changes/git-domain-worktree-default/proposal.md`：说明做什么、为什么做、怎么做与验收标准。
- `openspec/changes/git-domain-worktree-default/specs/git-domain/spec.md`：定义本阶段新增/变更要求。
- `openspec/specs/git-domain/spec.md`：Git 领域基线规范，用于判定是否偏离目标。
- `docs/rts_workflow.md`、`docs/v0.2.0_parity.md`、`docs/implementation_plan.md`：workspace/worktree 方向与边界参考。
- `/root/autodl-tmp/zjj/p/wsl-docs/00-元语/git-worktree.md`：worktree 实践边界（目录规划、维护清理）。

## 1. 文档与规格

- [x] 编写 proposal。
- [x] 编写 spec delta。
- [x] 在任务中显式约束“worktree first, copy fallback”。

## 2. Runtime 实现

- [ ] 新增 worktree 创建 API（基于 `git worktree add --detach`）。
- [ ] 返回可诊断错误，供 app-server 判定 fallback。
- [ ] 补充 runtime 单测（至少包含非仓库失败场景）。

## 3. App-server 接入

- [ ] `prepare_isolated_workspace` 默认走 worktree。
- [ ] 失败时自动回退 copy（保持旧能力）。
- [ ] 保持现有 patch handoff 与 auto-apply 流程不变。

## 4. 验证

- [ ] `cargo check --workspace`
- [ ] `cargo test -p omne-thread-git-snapshot-runtime`
- [ ] `cargo test -p omne-app-server fan_out_result_writer_auto_applies`
- [ ] `cargo test -p omne-app-server fan_out_result_writer_auto_apply`
