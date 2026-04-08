# omne-agent Architecture

这个文件只描述 workspace 顶层分层与依赖方向，不展开实现细节。更窄的稳定事实继续写在 `README.md`、`docs/README.md` 和专题文档里。

## 顶层分层

- `crates/app-server/`
  - 控制面、agent loop、tool dispatch、turn/thread 生命周期、JSONL 事件落盘与回放。
- `crates/agent-cli/`
  - 人类可用 CLI / TUI；通过 JSON-RPC 调用 `omne-app-server`，不重复实现控制面语义。
- `crates/app-server-protocol/`、`crates/agent-protocol/`
  - 跨边界 DTO、schema、导出产物；协议层不承载业务编排。
- `crates/core/`
  - 仓库内共享的 agent runtime 通用能力：paths、storage、threads、modes、redaction、router 等。
- `crates/eventlog/`
  - append-only JSONL event log 与派生读取。
- `crates/*-runtime` / `crates/*-spec`
  - 当前仓内仍保留、但应保持窄边界的运行时适配与规格层。

## 依赖方向

- `agent-cli` -> `app-server-protocol` / `agent-protocol` / `core`
- `app-server` -> `core` / `eventlog` / `app-server-protocol` / `agent-protocol`
- `core` -> `eventlog` / `agent-protocol`
- `protocol` crates 不反向依赖 `app-server` 或 `agent-cli`

## 边界约束

- `omne-agent` 不是通用 foundation 仓库；更硬、更窄的 primitives 继续进入 `omne-runtime`。
- 通用 config / MCP / notify / policy metadata 等共享 kit 优先复用 `omne_foundation`，不要在控制面继续复制领域实现。
- `app-server` 保留审批、事件编排和 agent runtime 语义；不要在这里重新长出共享 SDK 或 provider transport 层。

## 继续下钻

- 外部概览：`README.md`
- 执行者地图：`AGENTS.md`
- 文档入口：`docs/README.md`
- 文档系统地图：`docs/docs-system-map.md`
- 边界审计：`docs/reports/domain-boundary-audit-20260304.md`
