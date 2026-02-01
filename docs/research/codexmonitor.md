# CodexMonitor（example/CodexMonitor）能力与设计分析

> Snapshot: `example/CodexMonitor` @ `b1d3182`
>
> 结论先行：CodexMonitor 的核心价值是把 `codex app-server` 变成“可视化、多 workspace、多 worktree 的编排器”。它对我们最有价值的点有三类：**(1) 作为 app-server 客户端的工程实现**、**(2) worktree 的生命周期管理与“把变更回填到父仓库”的补丁策略**、**(3) 远端 daemon 化（Remote Backend POC）为未来多端/分布式打基础**。

---

## 1. “所有能力”盘点（README 明确列出的功能）

来自 `example/CodexMonitor/README.md`：

- **Workspace 管理**：添加/持久化 workspaces；home dashboard 展示最近 agent 活动。
- **多进程编排**：每个 workspace 启动一个 `codex app-server` 子进程；通过 JSON-RPC 事件流连接；可在 UI 中切换并恢复 threads。
- **Thread/Turn 驱动**：启动 thread、发送消息、渲染 reasoning/tool/diff items、处理 approvals。
- **Worktree agents**：每个 workspace 可创建/删除 git worktrees（存放于数据目录下）；提供 worktree 信息快速查看。
- **Git 面板**：
  - diff stats / file diffs / commit log
  - GitHub Issues（通过 `gh`）
  - 检测 remote 并支持打开 GitHub commit
- **分支管理**：分支列表、checkout、新建分支流程。
- **模型与推理配置**：
  - model picker
  - reasoning effort selector
  - access mode（read-only/current/full-access）
  - context usage ring
- **Skills 与输入增强**：
  - skills menu
  - composer autocomplete：`$skill`、`/prompts:...`、`/review ...`、`@file` token
- **Plan 面板**：每 turn 的计划更新 + interrupt controls。
- **Review**：针对未提交变更、base branch、commits 或自定义指令运行 review。
- **Debug 面板**：warning/error events 查看与导出。
- **用量/额度**：sidebar usage + credits meter（rate limits）。
- **多媒体输入**：图片附件（picker/拖拽/粘贴），并保存 per-thread drafts。
- **可持久化 UI 布局**：面板尺寸、recent activity 等持久化；响应式布局（desktop/tablet/phone）。
- **内置更新器**：toast 驱动的下载/安装。
- **macOS UI 细节**：overlay title bar、vibrancy、reduced transparency toggle。

README 的“Notes”里还有一些容易被忽略但很关键的工程细节：

- 启动/聚焦时会重新连接并刷新 thread list。
- threads 恢复是通过 `thread/list` + `cwd` 过滤实现的；选择 thread 会调用 `thread/resume` 刷新消息。
- Codex sessions 默认使用用户的 Codex home（通常 `~/.codex`）；如果 workspace 内存在 legacy `.codexmonitor/`，则该 workspace 使用它作为 codex home（隔离配置与缓存）。
- worktree agents 的目录存放在 app data directory（`worktrees/<workspace-id>`），并保留对 legacy `.codex-worktrees/` 的兼容。

---

## 2. 架构与关键模块（从代码反推）

### 2.1 Tauri IPC：前端 → Rust backend → codex app-server

README 指明：

- 前端 IPC 映射：`example/CodexMonitor/src/services/tauri.ts`
- Rust commands：`example/CodexMonitor/src-tauri/src/lib.rs`
- 后端与 app-server 交互：`spawn one codex app-server per workspace`，通过 stdio JSON-RPC。

这对我们有直接启示：

- `codex app-server` 是一个稳定的“UI/backend 分离”协议层；
- 我们的 `omne-agent` 若要有 UI/远端控制，优先考虑复用 app-server 风格的 JSON-RPC over stdio/uds/tcp。

补充：CodexMonitor 同时实现了“approval request 的响应”（README 的 Tauri IPC Surface 列出 `respond_to_server_request`），这意味着 UI 端可以像一个“审批终端”一样驱动后端执行。对 `omne-agent` 而言，这个交互模式非常适合做“PR 合并前需要人工确认”的工作流。

### 2.2 Worktree 管理：安全命名 + 唯一路径 + 清理/回填

核心实现集中在 `example/CodexMonitor/src-tauri/src/workspaces.rs`：

- **安全命名**：`sanitize_worktree_name()` 仅允许 `[A-Za-z0-9-_.]`，其它字符替换为 `-`；空结果回退为 `"worktree"`。
- **git worktree add/remove/prune**：全部通过 `tokio::process::Command` 参数数组执行（避免 shell 注入）。
- **父仓库 clean check**：apply 变更前检查 parent repo `git status --porcelain` 必须为空，避免污染主分支工作区。

### 2.3 “回填变更”的补丁策略（非常值得学习）

`apply_worktree_changes()` 的策略值得我们完整吸收：

1. 从 worktree 收集 patch：
   - staged: `git diff --binary --no-color --cached`
   - unstaged: `git diff --binary --no-color`
   - untracked: `git diff --binary --no-color --no-index -- null_device path`
2. 将 patch 通过 stdin 喂给父仓库的 `git apply`：
   - `git apply --3way --whitespace=nowarn -`
3. 对返回信息做分类：
   - applied with conflicts → 提示用户/上层处理冲突
   - partial apply → 提示需要人工处理

这一招的优势：

- 不依赖 `git merge`，可以把 worktree 的“工作副本变更”以 patch 形式移植回父仓库；
- `--3way` 能在一定程度上自动 resolve；
- `--binary` 支持二进制改动（非常关键）。

> 对 `omne-agent` 的价值：我们未来的 `Merger` 可能会遇到“多 PR 并发修改同文件”。即使我们不使用 worktree，也可以用同样的 patch 技术实现“从 task workspace 回填到合并 workspace”。

### 2.4 文件列表：忽略大目录、保持体验

`list_workspace_files_inner()` 使用 `ignore::WalkBuilder`：

- 跳过 `.git/node_modules/dist/target/release-artifacts` 等目录；
- `follow_links(false)` 避免追随 symlink；
- 最后排序输出，便于 UI 搜索与稳定 diff。

---

## 3. 远端后端 POC（daemon）——未来扩展的关键伏笔

`example/CodexMonitor/REMOTE_BACKEND_POC.md` 描述了一个独立 daemon：

- 在 WSL2/Linux 等环境把后端逻辑放到独立进程，通过 TCP 暴露**行分隔 JSON-RPC**。
- 有 token auth handshake（第一条请求必须 `auth`）。
- 支持一批核心方法（workspaces、threads、reviews、models、skills、respond_to_server_request 等）。

> 对 `omne-agent` 的启示：我们如果要“hook 回主流程/远端编排/集群 worker”，强烈建议从一开始就把 orchestrator 的控制面做成可远端的协议（最小实现可先 HTTP webhook，再升级成 JSON-RPC/GRPC）。

---

## 4. CodexMonitor 的“特色/巧思”总结

1. **把 `codex app-server` 当作稳定后端协议**：UI 只需要实现 JSON-RPC 客户端，不必关心模型细节。
2. **worktree 管理工程化**：命名、路径、清理、以及“把变更回填”的完整闭环。
3. **强 UX 意识**：context usage ring、plan panel、draft 持久化、分面板布局、debug 面板等，都是“长对话 + 高并发事件流”的刚需。
4. **daemon POC 提前布局**：把“桌面 app 专属后端”抽象成可远端协议，扩展空间巨大。

---

## 5. 对 `omne-agent` 的可复用建议（按优先级）

### P0（立刻可用）

- 复用 `apply_worktree_changes` 的 patch 思路作为我们 `Merger` 的“冲突缓解手段”之一。
- 复用“安全命名 + 参数化 Command”的安全习惯（避免 shell 拼接）。

### P1（需要结合 codex 魔改）

- 把 `codex app-server` 作为 worker 运行时的统一接口：每个 task workspace 启动一个 app-server（或复用内嵌库），由 orchestrator 汇聚事件与状态。
- 引入 “daemon 控制面” 作为未来多端 UI/CI 的稳定接口（先 POC，后产品化）。
