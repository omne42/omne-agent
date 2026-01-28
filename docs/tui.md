# TUI（Terminal UI）：v0.2.0 P0 规格（薄客户端）

> 结论先行：TUI 不是“另一套 core”，只是 `pm-app-server`（JSON-RPC 事件流）的一个投影。**唯一真相**仍然是 `Thread/Turn/Item` 落盘事件与可重放语义。
>
> v0.2.0 现状：已落地 `pm`/`pm tui`（默认新建 thread；Esc 回 thread picker；Ctrl-K 指令菜单；Ctrl-C 中断（空闲退出）；transcript scrollback；`item/delta` 流式）以及 Approvals/Processes/Artifacts 弹窗（只调用既有 JSON-RPC；无 stdin/PTY 交互）。

## 1) v0.2.0 P0 目标与边界

### 1.1 必须做（P0）

- `pm tui`：全屏交互 UI（Rust，Ratatui 风格）。
- Thread 列表：展示 thread 元信息与 Attention 状态，能进入某个 thread。
- Thread 视图：
  - 订阅事件并增量渲染 transcript（含 `item/delta` 文本流）。
  - 支持输入并提交 `turn/start`。
- Approvals：展示 pending approvals，并在 UI 内 `approve/deny`（调用既有 JSON-RPC；不引入新语义）。
- Process：只读 `inspect`（stdout/stderr tail）+ `kill` + `interrupt`（继续遵守 v0.2.0 约束：**禁止 stdin/PTY 交互**）。
- Artifacts：列表/读取（内置滚动查看，pager/less 级别即可）。

### 1.2 明确不做（v0.2.0）

- 不做“交互式进程 attach”（stdin/PTY）；只读查看 + kill。
- 不引入新的“UI 专用语义/协议”；TUI 只是既有 app-server API 的一个 client。
- 不追求 60fps 动画；渲染是事件驱动（输入/事件/resize 才重绘）。
- 不做远程多端同步/多人协作 UI（P1 再谈）。

## 2) 架构（数据结构与所有权）

### 2.1 进程边界：TUI = `pm` 的一个前端

默认形态与现有 `pm cli`（REPL 风格）一致：

- `pm`/`pm tui` 优先连接 `<pm_root>/daemon.sock`，失败则 spawn `pm-app-server`，并完成 `initialize/initialized`。
- TUI 只做两件事：
  1. 消费 notifications + `thread/subscribe` 的重放事件，派生出 UI state；
  2. 把用户动作映射为 JSON-RPC request（`thread/*`、`turn/*`、`approval/*`、`process/*`、`artifact/*`）。

### 2.2 状态来源：事件流优先，UI state 必须可重建

- UI 不拥有业务状态；它只缓存派生视图（例如当前 thread 的 `ThreadState`、当前选中的 item）。
- 断线/崩溃恢复：记录 `since_seq`，重启后用 `thread/state`/`thread/subscribe` 从 `last_seq + 1` 补齐。

> 经验：Codex 的 TUI 直接驱动 core；OpenCode 的 TUI 通过 client/server+事件流驱动。对 CodePM 来说，**我们已经有 app-server + 事件模型**，最简单也最稳的是：TUI 做 thin client，不要再造一套“UI 专用 core”。

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
