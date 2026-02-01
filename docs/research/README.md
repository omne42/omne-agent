# Example 仓库调研索引

本目录收录对 `example/` 下各仓库的“长篇能力与设计分析”，用于指导 `omne-agent` 的架构取舍与实现优先级。

> 说明：调研基于本仓库内的 snapshot（例如 `example/<repo>` 或 `example/agent-gui/<repo>`），与上游可能存在差异。每篇文档顶部会标注对应 snapshot 的 git commit。

## 文档列表

| 仓库 | Snapshot | 重点方向 | 分析文档 |
| --- | --- | --- | --- |
| Claude Code | `74cc597` | 插件/Hook/Workflow 体系、权限与安全 guardrails、并行 review/commit 流程 | `docs/research/claude-code.md` |
| Claude Code Router | `c73fe0d` | 多 Provider 路由、请求/响应变换(Transformers)、配置/预设/激活机制 | `docs/research/claude-code-router.md` |
| Codex | `b66018a` | Rust 核心（`codex-rs`）、app-server 协议、Sandbox/ExecPolicy、Responses API（本期重点） | `docs/research/codex.md` |
| CodexMonitor | `b1d3182` | 多 workspace 编排、`codex app-server` 客户端、worktree 管理、远端 daemon POC | `docs/research/codexmonitor.md` |
| Kilo Code | `e4ced0062c` | VSCode Agent 平台的“模式/权限/工作流/技能”设计、fork 合并策略（markers） | `docs/research/kilocode.md` |
| OpenCode | `3fd0043d1` | Project/Session/Storage/Bus/Worktree 的工程化实现、client/server、provider-agnostic | `docs/research/opencode.md` |
| 1Code | `e23a469` | worktree 隔离、plan gate、worktree setup config（可复用为脚本化生命周期） | `docs/research/onecode.md` |
| Superset | `8d17373` | 多 agent 并行调度台、`.superset/config.json` + setup/teardown、外部资源隔离（端口/DB/生命周期脚本） | `docs/research/superset.md` |
| AionUi | `5abe63b` | CLI agent cowork、CLI/MCP 检测、预览面板与 preview 历史、WebUI 远程 | `docs/research/aion-ui.md` |

## 外部参考（非 snapshot）

| 主题 | 发布日期 | 重点方向 | 笔记 |
| --- | --- | --- | --- |
| Unrolling the Codex agent loop（OpenAI） | 2026-01-23 | stateless agent loop、prompt caching、ZDR、compaction | `docs/research/unrolling-the-codex-agent-loop.md` |
| Codex PR #1641（ZDR + sqlite + 内存密钥） | 2025-07-09 | 本地敏感数据存储策略、ZDR 取舍 | `docs/research/codex-pr-1641-zdr-sqlite.md` |

## 我们的落地方向（先写在这里，便于对齐）

- `omne-agent vNext` 以 `example/codex` 为主底座进行“魔改/复用”，允许直接复制/挪用其能力（优先 Rust 侧 `codex-rs`）。
- **第一阶段只要求支持 OpenAI Responses API**（未来再扩展到其它接口/Provider）。
- 核心基建优先级：**可编排的 Agent CLI（tool/sandbox/approvals + 事件流）** > 并发与隔离（workspace 生命周期脚本化） > artifacts/preview > Git 交付适配（可选）。
- 交互目标：实现“RTS 风格”的多 agent 控制台能力（高并发不是目的，**可观测/可暂停/可回放/可收口**才是）。
