# Tool Surface 变更顺序（强制执行顺序）

本文件定义“聚合工具 + 渐进披露”主题的推进顺序。后序阶段依赖前序产物。

## 顺序清单

1. `tool-surface-consolidation`
- 目标：建立 facade tools、help-first 披露与基础映射约束。
- 文档：`openspec/changes/tool-surface-consolidation/proposal.md`

2. `tool-surface-policy-hardening`（后续）
- 目标：补齐 facade->action 级审计字段、错误码一致性、拒绝路径回归测试。

3. `tool-surface-role-mode-split`（后续）
- 目标：把 `role` 与 `mode` 从共享 catalog 拆分为正交维度，并固化叠加规则。

4. `tool-surface-powershell-parity`（后续）
- 目标：补齐 PowerShell 侧命令拦截/策略一致性，避免仅 bash 路径完备。

5. `tool-surface-legacy-deprecation`（后续）
- 目标：在兼容窗口结束后逐步移除默认 legacy 暴露，并固化回滚窗口策略。

## 开发策略（Mainline）

- 本主题后续所有实现直接在 `main` 上持续开发。
- 禁止为该主题维护长期 feature 分支。
- 每次提交必须包含可复跑验证命令与文档同步记录。

## 交接要求

- 每个阶段离开前必须更新：
  - 当前完成度；
  - 下一步 1-3 条可执行动作；
  - 风险与回滚开关状态；
  - 最近一次通过的验证命令。
