# 任务：git-domain-e2e-handoff-hardening

## 相关文档与用途

- `openspec/changes/git-domain-e2e-handoff-hardening/proposal.md`：阶段目标与验收标准。
- `openspec/changes/git-domain-e2e-handoff-hardening/specs/git-domain/spec.md`：E2E 与交接规范条款。
- `openspec/changes/git-domain-sequence.md`：阶段顺序约束（第 6 阶段，最终收口）。
- `openspec/specs/git-domain/implementation-roadmap.md`：全链路里程碑与完成定义。
- `openspec/changes/git-domain-worktree-lifecycle/tasks.md`：生命周期回收验证前提。
- `openspec/changes/git-domain-worktree-policy-observability/tasks.md`：策略与观测验证前提。
- `openspec/specs/git-domain/handoff-template.md`：阶段交接模板与必填字段。

## 1. 文档与规范

- [x] 完善 proposal（做什么/为什么做/怎么做/验收）。
- [x] 增加 spec delta（E2E 覆盖范围与交接条款）。
- [x] 在任务中明确“离开时必须可交接”的检查项。

## 2. E2E 用例实现

- [x] 主链路 E2E：`isolated_write -> handoff -> auto-apply -> archive/delete cleanup`。
- [x] 策略分支 E2E：`auto` 成功、`auto` 回退、`worktree` 强制失败、`copy` 强制执行。
- [x] 异常场景 E2E：patch conflict、无 patch、cleanup best-effort 失败。
- [x] E2E 结果输出统一结构化摘要，便于回归对比。

## 3. 交接模板固化

- [x] 新增/更新交接模板文档（当前阶段、下一步、阻塞点、复跑命令）。
- [x] 每次阶段结束自动回写“已完成/待办/风险”。
- [x] 模板字段与 openspec 任务清单保持一一对应。

## 4. 验证

- [x] `cargo fmt --all --check`
- [x] `cargo check --workspace`
- [x] 目标 E2E 套件命令：
  - [x] `cargo test -p omne-app-server isolated_workspace_ -- --nocapture`
  - [x] `cargo test -p omne-app-server fan_out_result_writer_ -- --nocapture`
  - [x] `cargo test -p omne-app-server thread_archive_cleans_managed_detached_worktree -- --nocapture`
  - [x] `cargo test -p omne-app-server thread_delete_cleans_managed_detached_worktree -- --nocapture`
- [x] 边界扫描：`rg -n "Command::new\\(\\\"git\\\"\\)" crates/app-server/src/main crates/app-server/src/agent/tools/dispatch`（人工复核只允许 runtime 路径）

## 5. 完成定义（DoD）

- [x] E2E 套件可稳定复跑并覆盖主链路 + 关键分支。
- [x] 交接模板完整，下一位无需聊天上下文可直接继续。
- [x] Git 领域边界无回流，文档与代码状态一致。
