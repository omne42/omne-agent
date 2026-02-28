# Git 领域全链路实施路线（最终目标导向）

## 最终目标

在 `omne-agent` 内形成可独立演进的专属 Git 领域：

- 默认以 `git worktree` 承载 `isolated_write` 隔离工作区。
- `app-server` 仅承担编排与协议映射，不承载 Git 过程实现细节。
- Git 关键能力（snapshot、patch、auto-apply、worktree lifecycle）集中到 runtime。
- Git 核心主链路优先由 `gix` 承担，逐步移除对系统 `git` CLI 的硬依赖。
- 对外行为兼容、可回归、可交接。

## 全局边界（硬性）

- 目标终态中，`omne-agent/crates/app-server` 不实现 Git 领域过程逻辑。
- `app-server` 允许存在的仅有：编排、策略选择、协议字段映射、诊断信息组装。
- Git 领域实现（worktree、snapshot、apply、lifecycle）统一下沉到 Git runtime crate。
- 不启动本地 Git 服务（无 daemon/smart-http 依赖）。
- 对尚未迁移到 `gix` 的能力允许 runtime 内受控 fallback，但禁止在 app-server 旁路实现。

## 阶段路线（全链路）

### Phase 0：Git Snapshot 基线（已完成）

- 做什么：`thread/diff`、`thread/patch` 的 recipe/limits 下沉到 runtime。
- 为什么做：先收敛最基础、最稳定的 Git 调用边界。
- 怎么做：`disk_git_diff` 改为 runtime recipe + 限额。
- 验收标准：`thread_diff_tests` 通过，协议兼容。

### Phase 1：Auto-Apply 状态机下沉（进行中，代码已落地）

- 做什么：`git apply --check/apply` 相关状态机由 runtime 承担。
- 为什么做：避免状态判定逻辑散落在 app-server。
- 怎么做：runtime 输出失败阶段/原因，app-server 负责 JSON 映射。
- 验收标准：auto-apply 成功、冲突、无 patch 场景回归通过。

### Phase 2：Worktree 默认后端（进行中，当前主线）

- 做什么：`prepare_isolated_workspace` 默认走 `git worktree add --detach`。
- 为什么做：对齐 Claude Code 并行 worktree 实践，降低复制成本。
- 怎么做：worktree first；失败自动 copy fallback；不中断子任务；不引入本地 Git 服务进程。
- 验收标准：
  - Git 仓库场景默认 worktree；
  - 非 Git 场景自动回退 copy；
  - 现有 handoff/auto-apply 流程保持兼容。

### Phase 3：Worktree 生命周期（下一阶段）

- 做什么：补 `remove/prune/lock` 与 thread archive/cleanup 联动。
- 为什么做：没有生命周期就会留下悬挂 worktree 与脏元数据。
- 怎么做：runtime 新增 lifecycle API；app-server 在 thread 终态触发回收。
- 验收标准：长时间并发运行后无 worktree 泄漏，`git worktree list` 可收敛。

### Phase 4：策略与可观测性（下一阶段）

- 做什么：增加后端策略与结果可观测字段。
- 为什么做：便于灰度、排障、环境差异处理。
- 怎么做：
  - 策略开关：`worktree|copy|auto`；
  - 结果字段：`workspace_backend`、fallback 原因。
- 验收标准：日志、artifact、协议均可识别后端与降级路径。

### Phase 5：端到端收口与文档固化（最终收口）

- 做什么：补全 E2E 回归、失败恢复手册、交接模板。
- 为什么做：确保“任何时刻可交接、可持续推进”。
- 怎么做：
  - E2E：并发子任务 + worktree + auto-apply + archive 回收；
  - 文档：每阶段都包含做什么/为什么/怎么做/验收标准与相关文档。
- 验收标准：
  - 全链路命令一键回归通过；
  - 新接手者可按文档独立推进，不依赖聊天上下文。
  - 边界检查通过：`rg -n \"Command::new\\(\\\"git\\\"\\)\" crates/app-server/src` 仅允许出现在明确标注为过渡期的白名单位置，最终白名单收敛到 0。

### Phase 6：Gix Backend Foundation（新增）

- 做什么：在 `omne-git-runtime` 建立 `gix` 后端抽象，并迁移首批主链路能力。
- 为什么做：让核心 Git 能力不依赖系统 `git` 预装，提升单文件发行可用性。
- 怎么做：
  - 新增后端选择：`gix|cli`；
  - 优先迁移 `fetch/pull` 与仓库基础读能力；
  - 对未迁移路径保留 runtime 内 fallback（非 app-server）。
- 验收标准：
  - runtime 可配置 `gix` 后端并通过单测；
  - `fetch/pull` 能力有可执行验证；
  - `app-server` 无新增 Git 过程实现。

## 依赖与参考

- 仓库内：
  - `openspec/specs/git-domain/spec.md`
  - `docs/rts_workflow.md`
  - `docs/v0.2.0_parity.md`
  - `docs/implementation_plan.md`
- 外部：
  - Claude Code worktree 并行会话：
    https://code.claude.com/docs/en/tutorials#run-parallel-claude-code-sessions-with-git-worktrees
  - Git 官方 worktree：
    https://git-scm.com/docs/git-worktree

## 交接协议（必须执行）

每次离开前必须更新：

1. 当前所处 Phase 与完成度。
2. 下一步 1-3 个可直接执行任务（含命令与入口文件）。
3. 当前阻塞与回退方案。
4. 最近一次通过的验证命令清单。
