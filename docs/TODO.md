# TODO（Roadmap & Backlog）

> 口径：这里只记录**新增**待办项与跨文档的版本计划，不复制 `docs/v0.2.0_parity.md`。
>
> 每条 TODO 需要能验收：写清楚边界、落点（目录/协议）、以及最小可复制验证命令。

---

## v0.3.0：Node.js（分发 + 集成）（TODO）

**核心判断：** 值得做，但 Node 必须保持“薄”。Rust 是唯一可信执行体；Node 只做包装与集成。

### 目标（v0.3.0）

- 提供 Node 侧入口（npm 安装/IDE/GUI 集成）来启动并驱动 `omne-agent-app-server`。
- 发布与协议对齐的 TypeScript types（由 Rust 单一真源生成），避免前后端漂移。

### 明确边界（写死）

- Node **不实现** agent loop / tools / approvals / sandbox / execpolicy / eventlog（这些全部留在 Rust）。
- GUI/TUI **都只是 client**：业务状态来自 `Thread/Turn/Item` 落盘与回放；UI state 必须可重建。
- 任何副作用（写盘/跑命令/联网）必须通过 `omne-agent-app-server` 的 JSON-RPC 工具接口触发，并接受 `mode gate → sandbox → execpolicy → approvals` 裁决。

### 待办项（建议拆分）

- [x] Node 交付形态：先落地 **B（thin launcher/client）**，vendoring（A）作为后续增强
  - B) 不打包 Rust binaries：Node 只做“启动/连接/协议封装”，二进制来自 PATH 或显式指定
  - A) npm 包内 vendoring 多平台 Rust binaries（Codex `codex-cli/bin/codex.js` 模式；后续再做）
- [x] 定义 Node 目录布局（`packages/*`，与 Rust `crates/*` 平行；不污染 Rust workspace）
  - 已新增：`packages/omne-agent`（launcher）、`packages/app-server-client`（stdio JSON-RPC client）
- [ ] 生成并发布协议 TS types（单一真源：`crates/app-server-protocol`）
  - [x] 已支持生成：`omne-agent-app-server generate-ts --out <dir>` / `omne-agent-app-server generate-json-schema --out <dir>`
  - [ ] 产物以 Node package 形态发布（例如 `packages/omne-agent-app-server-protocol`），避免 consumers 依赖 Rust toolchain
- [x] Node launcher（`packages/omne-agent/bin/omne-agent.js`）
  - target triple 映射 + vendored layout 探测（预留）+ env override + PATH fallback
  - 信号转发（SIGINT/SIGTERM/SIGHUP）+ exit code 透传
- [x] Node client（`packages/app-server-client`）
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
  - `OMNE_AGENT_BIN=target/debug/omne-agent node ./packages/omne-agent/bin/omne-agent.js --help`
  - `OMNE_AGENT_APP_SERVER_BIN=target/debug/omne-agent-app-server node ./packages/omne-agent/bin/omne-agent.js app-server --help`
- Node SDK 模式（B）：
  - `OMNE_AGENT_APP_SERVER_BIN=target/debug/omne-agent-app-server node ./packages/app-server-client/examples/basic.mjs`

---

## UI 放置结论（长期不变）

- **TUI**：继续放在 Rust（`crates/agent-cli`，`omne-agent tui`），thin client，见 `docs/tui.md`。
- **GUI**：放在 Node/TypeScript（未来 `packages/gui` 或独立 repo），thin client，通过 `omne-agent-app-server` 协议驱动；不要复制任何 core 语义到前端。

---

## v0.2.x：跟进 Codex 上游（266 commits）（TODO）

背景：

- Codex 上游快照从 `079fd2adb96bf1b66f3d339e6ee0c0b71f35c322` 更新到 `a90ff831e7d7a049c5638cda6fa72f2abc0b62e6`（差 `266` 次提交）。
- 变更面很大（Rust core/tui/app-server/protocol + sandbox/network-proxy + CI/release + file-search + connectors + personality + smart approvals 等）。

### t0 - 先做“可审计的差异调研”（必须先做）

- [ ] 产出一份可追溯的调研文档：`docs/research/codex-upstream-delta-079fd2adb-a90ff831e.md`
  - 必须包含：按目录/主题分组的提交清单、关键协议/API 变化点、以及“对 omne-agent 的建议采纳/不采纳”结论。
  - 必须在文档顶部记录：调研日期 + 对应 Codex commit range。
  - 最小验收（可复制）：`rg -n "codex-upstream-delta-079fd2adb-a90ff831e" docs/research` 能找到该文档且包含上述字段。

### 候选采纳点（先列清单，后在 t0 文档里给结论）

- [ ] `Personality`（prompt 风格模板化 + thread/config 可控 + TUI 入口）
  - 落点：`crates/core`（thread config）、`crates/app-server`（preamble/instructions 注入）、`crates/agent-cli`（TUI/CLI 命令）。
  - 验收：能在同一 thread 内切换 personality 并落盘事件；`cargo test -p omne-agent-app-server` 覆盖一次切换路径。
- [ ] `Smart approvals`（减少重复确认，但仍然落盘审计）
  - 目标：在不放宽 `deny` 前提下，把“低风险 prompt”自动化；保持 `ApprovalRequested/Decided` 成对写入。
  - 落点：`crates/app-server` 的 approval gate；`docs/approvals.md` 同步策略矩阵。
  - 验收：新增/更新用例覆盖 remembered decision / prompt_strict 不可自动化。
- [ ] `ExecPolicy` 与“requirements”（规则来源统一、可解释）
  - 目标：把 execpolicy rules 的来源层级（global/mode/thread/requirements）做成可解释输出，并保持 fail-closed。
  - 落点：`crates/execpolicy` + `crates/app-server` 配置加载 + `docs/execpolicy.md`。
  - 验收：`omne-agent thread config-explain <thread>` 能解释有效规则来源；`cargo test --workspace` 覆盖至少 1 条解释路径。
- [ ] `Network sandbox`（network-proxy / 平台差异）
  - 目标：在非 Linux 场景也能对出站网络做收口（至少为 “禁止公网 + 只允许 loopback/allowlist” 提供硬约束手段）。
  - 落点：`crates/core` hardening/sandbox；必要时新增独立 crate（但先证明必要性）。
  - 验收：给出可复制的本机验证步骤（至少包含 1 条被阻断与 1 条被允许的请求）。
- [ ] `Connectors`（目录 + runtime 可用性合并）
  - 目标：把“可安装的外部能力”与“当前 thread 可调用的工具”拆开建模并合并展示（可用于 forge/第三方工具生态）。
  - 落点：`docs/mcp.md` + 可能新增 `crates/*`（先在 t0 调研文档里说明最小可行范围）。
  - 验收：能列出 connectors 列表（即便为空）且输出可审计（artifact/event）。
- [ ] `File search`（multi-root + perf）
  - 目标：当 thread CWD 变化或多 repo/worktree 场景下，repo/search 仍然可预测且性能稳定。
  - 落点：`docs/repo_index.md` + `crates/*` 的 repo/search 实现。
  - 验收：新增基准/测试（至少 1 条）证明多 root 结果正确。
- [ ] `Protocol / thread` 细节对齐：thread/read、archive/unarchive、ephemeral thread、plan items/compaction items 等
  - 目标：明确哪些是“必要能力”，哪些是 Codex 内部产品特化；避免盲目对齐。
  - 落点：`crates/app-server-protocol` + `docs/thread_event_model.md`。
  - 验收：协议变更后 `generate-ts` / `generate-json-schema` 可跑通，并有最小 e2e 测试覆盖。
