# 1Code（example/agent-gui/1code）能力与设计分析

> Snapshot: `example/agent-gui/1code` @ `e23a4695d5ee`
>
> 结论先行：1Code 的核心不是“又一个聊天 UI”，而是把 **Claude Code 的执行**放进一个可控的桌面工作台：每个会话一个 worktree、可后台运行、实时 diff/工具可见，并且把“创建 worktree 后要做的脏活（复制 env/装依赖/起服务）”做成**可版本化的配置文件**。对 `omne-agent vNext` 来说，它最值得抄的是：**工作区生命周期脚本化（setup-worktree）+ plan/agent 模式硬切换 + 把 side effects 做成可观察事件**；而不是它的内建 git client。

---

## 1) 产品定位（从 README 看它卖什么）

`example/agent-gui/1code/README.md` 的核心卖点很直接：

- **Git Worktree Isolation**：每个 chat session 一个 worktree，不碰 `main`。
- **Background Execution**：agent 后台跑，你可以切走干别的（这就是 RTS “多单位同时行动” 的基本盘）。
- **Diff Previews / Tool visibility**：实时看到 Claude 的 bash/file/web 工具行为与 diff（可观测性优先）。
- **Plan mode**：先澄清问题 → 结构化 plan → 预览 → 再执行（把“先想清楚”变成硬门槛）。
- **Local-first**：不把代码同步到云端。

对 `omne-agent` 的启示：你要做 RTS 风格，核心不是并发数，而是**注意力管理**：让用户知道“哪个 agent 在干嘛、哪里需要我介入”，并且任何落盘/命令执行都能追溯。

---

## 2) Worktree 配置：把“环境特殊情况”外包给文件

### 2.1 配置文件路径与兼容性

1Code 明确支持两套路径（可视为“兼容 Cursor 的 worktree 约定”）：

- `.1code/worktree.json`
- `.cursor/worktrees.json`

实现证据：`example/agent-gui/1code/src/main/lib/git/worktree-config.ts`。

它的 detection 优先级也写死了（别靠 UI/用户记忆）：

1. custom path（如果传入）
2. `.cursor/worktrees.json`
3. `.1code/worktree.json`

### 2.2 配置 schema（只做一件事：worktree setup）

核心 keys：

- `setup-worktree`（跨平台优先）
- `setup-worktree-unix`
- `setup-worktree-windows`

实现证据：`example/agent-gui/1code/src/main/lib/git/worktree-config.ts` 中 `WorktreeConfig` + `getSetupCommands`。

### 2.3 执行模型：创建 worktree 后跑命令

`executeWorktreeSetup(worktreePath, mainRepoPath)` 会：

- 读取主 repo 下的 worktree config（detect）
- 选择当前平台对应的 commands
- 在新 worktree 的 `cwd` 里逐条 `exec` 执行
- 注入 `ROOT_WORKTREE_PATH` 环境变量（让脚本知道“根目录在哪”）
- 单条命令超时 5 分钟；失败不会立刻中止（继续执行后续命令）

实现证据：`example/agent-gui/1code/src/main/lib/git/worktree-config.ts`。

对 `omne-agent`：你不需要发明复杂 DSL。**一个 repo 内可版本化的 JSON + “固定生命周期钩子”**就能干掉 80% 的“并发 worktree 开发的环境脏活”。

---

## 3) “安全与好品味”：把危险 API 关进小盒子

1Code 有一套很值得学习的“语义化 git helpers”（避免把 `git checkout -- <path>` 当成切分支）：

- 分支切换使用 `git switch`，并对旧 git fallback（`git checkout <branch>`）。
- 文件 checkout 明确使用 `git checkout -- <path>`（`--` 语义固定）。
- `git add -- <path>` / `git reset HEAD -- <path>` 这类接口统一加 `--`，避免路径被当成 flag。

实现证据：`example/agent-gui/1code/src/main/lib/git/security/git-commands.ts`。

对 `omne-agent`：这类 wrapper 的价值不是“封装 git”，而是**把可被注入/误用的命令面缩到最小**。对应到 vNext，我们的核心执行层也应当如此：把工具执行变成“语义化工具 + 强类型输入”，而不是任意字符串 shell。

---

## 4) 对 omne-agent vNext 的启示（只取精华）

1. **把 worktree/workspace lifecycle 规范化**：至少要有 `setup` hook（类似 1Code 的 `setup-worktree`）。
2. **plan/agent 模式必须是硬门槛**：默认先 plan，再允许执行 side effects（RTS 控制台要能“暂停/步进”）。
3. **可观测性是产品核心**：diff + 工具执行 + 文件写入必须进入事件流（UI/daemon 只是事件消费者）。
4. **安全边界要“结构化”**：不要把安全寄托在 prompt；把危险能力关进小 API（类似 `git-commands.ts` 的做法）。

