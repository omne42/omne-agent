# TUI（Terminal UI）：v0.2.0 P0 规格（薄客户端）

> 结论先行：TUI 不是“另一套 core”，只是 `omne-app-server`（JSON-RPC 事件流）的一个投影。**唯一真相**仍然是 `Thread/Turn/Item` 落盘事件与可重放语义。
>
> v0.2.0 现状：已落地 `omne`/`omne tui`（默认新建 thread；Esc 回 thread picker；Ctrl-K 指令菜单；Ctrl-C 中断（空闲退出）；transcript scrollback；`item/delta` 流式；支持 `/allowed-tools` 与 `/execpolicy-rules` 的 thread 级快速配置；thread picker 支持 `h` 切换是否包含 archived threads，支持 `l`（linkage）、`a`（auto-apply-error）、`b`（fan-in dependency blocked）与 `s`（subagent proxy approvals）快捷切换过滤，支持 `c` 一键清空过滤；列表头显式显示 `threads [all|link|auto|fanin|subagent|...]` 与 `archived=on|off`、footer 显示 `threads f=all|link|auto|fanin|subagent|...`，并在列表 Attention badge 中显示 `sub<N>`（待处理 subagent proxy approvals 数量，`N>999` 时显示 `sub999+`）；进入 thread 后，footer 也会显示 `sub=<total>`（宽屏附带状态分布如 `running:2,failed:1`），便于快速判断是否需要先处理子任务审批；打开 approvals overlay 时，标题会展示 `filter=<all|failed|running>`、`failed=<N>`（失败子任务审批计数）与 `sub=<total>(...)` 汇总，pending 行会对 `subagent/proxy_approval` 直接附加紧凑状态提示（如 `(running)` / `(failed)`，并按状态着色：`running` 黄色、`failed` 红色），同时列表默认按风险优先排序（`failed/error` 子任务审批在前，其次 `running`，再到其它），并支持 `t` 循环过滤（all/failed/running）与 `f/F` 快捷键跳到下一个/上一个 failed/error 子任务审批，方便在审批列表内保持全局感知；同一能力也可通过 Ctrl-K 根菜单 `archived=on|off`、`linkage-filter=on|off`、`auto-apply-filter=on|off`、`fan-in-filter=on|off`、`subagent-filter=on|off` 与 `clear-filters` 切换/重置；footer 显示当前 thread gate 摘要 `g=<allowed_tools_count|*>/<execpolicy_rules_count>`；Ctrl-K 根菜单显示 `allowed-tools=<*|N>` 与 `execpolicy-rules=<N>` 当前值）以及 Approvals/Processes/Artifacts 弹窗（只调用既有 JSON-RPC；无 stdin/PTY 交互；Artifacts 支持 `artifact/versions` 历史版本浏览、按版本读取，提供 `0` 快捷回到 latest 与 `R` 强制刷新版本列表；若历史版本已清理/不存在会提示回退到 latest）。

## 1) v0.2.0 P0 目标与边界

### 1.1 必须做（P0）

- `omne tui`：全屏交互 UI（Rust，Ratatui 风格）。
- Thread 列表：展示 thread 元信息与 Attention 状态，能进入某个 thread。
- Thread 视图：
  - 订阅事件并增量渲染 transcript（含 `item/delta` 文本流）。
  - 支持输入并提交 `turn/start`。
- Approvals：展示 pending approvals，并在 UI 内 `approve/deny`（调用既有 JSON-RPC；不引入新语义）。
  - 在 approvals overlay 内支持 `Ctrl-K` 打开局部 command palette（`refresh` / `select-prev` / `select-next` / `filter-cycle` / `next-failed` / `prev-failed` / `approve` / `deny` / `remember-toggle` / `details`）。
- Process：只读 `inspect`（stdout/stderr tail）+ `kill` + `interrupt`（继续遵守 v0.2.0 约束：**禁止 stdin/PTY 交互**）。
  - 在 processes overlay 内支持 `Ctrl-K` 打开局部 command palette（`refresh` / `select-prev` / `select-next` / `inspect` / `kill` / `interrupt`）。
- Artifacts：列表/读取（内置滚动查看，pager/less 级别即可），支持加载版本列表并按历史版本读取。
  - 在 artifacts overlay 内支持 `Ctrl-K` 打开局部 command palette（`refresh` / `select-prev` / `select-next` / `read` / `versions` / `versions-reload` / `version-prev` / `version-next` / `version-latest`）。
  - `Ctrl-K` 触发后会在 status 显示 overlay 上下文（`overlay commands: approvals|processes|artifacts`）；若当前 overlay 不支持局部菜单则显示 `overlay commands unavailable`；两类提示均约 2 秒后自动清理。

### 1.2 明确不做（v0.2.0）

- 不做“交互式进程 attach”（stdin/PTY）；只读查看 + kill。
- 不引入新的“UI 专用语义/协议”；TUI 只是既有 app-server API 的一个 client。
- 不追求 60fps 动画；渲染是事件驱动（输入/事件/resize 才重绘）。
- 不做远程多端同步/多人协作 UI（P1 再谈）。

## 2) 架构（数据结构与所有权）

### 2.1 进程边界：TUI = `omne` 的一个前端

默认形态与现有 `omne cli`（REPL 风格）一致：

- `omne`/`omne tui` 优先连接 `<omne_root>/daemon.sock`，失败则 spawn `omne-app-server`，并完成 `initialize/initialized`。
- TUI 只做两件事：
  1. 消费 notifications + `thread/subscribe` 的重放事件，派生出 UI state；
  2. 把用户动作映射为 JSON-RPC request（`thread/*`、`turn/*`、`approval/*`、`process/*`、`artifact/*`）。

### 2.2 状态来源：事件流优先，UI state 必须可重建

- UI 不拥有业务状态；它只缓存派生视图（例如当前 thread 的 `ThreadState`、当前选中的 item）。
- 断线/崩溃恢复：记录 `since_seq`，重启后用 `thread/state`/`thread/subscribe` 从 `last_seq + 1` 补齐。

> 经验：Codex 的 TUI 直接驱动 core；OpenCode 的 TUI 通过 client/server+事件流驱动。对 OmneAgent 来说，**我们已经有 app-server + 事件模型**，最简单也最稳的是：TUI 做 thin client，不要再造一套“UI 专用 core”。

## 3) 渲染与性能（别自找麻烦）

- 不固定帧率；用“事件触发重绘”：
  - 键盘输入/鼠标滚轮/resize → schedule frame
  - 新事件到达 → schedule frame
- 对高频 `item/delta` 做批处理：
  - 允许以 16ms 窗口合并多条 delta（减少抖动与 CPU）
  - 但不能牺牲交互延迟（窗口只在 burst 时生效）

## 4) 终端工程（必须可恢复）

- 进入/退出全屏（alt-screen）要可靠；panic/异常必须恢复终端状态。
- 打开外部 pager/editor 时，先“暂时恢复终端模式”，结束后再接管（参考 Codex 的 `with_restored` 思路）。

## 5) 测试策略（别把 TUI 变成黑盒）

- 使用 Ratatui `TestBackend` 做 snapshot tests（参考 Codex 思路），至少覆盖：
  - thread 列表渲染
  - transcript 基础渲染
  - overlays（approvals/processes/artifacts）的基础渲染与交互状态
- 关键渲染函数尽量纯函数化（输入 state → 输出 layout），降低 `.clone()` 与生命周期噪音。

## 6) 参考实现（上游快照）

- Codex（Ratatui + tokio select + 终端恢复 + vt100 tests）：
  - `example/codex/codex-rs/tui/src/app.rs`
  - `example/codex/codex-rs/tui/src/tui.rs`
  - `example/codex/codex-rs/tui/src/test_backend.rs`
- OpenCode（OpenTUI + Worker + 事件批处理 + attach 思路）：
  - `example/opencode/packages/opencode/src/cli/cmd/tui/thread.ts`
  - `example/opencode/packages/opencode/src/cli/cmd/tui/worker.ts`
  - `example/opencode/packages/opencode/src/cli/cmd/tui/context/sdk.tsx`
