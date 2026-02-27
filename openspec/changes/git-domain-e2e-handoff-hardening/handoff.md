# Git 领域 Phase 6 交接卡（git-domain-e2e-handoff-hardening）

## 1. 当前阶段状态

- 阶段：`git-domain-e2e-handoff-hardening`
- 目标：补齐 Git 领域主链路 E2E、异常 E2E、交接模板与验收门禁
- 当前状态：`已完成`
- 对应提交：
- `dfb4f9c openspec: add git-domain handoff template and phase6 concrete checks`

## 2. 本阶段已完成

- 新增主链路 E2E：`isolated_write -> handoff -> auto-apply -> archive cleanup`
- 新增主链路 E2E：`isolated_write -> handoff -> auto-apply -> delete cleanup`
- 新增异常场景 E2E：cleanup best-effort 失败不阻断（archive/delete）
- E2E 中新增统一结构化摘要字段（`phase/chain/backend/auto_apply_applied`）
- 固化并落地阶段交接模板：`openspec/specs/git-domain/handoff-template.md`

## 3. 下一步（必须可直接执行）

1. 复核 phase 6 改动并确认进入主分支
   - 入口文件：`openspec/changes/git-domain-e2e-handoff-hardening/tasks.md`
   - 命令：`git log --oneline -n 10`
2. 执行主分支合并与最终回归
   - 入口文件：`crates/app-server/src/agent/tools/dispatch/subagents_agent_spawn_guard_tests.rs`
   - 命令：`cargo test -p omne-app-server fan_out_result_writer_ -- --nocapture`
3. 合并后边界复核
   - 入口文件：`crates/app-server/src/main`
   - 命令：`rg -n "Command::new\\(\"git\"\\)" crates/app-server/src/main crates/app-server/src/agent/tools/dispatch`

## 4. 阻塞与风险

- 阻塞：无
- 风险：当前仓库存在大量非本阶段改动（已暂存/未暂存混合）；合并主分支时必须使用路径级提交与差异复核，避免误带无关改动。
- 回退策略：若合并验证失败，回退到本阶段提交并按测试命令逐项复跑定位。

## 5. 最近通过的验证命令

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test -p omne-app-server isolated_workspace_ -- --nocapture`
- `cargo test -p omne-app-server fan_out_result_writer_ -- --nocapture`
- `cargo test -p omne-app-server thread_archive_cleans_managed_detached_worktree -- --nocapture`
- `cargo test -p omne-app-server thread_delete_cleans_managed_detached_worktree -- --nocapture`
- `cargo test -p omne-app-server thread_archive_ignores_cleanup_errors_for_managed_broken_worktree -- --nocapture`
- `cargo test -p omne-app-server thread_delete_ignores_cleanup_errors_for_managed_broken_worktree -- --nocapture`
- `rg -n "Command::new\\(\"git\"\\)" crates/app-server/src/main crates/app-server/src/agent/tools/dispatch`

## 6. 架构边界核对

- `app-server` 新增/修改内容仅限：测试与编排链路验证
- Git 过程实现是否在 runtime：是
- 是否发现边界回流：否（扫描结果为测试文件命中）

## 7. 相关文档

- `openspec/changes/git-domain-sequence.md`
- `openspec/specs/git-domain/spec.md`
- `openspec/specs/git-domain/implementation-roadmap.md`
- `openspec/specs/git-domain/handoff-template.md`
- `openspec/changes/git-domain-e2e-handoff-hardening/tasks.md`
