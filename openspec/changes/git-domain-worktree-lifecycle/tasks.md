# 任务：git-domain-worktree-lifecycle

## 相关文档与用途

- `openspec/changes/git-domain-worktree-lifecycle/proposal.md`：本阶段的做什么/为什么做/怎么做/验收标准。
- `openspec/changes/git-domain-worktree-lifecycle/specs/git-domain/spec.md`：本阶段新增规范约束。
- `openspec/changes/git-domain-sequence.md`：全链路顺序（本阶段必须在 worktree-default 之后）。
- `openspec/specs/git-domain/spec.md`：Git 领域总体边界，防止职责回流到 app-server。
- `docs/rts_workflow.md`：runtime 下沉原则与服务层职责边界。
- `/root/autodl-tmp/zjj/p/wsl-docs/00-元语/git-worktree.md`：worktree 生命周期维护实践。

## 1. 文档与规格

- [x] 补齐 lifecycle proposal，写明做什么/为什么做/怎么做/验收标准。
- [x] 增加 lifecycle spec delta。
- [x] 在任务中写明“app-server 不承载 Git 实现”的边界检查。

## 2. Runtime 实现

- [ ] 在 `omne-thread-git-snapshot-runtime` 增加 worktree 生命周期 API：
- [ ] 识别 detached linked worktree（避免误删普通目录）。
- [ ] 执行 `worktree remove --force`。
- [ ] 执行 `worktree prune` 清理元数据。
- [ ] 补齐 runtime 单测（受管 worktree 成功、非 worktree 不误删）。

## 3. App-server 接入

- [ ] 在 `thread/archive` 路径调用 runtime 生命周期 API（best-effort，不阻断主流程）。
- [ ] 在 `thread/delete` 路径调用 runtime 生命周期 API（best-effort，不阻断主流程）。
- [ ] app-server 仅保留路径判定和结果映射，不新增 git 命令实现。

## 4. 验证

- [ ] `cargo fmt --all --check`
- [ ] `cargo test -p omne-thread-git-snapshot-runtime`
- [ ] `cargo test -p omne-app-server thread_archive_`
- [ ] `cargo test -p omne-app-server thread_delete_`
- [ ] `rg -n "Command::new\\(\\\"git\\\"\\)" crates/app-server/src/main/thread_manage`（应无新增命中）

## 5. 下一阶段衔接（第 5 阶段）

- [ ] 进入 `git-domain-worktree-policy-observability`：
- [ ] 增加 backend 策略开关（`worktree|copy|auto`）。
- [ ] 增加 fallback reason / backend 字段，提升线上可诊断性。

## 6. 交接检查清单（离开前必须满足）

- [ ] 文档已同步：proposal + tasks + spec delta。
- [ ] 本阶段代码与测试结果已回写到本文件（勾选状态与命令结果一致）。
- [ ] 明确下一步入口文件与命令，下一位无需聊天上下文可继续推进。
