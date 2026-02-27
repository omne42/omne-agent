# 提案：git-domain-e2e-handoff-hardening

## 相关文档

- `openspec/changes/git-domain-sequence.md`：第 6 阶段收口要求。
- `openspec/specs/git-domain/implementation-roadmap.md`：全链路目标与最终验收。
- `openspec/specs/git-domain/spec.md`：跨阶段边界约束。
- `openspec/specs/git-domain/handoff-template.md`：阶段交接模板（离开前必填）。
- `openspec/changes/git-domain-worktree-lifecycle/proposal.md`：生命周期回收行为基线。
- `openspec/changes/git-domain-worktree-policy-observability/proposal.md`：策略与观测字段基线。

## 做什么

- 建立 Git 领域端到端回归套件，覆盖从 subagent spawn 到 thread 终态回收的完整链路。
- 固化“任意时刻可交接”文档模板与离开前检查清单。
- 形成发布前统一验收门禁：行为 + 边界 + 文档可接力。

## 为什么做

- 前 1-5 阶段已形成多点能力，但缺少统一 E2E 收口，跨阶段回归风险高。
- 没有标准化交接模板，人员切换时容易丢失关键上下文。
- 最终目标是“长期可维护的 Git 领域”，不仅是一次性功能可用。

## 怎么做

- 增加 E2E 主链路：
  - `subagent isolated_write`
  - `worktree/copy backend`
  - `patch handoff`
  - `auto-apply`
  - `thread/archive` + `thread/delete` cleanup
- 增加策略分支 E2E：
  - `auto` 成功/回退
  - `worktree` 强制失败
  - `copy` 强制执行
- 固化交接模板字段：
  - 当前阶段状态
  - 下一步入口
  - 阻塞点
  - 可复跑命令

## 非目标

- 不在本阶段扩展新的业务功能。
- 不在本阶段升级或重构协议版本。
- 不在本阶段引入新的运行时后端。

## 验收标准

- 行为正确性：
  - E2E 套件可重复通过，覆盖主链路与关键分支。
- 架构边界：
  - `app-server` 不新增 Git 实现回流，runtime 分层保持稳定。
- 交接可用性：
  - 新接手者仅依赖文档即可复跑与继续推进，无“下一步不明确”断点。
