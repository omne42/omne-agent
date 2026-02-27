# 提案：git-domain-worktree-policy-observability

## 做什么

- 增加隔离后端策略开关：`worktree | copy | auto`。
- 增加可观测字段：实际使用后端、fallback 原因、关键失败阶段。

## 为什么做

不同仓库环境（权限、文件系统、Git 状态）对 worktree 支持程度不同。没有策略与可观测性，灰度和排障成本高。

## 怎么做

- 在 runtime/app-server 增加后端策略解析。
- 在结果 payload 与日志中回填 backend/fallback 诊断信息。
- 补回归：强制 worktree、强制 copy、auto fallback 三条路径。

## 非目标

- 不在本阶段改动核心调度算法。
- 不在本阶段引入新的远程存储或外部状态服务。

## 验收标准

- 三种策略路径均可稳定运行并可被测试断言。
- payload 中可明确识别 backend 与 fallback 触发原因。
- `cargo check --workspace` 与相关测试通过。
