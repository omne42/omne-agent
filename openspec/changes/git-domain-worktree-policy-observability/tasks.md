# 任务：git-domain-worktree-policy-observability

## 相关文档与用途

- `openspec/changes/git-domain-worktree-policy-observability/proposal.md`：阶段目标与验收标准。
- `openspec/changes/git-domain-worktree-policy-observability/specs/git-domain/spec.md`：策略与观测字段规范。
- `openspec/changes/git-domain-sequence.md`：执行顺序约束（第 5 阶段）。
- `openspec/specs/git-domain/spec.md`：Git 领域边界与职责划分。
- `openspec/changes/git-domain-worktree-default/proposal.md`：默认后端行为参考。
- `openspec/changes/git-domain-worktree-lifecycle/proposal.md`：生命周期联动前提能力。

## 1. 文档与规格

- [x] 补齐 policy/observability proposal（做什么/为什么做/怎么做/验收）。
- [x] 增加 spec delta，明确策略值与字段语义。
- [x] 在任务中写明边界检查（app-server 不直接执行 git 命令）。

## 2. 策略实现

- [x] 增加后端策略枚举：`auto | worktree | copy`。
- [x] 增加统一解析入口（默认 `auto`，非法值回退默认并记录）。
- [x] `worktree` 强制模式失败时返回错误，不自动 fallback。
- [x] `auto` 模式失败时 fallback 到 copy。
- [x] `copy` 模式直接执行 copy 路径。

## 3. 可观测字段

- [x] 在 `isolated_write` 结构化结果中写入 `requested_backend`。
- [x] 写入 `backend`（最终执行后端）。
- [x] `auto/worktree` 失败回退时写入 `fallback_reason`。
- [x] 失败字段语义与 auto-apply 现有字段保持一致（避免歧义）。

## 4. 验证

- [x] `cargo fmt --all --check`
- [x] `cargo test -p omne-app-server isolated_workspace_`
- [x] `cargo test -p omne-app-server fan_out_result_writer`
- [x] `rg -n "Command::new\\(\\\"git\\\"\\)" crates/app-server/src/agent/tools/dispatch/subagents_runtime_artifacts.rs || true`（应无输出）

## 5. 下一阶段衔接（第 6 阶段）

- [ ] 进入 `git-domain-e2e-handoff-hardening`：
- [ ] 建立全链路 E2E 套件（spawn -> isolated_write -> handoff -> auto-apply -> archive/delete cleanup）。
- [ ] 固化交接清单模板，确保“任意时刻可接力”。

## 6. 交接检查清单（离开前必须满足）

- [x] 文档已同步：proposal + tasks + spec delta。
- [x] 测试命令、边界扫描命令可直接复跑。
- [x] 下一阶段入口与阻塞点已写明。
