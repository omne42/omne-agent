# Git 领域变更顺序（强制执行顺序）

本文件定义“专属 Git 领域”全链路变更的执行顺序。必须按序推进；后序变更依赖前序产物。

## 顺序清单

1. `git-domain-runtime-extraction`
- 目标：抽离 `thread/diff`、`thread/patch` 基线能力到 runtime。
- 文档：`openspec/changes/git-domain-runtime-extraction/proposal.md`

2. `git-domain-auto-apply-runtime`
- 目标：抽离 auto-apply 状态机到 runtime，app-server 仅做映射。
- 文档：`openspec/changes/git-domain-auto-apply-runtime/proposal.md`

3. `git-domain-worktree-default`
- 目标：`isolated_write` 默认 worktree，失败 copy fallback。
- 文档：`openspec/changes/git-domain-worktree-default/proposal.md`

4. `git-domain-worktree-lifecycle`
- 目标：worktree `remove/prune/lock` 与 thread archive 联动。
- 文档：`openspec/changes/git-domain-worktree-lifecycle/proposal.md`

5. `git-domain-worktree-policy-observability`
- 目标：后端策略开关与可观测字段（backend/fallback reason）。
- 文档：`openspec/changes/git-domain-worktree-policy-observability/proposal.md`

6. `git-domain-e2e-handoff-hardening`
- 目标：E2E 收口与交接标准固化，确保可持续接力。
- 文档：`openspec/changes/git-domain-e2e-handoff-hardening/proposal.md`

## 交接要求

- 每个变更必须包含：`做什么`、`为什么做`、`怎么做`、`验收标准`。
- 每个变更推进时都要更新自身 `tasks.md`，并记录下一步可执行动作。
