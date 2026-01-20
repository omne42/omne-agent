# Example 仓库调研索引

本目录收录对 `example/` 下各仓库的“长篇能力与设计分析”，用于指导 `CodePM` 的架构取舍与实现优先级。

> 说明：调研基于本仓库内的 snapshot（`example/<repo>`），与上游可能存在差异。每篇文档顶部会标注对应 snapshot 的 git commit。

## 文档列表

| 仓库 | Snapshot | 重点方向 | 分析文档 |
| --- | --- | --- | --- |
| Claude Code | `74cc597` | 插件/Hook/Workflow 体系、权限与安全 guardrails、并行 review/commit 流程 | `docs/research/claude-code.md` |
| Claude Code Router | `c73fe0d` | 多 Provider 路由、请求/响应变换(Transformers)、配置/预设/激活机制 | `docs/research/claude-code-router.md` |
| Codex | `b66018a` | Rust 核心（`codex-rs`）、app-server 协议、Sandbox/ExecPolicy、Responses API（本期重点） | `docs/research/codex.md` |
| CodexMonitor | `b1d3182` | 多 workspace 编排、`codex app-server` 客户端、worktree 管理、远端 daemon POC | `docs/research/codexmonitor.md` |
| Kilo Code | `e4ced0062c` | VSCode Agent 平台的“模式/权限/工作流/技能”设计、fork 合并策略（markers） | `docs/research/kilocode.md` |
| OpenCode | `3fd0043d1` | Project/Session/Storage/Bus/Worktree 的工程化实现、client/server、provider-agnostic | `docs/research/opencode.md` |

## 我们的落地方向（先写在这里，便于对齐）

- `CodePM` 以 `example/codex` 为主底座进行“魔改/复用”，允许直接复制/挪用其能力（优先 Rust 侧 `codex-rs`）。
- **第一阶段只要求支持 OpenAI Responses API**（未来再扩展到其它接口/Provider）。
- 本项目当前核心目标是：**临时目录隔离 + 多任务并发 + Git PR 流水线 + AI 合并**；其它 UI/生态能力以“可插拔”方式预留。

