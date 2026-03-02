# Omne Tooling Alignment Todo (vs Codex/OpenCode)

目标：在不降低安全链路的前提下，把 Omne 工具面收敛为“默认极小 + 按需披露 + 可开关扩展”，并对齐 Codex/OpenCode 的动态能力与可维护性。

## A. 当前完成（已落地）

- [x] A1 Facade 收敛：默认 model-facing 工具面收敛到 `workspace/process/thread/artifact`（可选 `integration`）。
- [x] A2 Help-first：聚合工具统一 `op=help`，返回 `quickstart + advanced`。
- [x] A3 路由复用：`facade op -> legacy action`，不改后端细粒度 handler。
- [x] A4 安全链路不降级：继续经过 `allowed_tools -> mode -> sandbox -> execpolicy -> approval`。
- [x] A5 观测：每轮输出 `tool_count` / `tool_schema_bytes`。
- [x] A6 成本收益实测：`legacy_bytes=7669` -> `facade_bytes=1815`（约 `76.33%` 降幅）。
- [x] A7 角色维度首版：`ToolRoleProfile`（`chatter/default/codder`）已接入工具暴露裁剪。
- [x] A8 CLI/TUI 可视化：`ask/event/tui` 已显示 `facade_tool/op/mapped_action` 摘要。

## B. 缺口清单（Omne 相比 Codex/OpenCode）

- [x] B1 子代理生命周期工具闭环已补齐：`send_input` / `wait` / `close_agent`（facade `thread` 下）。
- [x] B2 动态工具入口 MVP 已落地：本地 runtime registry（默认关，read-only 映射，fail-closed）。
- [x] B3 role/mode 正交执行已落地：运行时 gate 使用 mode，tool 暴露/配置解释使用 role(permission_mode) 叠加。
- [x] B4 facade 拒绝路径矩阵测试补齐（mode/execpolicy/approval 已分别覆盖）。
- [ ] B5 provider/model 裁剪策略较轻量，尚未达到 Codex/OpenCode 的细粒度能力分层。

## C. 开关策略（建议默认值）

以下为建议保留的默认开关策略：

- [x] `OMNE_TOOL_FACADE_ENABLED=true`：默认开（核心收敛能力）。
- [x] `OMNE_TOOL_FACADE_EXPOSE_LEGACY=false`：默认关（避免 schema 膨胀）。
- [x] `OMNE_ENABLE_MCP=false`：默认关（降低外部依赖与风险）。
- [x] `OMNE_TOOL_EXPOSE_WEB=false`：默认关（按需开启）。
- [x] `OMNE_TOOL_EXPOSE_SUBAGENT=false`：默认关（资源和治理成本高）。
- [x] `OMNE_TOOL_EXPOSE_THREAD_INTROSPECTION=false`：默认关（低频运维工具）。
- [x] `OMNE_TOOL_EXPOSE_THREAD_HOOK=false`：默认关（避免无意触发 hook）。
- [x] `OMNE_TOOL_EXPOSE_REPO_SYMBOLS=false`：默认关（成本较高，按需开启）。
- [x] `OMNE_TOOL_MODEL_PROFILE=auto`：默认自动（mini/flash/haiku -> compact）。

建议新增（部分已落地）：

- [ ] `OMNE_TOOL_EXPOSE_SUBAGENT_LIFECYCLE`：默认关，开启后暴露 `send_input/wait/close_agent`。
- [x] `OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED`：默认关，开启后支持本地动态工具注册（MVP）。
- [ ] `OMNE_ROLE_MODE_STRICT_SPLIT`：默认开（目标状态），强制 role 与 mode 独立校验与独立配置源。

## D. Role + Mode 联合治理（目标设计）

- [x] D1 协议层共存：`mode` 与 `role` 已同时存在于 thread config。
- [x] D2 配置源拆分：`mode_catalog` 与 `role_catalog` 分离（保留 mode-name 兼容回退）。
- [x] D3 叠加规则显式化：`effective_permissions = mode_policy ∩ role_policy ∩ allowed_tools`。
- [x] D4 去除隐式回退：已移除 `role = mode` 默认映射。
- [ ] D4.1 默认角色策略：是否切换为显式默认 `role=default`（当前默认仍为 `coder`）。
- [x] D5 explain 输出增强：已增加 `role_catalog` 层（effective_role/permission_mode/resolution_source）。

## E. 下一阶段实施任务（可直接推进）

### E1 Facade 安全回归补齐

- [x] 为 facade 增加 `mode_denied` 回归用例（至少 1 条）。
- [x] 为 facade 增加 `execpolicy_denied` 回归用例（至少 1 条）。
- [x] 为 facade 增加 `approval_denied` 回归用例（至少 1 条）。

### E2 子代理生命周期对齐

- [x] 设计 `thread` facade 下的 `send_input/wait/close` op 契约。
- [x] 对应映射到 internal action，复用既有安全链。
- [x] 补齐 catalog/spec/dispatch/tests 与 `/help` 文案。

### E3 动态工具能力（可选开关）

- [x] 设计动态工具注册协议（MVP：只读工具优先）。
- [x] 增加运行时加载与 schema 转换。
- [x] 增加动态工具审计字段（`dynamic_tool/mapped_tool/mapped_action`）。
- [ ] deny 回归矩阵补齐（approval/execpolicy/mode 组合）。

### E4 Role/Mode 正交化

- [x] 增加独立 `role_catalog` 与内建角色定义（`chatter/default/codder`）。
- [x] `thread/configure` role 校验切换到 `role_catalog`。
- [x] `thread_config_explain` 输出 role 定义来源与最终叠加结果。

## F. 验证与门禁

每个小步提交至少执行：

- [x] `cargo fmt --all --check`
- [x] `cargo check --workspace --all-targets`
- [x] `cargo test -p omne-app-server`
- [x] `cargo test -p omne`

针对当前主题建议附加：

- [x] `cargo test -p omne-app-server facade`（或具体 facade 测试过滤）
- [x] `cargo test -p omne-app-server tool_catalog_tests::facade_tool_surface_reduces_schema_bytes_vs_legacy_default -- --nocapture`

## G. 当前阶段结论

- Facade 收敛目标已达成，且 token/schema 开销收益明确。
- role/mode 正交主链已完成（保留 mode-name 兼容回退）。
- CLI/TUI 对 facade 映射的可观测性已补齐。
- 下一优先级建议：`E3 deny 矩阵 -> B5`。
