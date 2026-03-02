# Tool Surface Execution Todo

更新时间：2026-03-02

## 状态约定

- `DONE`：已实现并验证
- `IN_PROGRESS`：进行中
- `TODO`：待开始

## 阶段 1：Facade 安全拒绝矩阵

- [x] `DONE` facade `mode_denied` 路径测试（至少 1 条）
- [x] `DONE` facade `execpolicy_denied` 路径测试（至少 1 条）
- [x] `DONE` facade `approval_denied` 路径测试（至少 1 条）
- [x] `DONE` 目标测试命令回归通过

## 阶段 2：Role/Mode 正交化

- [x] `DONE` role catalog 独立于 mode catalog（保留 mode-name 兼容回退）
- [x] `DONE` 去除 `role = mode` 隐式回退
- [x] `DONE` explain 输出 mode/role 叠加决策明细（新增 role catalog layer）

## 阶段 3：Thread Facade 生命周期能力

- [x] `DONE` 新增 `send_input/wait/close` facade op（含 `close_agent` 兼容别名）
- [x] `DONE` 映射到 internal action 并复用安全链（`subagent/send_input`、`subagent/wait`、`subagent/close`）
- [x] `DONE` 增补测试与 help 文档

## 阶段 3.5：CLI/TUI 映射摘要可视化

- [x] `DONE` `ask` 输出显示 `facade_tool/op/mapped_action`
- [x] `DONE` `event_render` 输出显示 `facade_tool/op/mapped_action`
- [x] `DONE` TUI transcript 显示映射摘要（含 denied/failed 状态）
- [x] `DONE` 目标测试命令回归通过（`ask_exec_tests` / `event_render_tests` / `tool_format_tests`）

## 阶段 4：动态工具注册（可选开关）

- [x] `DONE` 设计动态注册协议（MVP：本地 registry、read-only 映射）
- [x] `DONE` 加载、schema 转换与审计（catalog 暴露 + dispatch 映射 + wrapper 审计字段）
- [x] `DONE` 灰度开关与回滚策略（`OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED` + 路径覆盖开关）

## 阶段 4.5：动态注册回归

- [x] `DONE` `tool_catalog` 动态注册暴露/过滤测试通过
- [x] `DONE` 动态工具调度与参数校验测试通过（含 `dynamic_invalid_params`）
