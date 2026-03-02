# 任务：tool-surface-consolidation

## 相关文档与用途

- `openspec/changes/tool-surface-consolidation/proposal.md`：目标、边界、验收标准。
- `openspec/changes/tool-surface-consolidation/specs/tool-surface/spec.md`：本次规格增量。
- `openspec/changes/tool-surface-consolidation/references.md`：代码与文档证据索引。
- `openspec/changes/tool-surface-sequence.md`：主题推进顺序与阶段衔接。

## 1. 文档与规格

- [x] 补齐 proposal（做什么/为什么做/怎么做/验收标准/非目标）。
- [x] 补齐 references（内部事实、对比资料、关键约束）。
- [x] 补齐 spec delta（默认工具数、help 披露、动作映射约束）。

## 2. Facade 契约定义

- [x] 在协议层新增 facade 请求/响应结构（`workspace/process/thread/artifact/integration`）。
- [x] 定义统一字段：`op`、`args`、`help/topic`。
- [x] 定义稳定错误码映射（参数错误、动作不支持、策略拒绝）。

## 3. Tool 暴露与路由

- [x] 在 catalog 中增加 facade tool schema。
- [x] 默认仅暴露 facade tools（legacy tools 默认隐藏）。
- [x] 增加开关：`OMNE_TOOL_FACADE_ENABLED` / `OMNE_TOOL_FACADE_EXPOSE_LEGACY`。
- [x] 在 dispatch 层实现 `facade op -> internal action` 路由。

## 4. 权限与审计对齐

- [x] 映射后动作继续走 `allowed_tools` 校验（硬拒绝链路不变）。
- [x] 映射后动作继续走 mode/sandbox/execpolicy/approval。
- [x] ToolStarted/ToolCompleted 增加可追踪字段：`facade_tool`、`op`、`mapped_action`。

## 5. 渐进式帮助

- [x] 每个 facade 支持 `op=help`。
- [x] `op=help` 返回 quickstart 与 advanced 两层。
- [x] REPL `/help` 增加“默认最简 + 如何进阶到 help”的说明。

## 6. 观测与回归

- [x] 记录并对比 facade 上线前后 `tool_count/tool_schema_bytes`。
- [x] 增加测试：默认暴露工具数断言（`<= 5`）。
- [x] 增加测试：关键拒绝路径在 facade 下保持一致。
- [x] 增加测试：`op=help` 的返回结构稳定。

## 7. Mainline 开发执行（强制）

- [x] 在 OpenSpec 文档中明确“直接在 `main` 持续开发”。
- [ ] 后续实现提交按小步迭代进入 `main`，每次提交附最小验证命令。
- [ ] 禁止为该主题维护长期 feature 分支。

## 8. 推荐验证命令（实现期执行）

- [x] `cargo fmt --all --check`
- [x] `cargo check --workspace --all-targets`
- [x] `cargo test -p omne-app-server`
- [x] `cargo test -p omne`
- [x] `rg -n "tool_count|tool_schema_bytes" crates/app-server/src/agent/core/run_turn.rs`

## 9. 交接检查清单

- [x] proposal/tasks/spec/references 四件套均已更新。
- [x] 当前阶段完成度、下一步 1-3 条可执行动作已写清。
- [x] 回滚开关状态与默认值已写清。

## 10. 对齐缺口（后续阶段入口）

- [x] facade 拒绝路径矩阵测试补齐（mode/execpolicy/approval 至少各 1 条）。
- [x] role/mode 正交化方案落盘（role catalog 独立、叠加规则显式化）。
- [x] `thread` facade 子代理生命周期操作补齐（`send_input/wait/close`）。
- [x] 动态工具注册方案评审（本地/插件工具）与开关策略确定。
- [x] CLI/TUI 显示层增加 facade 映射摘要（`facade_tool/op/mapped_action`）。

### 当前阶段完成度（2026-03-02）

- Facade 契约、暴露、路由、权限与审计、help-first、关键回归均已落地并验证通过。
- 默认工具面已收敛为 `workspace/process/thread/artifact`（`integration` 按能力可选）。
- 实测 schema 开销从 `legacy_count=21/bytes=7669` 收敛到 `facade_count=4/bytes=1815`。
- CLI/TUI 文本层已显示 facade 映射摘要（`facade_tool/op/mapped_action`），便于审计与排障。
- role/mode 已完成正交收口：执行链使用 mode，tool 暴露与 explain 同步 role(permission_mode) 叠加。
- 动态工具注册 MVP 已落地（默认关闭，read-only 映射，支持本地 registry 文件加载与审计）。

### 下一步 1-3 条可执行动作

1. 为动态工具注册补文档化示例（registry 文件模板 + 常见错误）。
2. 增加动态工具调用的拒绝路径矩阵测试（approval/execpolicy/mode 组合）。
3. 评估 `tool-surface-legacy-deprecation` 阶段的迁移窗口与回滚 SOP。

### 回滚开关状态（默认值）

- `OMNE_TOOL_FACADE_ENABLED=true`
- `OMNE_TOOL_FACADE_EXPOSE_LEGACY=false`
