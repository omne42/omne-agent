# 提案：git-domain-worktree-policy-observability

## 相关文档

- `openspec/changes/git-domain-sequence.md`：本变更为第 5 阶段，依赖 lifecycle 阶段产物。
- `openspec/specs/git-domain/spec.md`：Git 领域边界基线。
- `openspec/specs/git-domain/implementation-roadmap.md`：全链路目标与阶段衔接。
- `openspec/changes/git-domain-worktree-default/proposal.md`：默认 worktree/copy fallback 的前置行为。
- `openspec/changes/git-domain-worktree-lifecycle/proposal.md`：生命周期回收的前置能力。

## 做什么

- 增加隔离后端策略开关：`worktree | copy | auto`（保留 `auto` 为默认）。
- 增加可观测字段：实际后端、fallback 原因、策略来源。
- 将策略解析与决策结果写入结构化结果，便于回放与排障。

## 为什么做

- 不同仓库环境（权限、文件系统、Git 状态）对 worktree 支持差异大。
- 没有显式策略时，灰度和问题定位成本高；只有“成功/失败”不足以支撑运维。
- 可观测性应成为 Git 领域的默认能力，避免故障时只能靠日志猜测路径。

## 怎么做

- 在 runtime/app-server 增加统一策略解析（默认 `auto`）。
- 在 `isolated_write` 路径回填结构化字段：
  - `backend`：`worktree` 或 `copy`
  - `requested_backend`：策略请求值
  - `fallback_reason`：仅在 `auto/worktree` 回退时存在
- 增加测试覆盖：强制 worktree、强制 copy、auto fallback 三条路径。

## 非目标

- 不在本阶段改动核心调度算法与任务编排策略。
- 不在本阶段引入远程状态服务或集中式配置中心。
- 不在本阶段改 fan-in/fan-out 聚合协议版本。

## 验收标准

- 行为正确性：
  - 三种策略路径可稳定运行并可被测试断言。
  - `worktree` 强制模式在失败时返回明确失败，不静默回退。
  - `auto` 模式在失败时自动回退 copy 并记录原因。
- 架构边界：
  - Git 执行动作仍归属 runtime；app-server 只做策略解析、调度编排、结果映射。
- 可验证检查：
  - `cargo test -p omne-app-server isolated_workspace_`
  - `cargo test -p omne-app-server fan_out_result_writer`
  - `rg -n "Command::new\\(\\\"git\\\"\\)" crates/app-server/src/agent/tools/dispatch/subagents_runtime_artifacts.rs`（应无新增命中）
