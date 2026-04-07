# TODO（Roadmap & Backlog）

> 口径：这里只记录**新增**待办项与跨文档的版本计划，不复制 `docs/v0.2.0_parity.md`。
>
> 每条 TODO 需要能验收：写清楚边界、落点（目录/协议）、以及最小可复制验证命令。

---

## v0.3.0：Node.js（分发 + 集成）（TODO）

**核心判断：** 值得做，但 Node 必须保持“薄”。Rust 是唯一可信执行体；Node 只做包装与集成。

### 目标（v0.3.0）

- 提供 Node 侧入口（npm 安装/IDE 集成）来启动并驱动 `omne-app-server`。
- 发布与协议对齐的 TypeScript types（由 Rust 单一真源生成），避免前后端漂移。

### 明确边界（写死）

- Node **不实现** agent loop / tools / approvals / sandbox / execpolicy / eventlog（这些全部留在 Rust）。
- TUI 是当前唯一 UI client：业务状态来自 `Thread/Turn/Item` 落盘与回放；UI state 必须可重建。
- 任何副作用（写盘/跑命令/联网）必须通过 `omne-app-server` 的 JSON-RPC 工具接口触发，并接受 `allowed_tools`、hard boundary / config validation，以及进入策略合并后的 `mode gate → execpolicy → approvals` 裁决。

### 待办项（建议拆分）

- [x] Node 交付形态：先落地 **B（thin launcher/client）**，vendoring（A）作为后续增强
  - B) 不打包 Rust binaries：Node 只做“启动/连接/协议封装”，二进制来自 PATH 或显式指定
  - A) npm 包内 vendoring 多平台 Rust binaries（Codex `codex-cli/bin/codex.js` 模式）已补最小基础：launcher 支持 vendored `path/` prepend + `packages/omne/scripts/assemble-vendor.mjs` 组装脚本 + `packages/omne/scripts/build-vendor-bundle.mjs` 产物清单（`manifest.json`）+ `packages/omne/scripts/verify-vendor-bundle.mjs` 完整性校验 + `packages/omne/scripts/release-vendor-bundle.mjs` 版本化 release 目录（`RELEASE.json`/`SHA256SUMS`）+ `packages/omne/scripts/release-host-vendor-bundle.mjs` 主机目标自动解析（支持自动版本号）+ `packages/omne/scripts/update-release-index.mjs` 索引生成 + `packages/omne/scripts/release-local-vendor-bundle.mjs` 一键本地发布 + `packages/omne/scripts/release-matrix-vendor-bundle.mjs` 多目标矩阵发布与 `last-run.json` 汇总；并已补最小 CI 预演工作流（`.github/workflows/omne-node-vendor.yml`：Linux/macOS/Windows host matrix 的 check/test + host release artifact，支持手动传入 `profile/version/clean`，`v*` tag 自动以 tag 作为版本上传 release assets 并附 `SHA256SUMS`，发布前通过 `packages/omne/scripts/validate-tag-release-artifacts.mjs` 校验 artifact 结构、per-target 唯一性、`index.json` 与 bundle 目录目标集合完全一致、`release_dir`/bundle-name 与 `version/target` 一致性，再通过 `packages/omne/scripts/package-tag-release-assets.mjs` 仅打包当前 tag 对应版本目录（同样强制目标集合一致），并通过 `packages/omne/scripts/verify-tag-release-tarballs.mjs` 校验 tarball 条目白名单与 tar 内 `index.json`/`RELEASE.json` 语义一致性）；完整自动化发布策略仍后续增强
- [x] 定义 Node 目录布局（`packages/*`，与 Rust `crates/*` 平行；不污染 Rust workspace）
  - 已新增：`packages/omne`（launcher）、`packages/omne-client`（stdio JSON-RPC client）
- [x] 生成并发布协议 TS types（单一真源：`crates/app-server-protocol`）
  - [x] 已支持生成：`omne-app-server generate-ts --out <dir>` / `omne-app-server generate-json-schema --out <dir>`
  - [x] 产物以 Node package 形态发布：`packages/omne-app-server-protocol`（包含 `generated/*.d.ts` + `schema/*.schema.json` + `scripts/sync.mjs`）
- [x] Node launcher（`packages/omne/bin/omne.js`）
  - target triple 映射 + vendored layout 探测（预留）+ env override + PATH fallback
  - 信号转发（SIGINT/SIGTERM/SIGHUP）+ exit code 透传
- [x] Node client（`packages/omne-client`）
  - stdio JSON-RPC client：`call(method, params)` + notifications 解析
  - 示例：`examples/basic.mjs` 跑通 `initialize → thread/start → thread/subscribe(wait_ms=0)`（不触发 LLM）
  - 严禁在 Node 侧“补齐权限逻辑”（所有权限/审批语义只来自 server 返回）
- [x] Node client（增强）：订阅/重连能力（`since_seq` 断点续读）+ 断线重连策略
  - 已新增：`ThreadSubscribeStream`（`packages/omne-client/src/index.js`），支持自动重连（指数退避）与 `since_seq` 恢复
- [ ] Web GUI（暂停）
  - 当前阶段不做 Web GUI，仓库内相关实现已移除（原 `packages/omne-gui`）。
  - 若未来重启，需单独立项并重新确认范围（仍需坚持 thin client，不复制 Rust core 语义）。

### 最小验收（可复制）

> 这是一条“能跑”的验收，不是设计文档验收。

- Node launcher（当前为 B：PATH/env override；A vendoring 后续再补）：
  - `OMNE_PM_BIN=target/debug/omne node ./packages/omne/bin/omne.js --help`
  - `OMNE_APP_SERVER_BIN=target/debug/omne-app-server node ./packages/omne/bin/omne.js app-server --help`
  - `node ./packages/omne/scripts/assemble-vendor.mjs --target x86_64-unknown-linux-gnu --omne ./target/debug/omne --app-server ./target/debug/omne-app-server --clean`
  - `node ./packages/omne/scripts/build-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --omne ./target/debug/omne --app-server ./target/debug/omne-app-server --clean`
  - `node ./packages/omne/scripts/verify-vendor-bundle.mjs --bundle ./packages/omne/dist/vendor-bundle-x86_64-unknown-linux-gnu`
  - `node ./packages/omne/scripts/release-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --version v0.3.0-test --omne ./target/debug/omne --app-server ./target/debug/omne-app-server --clean`
  - `node ./packages/omne/scripts/release-host-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --target-dir ./target --profile debug --clean`
  - `node ./packages/omne/scripts/release-local-vendor-bundle.mjs --target x86_64-unknown-linux-gnu --target-dir ./target --profile debug --clean`
  - `node ./packages/omne/scripts/release-matrix-vendor-bundle.mjs --version v0.3.0-test --targets x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu --target-dir ./target --profile debug --clean`
  - `node ./packages/omne/scripts/release-matrix-vendor-bundle.mjs --version v0.3.0-test --target-dir ./target --profile debug --clean`（默认全矩阵目标）
  - `node ./packages/omne/scripts/update-release-index.mjs --release-out ./packages/omne/dist/releases`
- Node SDK 模式（B）：
  - `OMNE_APP_SERVER_BIN=target/debug/omne-app-server node ./packages/omne-client/examples/basic.mjs`
- Node SDK 订阅重连（B）：
  - `OMNE_APP_SERVER_BIN=target/debug/omne-app-server node ./packages/omne-client/examples/subscribe-resume.mjs`
- 协议 types/schema 包同步：
  - `node ./packages/omne-app-server-protocol/scripts/sync.mjs`
---

## v0.2.x：Rust Backend 收尾（TODO）

- [x] 基于 `subagent_start/subagent_stop` hooks 增加“fan-in 自动收敛模板”
  - 边界：仅生成/更新父 thread artifact（不改 mode gate / approval 语义，不做 GUI）
  - 落点：`crates/app-server/src/agent/tools/dispatch/subagents.rs`、`docs/subagents.md`
  - 验收：
    - `cargo test -p omne-app-server agent::agent_spawn_guard_tests::subagent_schedule_catch_up_running_events_writes_fan_in_summary_artifact -- --nocapture`
    - `cargo test -p omne-app-server agent::agent_spawn_guard_tests::subagent_schedule_catch_up_running_events_triggers_subagent_stop_hook -- --nocapture`

---

## UI 放置结论（长期不变）

- **TUI**：继续放在 Rust（`crates/agent-cli`，`omne tui`），thin client，见 `docs/tui.md`。
- **Web GUI**：当前阶段不做，仓库内不保留对应实现；如需恢复必须单独立项评审。
