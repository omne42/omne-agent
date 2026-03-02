# OmneAgent 的 OpenSpec

本目录采用 OpenSpec 风格的双轨结构：

- `specs/`：当前已生效的事实规格（source of truth）。
- `changes/`：尚未完全采纳的变更提案。

工作流程：

1. 创建 `changes/<change-id>/proposal.md`，说明动机与范围。
2. 创建 `changes/<change-id>/tasks.md`，给出可执行任务清单。
3. 在 `changes/<change-id>/specs/...` 下补充规格增量（spec delta）。
4. 按任务实现并完成验证。
5. 验收通过后把增量合并到 `specs/`，并归档该变更。

提案文档约束（`changes/*/proposal.md`）：

- 必须显式包含四段：`做什么`、`为什么做`、`怎么做`、`验收标准`。
- `怎么做` 要写清实现路径（涉及哪些 crate/模块、如何迁移边界）。
- 若有边界控制，补 `非目标`，防止范围漂移。

当前重点：

- `git-domain`：在 `omne-agent` 内定义并逐步下沉专属 Git runtime 领域归属，不迁移到 `safe-fs-tools`。
- `tool-surface`：收敛默认模型工具面到聚合入口（`<=5`），采用 help-first 渐进披露，并明确在 `main` 上持续小步开发；后续推进 role/mode 正交与策略加固。
