# TODO（Roadmap & Backlog）

> 口径：这里只记录**新增**待办项与跨文档的版本计划，不复制 `docs/v0.2.0_parity.md`。
>
> 每条 TODO 需要能验收：写清楚边界、落点（目录/协议）、以及最小可复制验证命令。

---

## v0.3.0：Node.js（分发 + 集成）（TODO）

**核心判断：** 值得做，但 Node 必须保持“薄”。Rust 是唯一可信执行体；Node 只做包装与集成。

### 目标（v0.3.0）

- 提供 Node 侧入口（npm 安装/IDE/GUI 集成）来启动并驱动 `pm-app-server`。
- 发布与协议对齐的 TypeScript types（由 Rust 单一真源生成），避免前后端漂移。

### 明确边界（写死）

- Node **不实现** agent loop / tools / approvals / sandbox / execpolicy / eventlog（这些全部留在 Rust）。
- GUI/TUI **都只是 client**：业务状态来自 `Thread/Turn/Item` 落盘与回放；UI state 必须可重建。
- 任何副作用（写盘/跑命令/联网）必须通过 `pm-app-server` 的 JSON-RPC 工具接口触发，并接受 `mode gate → sandbox → execpolicy → approvals` 裁决。

### 待办项（建议拆分）

- [ ] Node 交付形态：
  - A) npm 包内 vendoring 多平台 Rust binaries（Codex `codex-cli/bin/codex.js` 模式）
- [ ] 定义 Node 目录布局（建议 `packages/*`，与 Rust `crates/*` 平行；不污染 Rust workspace）
- [ ] 生成并发布协议 TS types（单一真源：`crates/app-server-protocol`）：
  - `pm-app-server generate-ts --out <dir>`
  - `pm-app-server generate-json-schema --out <dir>`（可选，供 GUI 校验/调试）
- [ ] Node launcher：
  - 复刻 `example/codex/codex-cli/bin/codex.js` 的模式：target triple 映射 + `spawn()` + 信号转发
  - vendor layout 建议：`vendor/<targetTriple>/pm/pm[.exe]`、`vendor/<targetTriple>/pm/pm-app-server[.exe]`
- [ ] Node client：
  - 负责 spawn/连接 `pm-app-server`（stdio JSON-RPC/JSONL）并提供订阅/重连能力（`since_seq`）
  - 严禁在 Node 侧“补齐权限逻辑”（所有权限/审批语义只来自 server 返回）
- [ ] GUI（可选；仅在 protocol/types 稳定后启动）：
  - 建议 TypeScript（React）实现，作为 app-server client；与 TUI 一样只消费事件流
  - 打包形态可选 Electron/Tauri（后者仍应把业务逻辑留在 Rust server，而不是搬进 GUI 进程）

### 最小验收（可复制）

> 这是一条“能跑”的验收，不是设计文档验收。

- Node launcher 模式（A）：
  - `node ./packages/<pkg>/bin/pm.js --help`
  - `node ./packages/<pkg>/bin/pm.js app-server --help`（或等价方式启动 server）
- Node SDK 模式（B）：
  - `pm-app-server --help`（先确保用户机器已有二进制）
  - `node ./packages/<client>/examples/basic.mjs`（跑通 `initialize → thread/start → turn/start`）

---

## UI 放置结论（长期不变）

- **TUI**：继续放在 Rust（`crates/agent-cli`，`pm tui`），thin client，见 `docs/tui.md`。
- **GUI**：放在 Node/TypeScript（未来 `packages/gui` 或独立 repo），thin client，通过 `pm-app-server` 协议驱动；不要复制任何 core 语义到前端。
