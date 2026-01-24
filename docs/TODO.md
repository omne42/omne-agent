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

- [x] Node 交付形态：先落地 **B（thin launcher/client）**，vendoring（A）作为后续增强
  - B) 不打包 Rust binaries：Node 只做“启动/连接/协议封装”，二进制来自 PATH 或显式指定
  - A) npm 包内 vendoring 多平台 Rust binaries（Codex `codex-cli/bin/codex.js` 模式；后续再做）
- [x] 定义 Node 目录布局（`packages/*`，与 Rust `crates/*` 平行；不污染 Rust workspace）
  - 已新增：`packages/pm`（launcher）、`packages/pm-client`（stdio JSON-RPC client）
- [ ] 生成并发布协议 TS types（单一真源：`crates/app-server-protocol`）
  - [x] 已支持生成：`pm-app-server generate-ts --out <dir>` / `pm-app-server generate-json-schema --out <dir>`
  - [ ] 产物以 Node package 形态发布（例如 `packages/pm-app-server-protocol`），避免 consumers 依赖 Rust toolchain
- [x] Node launcher（`packages/pm/bin/pm.js`）
  - target triple 映射 + vendored layout 探测（预留）+ env override + PATH fallback
  - 信号转发（SIGINT/SIGTERM/SIGHUP）+ exit code 透传
- [x] Node client（`packages/pm-client`）
  - stdio JSON-RPC client：`call(method, params)` + notifications 解析
  - 示例：`examples/basic.mjs` 跑通 `initialize → thread/start → thread/subscribe(wait_ms=0)`（不触发 LLM）
  - 严禁在 Node 侧“补齐权限逻辑”（所有权限/审批语义只来自 server 返回）
- [ ] Node client（增强）：订阅/重连能力（`since_seq` 断点续读）+ 断线重连策略
- [ ] GUI（可选；仅在 protocol/types 稳定后启动）
  - 建议 TypeScript（React）实现，作为 app-server client；与 TUI 一样只消费事件流
  - 打包形态可选 Electron/Tauri（后者仍应把业务逻辑留在 Rust server，而不是搬进 GUI 进程）

### 最小验收（可复制）

> 这是一条“能跑”的验收，不是设计文档验收。

- Node launcher（当前为 B：PATH/env override；A vendoring 后续再补）：
  - `CODE_PM_PM_BIN=target/debug/pm node ./packages/pm/bin/pm.js --help`
  - `CODE_PM_APP_SERVER_BIN=target/debug/pm-app-server node ./packages/pm/bin/pm.js app-server --help`
- Node SDK 模式（B）：
  - `CODE_PM_APP_SERVER_BIN=target/debug/pm-app-server node ./packages/pm-client/examples/basic.mjs`

---

## UI 放置结论（长期不变）

- **TUI**：继续放在 Rust（`crates/agent-cli`，`pm tui`），thin client，见 `docs/tui.md`。
- **GUI**：放在 Node/TypeScript（未来 `packages/gui` 或独立 repo），thin client，通过 `pm-app-server` 协议驱动；不要复制任何 core 语义到前端。
