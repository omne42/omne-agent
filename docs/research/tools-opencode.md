# OpenCode Tooling (example/opencode, Current Code Reality)

## Scope
本文件记录 `example/opencode` 的 tool registry、过滤规则、权限系统与扩展机制。

Primary sources:
- `/root/autodl-tmp/zjj/p/example/opencode/packages/opencode/src/tool/registry.ts`
- `/root/autodl-tmp/zjj/p/example/opencode/packages/opencode/src/flag/flag.ts`
- `/root/autodl-tmp/zjj/p/example/opencode/packages/opencode/src/config/config.ts`
- `/root/autodl-tmp/zjj/p/example/opencode/packages/opencode/src/agent/agent.ts`
- `/root/autodl-tmp/zjj/p/example/opencode/packages/opencode/src/tool/task.ts`
- `/root/autodl-tmp/zjj/p/example/opencode/packages/opencode/src/session/processor.ts`

## 1) 基础注册表（`ToolRegistry.all()`）

按代码顺序，基础工具集合是：
1. `invalid`
2. `question`（仅 `OPENCODE_CLIENT` 为 `app/cli/desktop` 时）
3. `bash`
4. `read`
5. `glob`
6. `grep`
7. `edit`
8. `write`
9. `task`
10. `webfetch`
11. `todowrite`
12. `todoread`
13. `websearch`
14. `codesearch`
15. `skill`
16. `apply_patch`
17. `lsp`（`OPENCODE_EXPERIMENTAL_LSP_TOOL`）
18. `batch`（`config.experimental.batch_tool=true`）
19. `plan_exit` + `plan_enter`（experimental plan mode + cli）
20. `custom`（本地工具文件 + 插件工具）

## 2) 最终暴露前二次过滤（`ToolRegistry.tools(...)`）

过滤条件（provider/model aware）：

1. `codesearch/websearch`
仅在 `providerID=="opencode"` 或启用 `OPENCODE_ENABLE_EXA` 时保留。

2. GPT 系列补丁策略
若 `modelID` 包含 `gpt-`，且不包含 `oss`、`gpt-4`：
- 启用 `apply_patch`
- 禁用 `edit`、`write`

3. GPT 系列 todo 简化
若 `modelID` 包含 `gpt-`：
- 去掉 `todoread`、`todowrite`

结论：最终工具面与 provider/model 强相关，不是固定全集。

## 3) 动态扩展能力（第一类）

`ToolRegistry.state()` 同时加载：
1. 本地 `{tool,tools}/*.{js,ts}`（配置目录）
2. 插件注入工具

两者都通过统一转换路径进入 runtime registry（`fromPlugin(...)`）。

## 4) 关键开关

来自 `flag.ts`：
- `OPENCODE_CLIENT`（默认 `cli`）
- `OPENCODE_ENABLE_EXA`（影响 `websearch/codesearch`）
- `OPENCODE_EXPERIMENTAL_LSP_TOOL`（开启 `lsp`）
- `OPENCODE_EXPERIMENTAL_PLAN_MODE`（开启 `plan_enter/plan_exit`）

来自 `config.ts`：
- `experimental.batch_tool`
- `experimental.primary_tools`
- `experimental.continue_loop_on_deny`

## 5) 权限系统

1. 配置级动作：`ask|allow|deny`
2. 支持按工具与模式做细粒度 permission 匹配（含 wildcard 风格）
3. 兼容 legacy `tools` 布尔配置并迁移到统一 permission 语义
4. `task` 工具可配合 `experimental.primary_tools` 限制子任务工具面
5. denied 之后是否继续循环可由 `continue_loop_on_deny` 控制

## 6) 对 Omne 的可借鉴点

1. 提供“provider/model 感知”的内建过滤，而不是只做轻量 profile。
2. 把本地工具与插件工具纳入统一 registry（不是旁路）。
3. permission 维度更细，策略表达能力更强（ask/allow/deny + pattern）。
4. 对 GPT 家族的编辑/patch 工具做自动换挡，减少误编辑风险。
