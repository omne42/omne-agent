# omne-agent Docs System

## 入口分工

- `README.md`
  - 对外概览与最小验证入口。
- `AGENTS.md`
  - 当前执行者指南与仓库地图，但不是完整规范手册。
- `docs/README.md`
  - 文档导航入口。
- `docs/start.md`
  - 当前高层起点与目标/范围说明。
- `docs/`
  - 版本化事实来源。

## 目录职责

- `docs/omne_data.md`、`runtime_layout.md`、`thread_event_model.md`
  - 运行时数据、目录布局和事件模型。
- `docs/modes.md`、`approvals.md`、`permissions_matrix.md`
  - 模式、审批与权限边界。
- `docs/model_routing.md`、`mcp.md`、`execpolicy.md`、`budgets.md`
  - 核心能力专题文档。
- `docs/reports/`
  - 对齐审计和问题复盘；有时间点语义，不是主规范来源。
- `docs/research/`
  - 外部产品/仓库调研；输入设计，不直接定义实现口径。

## 新鲜度规则

- 运行时或协议行为变化时，更新对应专题文档和 `docs/README.md` 导航。
- 文档入口结构变化时，同时更新本文件。
- 如果报告修正了过时认知，应把结论回写到专题文档，而不是把事实长期留在 `docs/reports/`。
- `AGENTS.md` 不应继续膨胀成完整手册。
- `scripts/check-docs-system.sh` 机械检查根入口与关键专题入口是否仍然存在。
- `scripts/check-docs-system.sh` 同时约束 `AGENTS.md` 保持短地图形态。
