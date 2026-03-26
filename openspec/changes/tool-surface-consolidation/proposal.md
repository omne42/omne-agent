# 提案：tool-surface-consolidation

## 相关文档

- `openspec/README.md`：OpenSpec 双轨流程与 proposal 结构约束。
- `openspec/changes/tool-surface-sequence.md`：本主题的强制推进顺序。
- `docs/research/tools-alignment-todo.md`：现有工具暴露裁剪与 token 成本治理背景。
- `docs/research/tools-omne.md`：Omne 当前工具暴露/分发现实口径。
- `docs/research/tools-codex.md`：Codex 小默认工具集与动态暴露对照。
- `docs/research/tools-opencode.md`：OpenCode registry/filter/permission 对照。
- `docs/tool_parallelism.md`：只读工具并发边界。
- `docs/execpolicy.md`：命令执行策略与前缀规则约束。

## 做什么

- 将默认 model-facing 工具从当前 30+ 收敛到不超过 5 个聚合工具（facade tools）：
  - `workspace`
  - `process`
  - `thread`
  - `artifact`
  - `integration`（默认隐藏）
- 每个聚合工具默认只暴露“最小可用法”；进阶参数通过统一 `help` 子操作渐进披露。
- 保留后端细粒度动作与现有安全链路（`mode -> sandbox -> execpolicy -> approval`）不变。
- 明确研发推进策略：该主题后续改动直接在 `main` 上小步迭代，不新开长期特性分支。

## 为什么做

- 当前工具面过大，模型侧 schema 体积和 token 开销高，且已存在明确治理目标“减少无效 tool token”。
- 大量并列工具会增加模型选工具和参数匹配负担，导致误调用与往返轮次增加。
- “默认最简单 + help 深挖”更符合 CLI 直觉，可降低新手门槛，同时保留专家能力。
- 前台聚合、后台不动可以在不降低安全性的前提下优化体验与成本。

## 怎么做

### 1) 前台聚合，后台复用

- 新增聚合工具 schema（facade），由聚合入口解析 `op` 并路由到既有 handler。
- 既有细粒度 action 不删除，继续作为内部执行面和策略校验面。
- 默认仅向模型暴露聚合工具；原工具作为兼容层可通过开关临时回退。

### 2) 渐进式披露（help-first）

- 统一请求形态：`{"op":"..."}`。
- 统一帮助入口：`{"op":"help"}` 或 `{"op":"help", "topic":"read"}`。
- `help` 返回双层信息：
  - `quickstart`：最短可执行示例；
  - `advanced`：可选参数与约束说明（按 topic 展开）。

### 3) 权限与审计保持细粒度

- facade 路由后必须映射到明确内部 action，再进入 `allowed_tools` 与 mode 决策。
- 审批与拒绝事件必须记录映射后的 action，避免“只看到 facade，不知道实际执行了什么”。
- 对 `process` 类操作继续强制走 `execpolicy`；不引入 shell 字符串拼接执行。

### 4) 交付与回滚策略

- 引入环境开关：
  - `OMNE_TOOL_FACADE_ENABLED=true|false`
  - `OMNE_TOOL_FACADE_EXPOSE_LEGACY=true|false`
- 默认开启 facade、关闭 legacy 暴露。
- 任一线上回归可快速回切 legacy 暴露，不阻断交付。

### 5) Mainline 开发策略（必须遵守）

- 本主题全部后续开发直接基于 `main` 小步提交推进。
- 每次提交必须满足最小验证门槛（编译 + 关键测试 + 文档同步）。
- 禁止为该主题维护长期分叉分支，避免规范与实现漂移。

## 非目标

- 不在本阶段改写底层安全模型（sandbox/approval/execpolicy 语义保持不变）。
- 不在本阶段删除内部细粒度工具实现。
- 不在本阶段引入跨进程插件系统或动态远程工具市场。

## 验收标准

- 工具面：默认 model-facing 工具数 `<= 5`。
- 成本：`tool_schema_bytes` 相比当前基线显著下降（目标 `>= 50%`）。
- 安全：既有拒绝路径（`allowed_tools_denied`、`mode_denied`、`execpolicy_denied`、`approval_denied`）行为不回退。
- 体验：每个 facade 都可通过 `op=help` 获取完整扩展用法。
- 可运维：出现回归时可通过开关回切 legacy 暴露。
- 流程：提交记录与文档明确标注“在 main 上持续开发”的执行事实。

## 对齐缺口与后续路线（vs Codex/OpenCode）

在本提案范围内，facade 收敛与 help-first 已落地；但对照 Codex/OpenCode，仍有后续缺口：

1. 动态工具注册已落地 MVP（默认关闭、只读映射）；插件生态与写操作治理仍待后续扩展。
2. `role` 与 `mode` 已完成执行链与 explain 口径正交，但默认角色策略与更细粒度权限模板仍可继续演进。

建议后续推进顺序：

1. 先扩展动态注册的测试矩阵与示例模板。
2. 再推进动态工具插件化与写操作治理（范围最大，需单独变更包）。

## 可选开关默认策略（落地口径）

当前建议默认值保持：

- `OMNE_TOOL_FACADE_ENABLED=true`
- `OMNE_TOOL_FACADE_EXPOSE_LEGACY=false`
- `OMNE_ENABLE_MCP=false`
- `OMNE_TOOL_EXPOSE_WEB=false`
- `OMNE_TOOL_EXPOSE_SUBAGENT=false`
- `OMNE_TOOL_EXPOSE_THREAD_INTROSPECTION=false`
- `OMNE_TOOL_EXPOSE_THREAD_HOOK=false`
- `OMNE_TOOL_EXPOSE_REPO_SYMBOLS=false`
- `OMNE_TOOL_MODEL_PROFILE=auto(full/compact)`

## 当前实现状态（2026-03-02）

- 已落地 facade tools：`workspace/process/thread/artifact/integration`。
- 已落地 `thread` 子代理生命周期 facade：`send_input/wait/close`（兼容 `close_agent` 别名）。
- 已落地开关与默认值：
  - `OMNE_TOOL_FACADE_ENABLED=true`
  - `OMNE_TOOL_FACADE_EXPOSE_LEGACY=false`
- 已落地 help-first：每个 facade 支持 `op=help` 与按 `topic` 查询。
- 已落地审计字段：facade wrapper 的 `ToolStarted/ToolCompleted` 可追踪 `facade_tool/op/mapped_action`。
- 已落地 CLI/TUI 映射摘要展示：`ask/event/tui` 视图会显示 `facade_tool/op/mapped_action`。
- 已落地 role/mode 正交收口：
  - 运行时权限 gate 以 `mode` 为准；
  - `thread/configure` 与 `thread/config_explain` 显式执行/展示
    `effective_permissions = mode ∩ role(permission_mode) ∩ allowed_tools`。
- 已落地动态工具注册 MVP：
  - 开关：`OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED`（默认 `false`）；
  - registry：`<thread_root>/.omne_data/spec/tool_registry.json`（可选 `OMNE_TOOL_DYNAMIC_REGISTRY_PATH`）；
  - 仅支持 read-only mapped tool（fail-closed）。
- 已完成一轮对比实测（`cargo test -p omne-app-server tool_catalog_tests::facade_tool_surface_reduces_schema_bytes_vs_legacy_default -- --nocapture`）：
  - legacy：`tool_count=21`，`tool_schema_bytes=7669`
  - facade：`tool_count=4`，`tool_schema_bytes=1815`
  - schema bytes 降幅约 `76.33%`
- 全量回归（`cargo test -p omne-app-server -- --nocapture`）通过：`448 passed; 0 failed`。

## 实现归属补充（2026-03-04）

- 文件工具执行链路已收敛：`file/read|glob|grep|write|patch|edit|delete|fs/mkdir`
  均走 `app-server -> omne-fs-runtime -> safe-fs-tools`。
- `omne-app-server` 已移除 `diffy` 直依赖；`file/patch` 由
  `safe-fs-tools::apply_unified_patch` 执行（`diffy` 仅在更底层依赖中出现）。
- `thread/diff` 的真实执行形态是：`git-runtime` 提供 recipe/limits，
  app-server 通过 `process/start` 执行命令并经 artifact 管线落盘，而不是“单点 runtime 直接出 diff 文本”。
- `artifact` 相关能力并非仅 `omne-artifact-store`：版本、历史快照、裁剪报告等编排逻辑仍在 app-server。
- `integration` 中 `web_search/web_fetch/view_image` 的主要实现位于
  `run_tool_call_once.rs`，与 `mcp/runtime.rs`（MCP 管理）职责分离。
