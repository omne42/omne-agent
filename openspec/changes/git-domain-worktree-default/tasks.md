# 任务：git-domain-worktree-default

## 相关文档与用途

- `openspec/changes/git-domain-worktree-default/proposal.md`：说明做什么、为什么做、怎么做与验收标准。
- `openspec/changes/git-domain-worktree-default/specs/git-domain/spec.md`：定义本阶段新增/变更要求。
- `openspec/changes/git-domain-sequence.md`：全链路顺序定义（必须按序推进）。
- `openspec/specs/git-domain/spec.md`：Git 领域基线规范，用于判定是否偏离目标。
- `openspec/specs/git-domain/implementation-roadmap.md`：最终目标导向的阶段路线总览。
- `docs/rts_workflow.md`、`docs/v0.2.0_parity.md`、`docs/implementation_plan.md`：workspace/worktree 方向与边界参考。
- `/root/autodl-tmp/zjj/p/wsl-docs/00-元语/git-worktree.md`：worktree 实践边界（目录规划、维护清理）。

## 1. 文档与规格

- [x] 编写 proposal。
- [x] 编写 spec delta。
- [x] 在任务中显式约束“worktree first, copy fallback”。

## 2. Runtime 实现

- [x] 新增 worktree 创建 API（基于 `git worktree add --detach`）。
- [x] 返回可诊断错误，供 app-server 判定 fallback。
- [x] 补充 runtime 单测（包含非仓库失败场景）。

## 3. App-server 接入

- [x] `prepare_isolated_workspace` 默认走 worktree。
- [x] 失败时自动回退 copy（保持旧能力）。
- [x] 保持现有 patch handoff 与 auto-apply 流程不变。

## 4. 验证

- [x] `cargo check --workspace`
- [x] `cargo test -p omne-thread-git-snapshot-runtime`
- [x] `cargo test -p omne-app-server isolated_workspace_`
- [x] `cargo test -p omne-app-server fan_out_result_writer_auto_applies`
- [x] `cargo test -p omne-app-server fan_out_result_writer_auto_apply`
- [x] 边界扫描：`rg -n \"Command::new\\(\\\"git\\\"\\)\" crates/app-server/src/agent/tools/dispatch/subagents_runtime_artifacts.rs`（应无输出）

## 5. 后续任务序列（下一位可直接执行）

- [ ] 新建变更：`git-domain-worktree-lifecycle`，实现 worktree `remove/prune` 与 thread archive 联动。
- [ ] 新建变更：`git-domain-worktree-observability`，在 `fan_out_result` 中补充 `workspace_backend=worktree|copy` 可观测字段。
- [ ] 新建变更：`git-domain-worktree-policy`，增加策略开关（禁用 worktree、强制 copy、失败重试次数）。
- [ ] 新建变更：`git-domain-worktree-recovery`，补充 worktree 冲突/脏状态恢复流程（reset/clean 边界明确）。

## 6. 交接检查清单（离开前必须满足）

- [x] 文档已包含：做什么、为什么做、怎么做、验收标准。
- [x] 相关文档已列出并说明用途（仓库内 + 外部依据）。
- [x] 关键入口文件已明确：
  - `crates/thread-git-snapshot-runtime/src/lib.rs`
  - `crates/app-server/src/agent/tools/dispatch/subagents_runtime_artifacts.rs`
  - `crates/app-server/src/agent/tools/dispatch/subagents_agent_spawn_guard_tests.rs`
- [x] 回归命令可直接执行且通过（见第 4 节）。
