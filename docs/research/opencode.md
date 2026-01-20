# OpenCode（example/opencode）能力与设计分析

> Snapshot: `example/opencode` @ `3fd0043d1`
>
> 结论先行：OpenCode 的定位是“开源 AI coding agent”，强调 **provider-agnostic**、**TUI-first**、**client/server**、**内置 LSP**。对 `CodePM` 来说，它最值得学习的是“把 agent 做成一个真正的软件系统”的工程化：**Project/Session/Message/Part 的数据建模、文件存储抽象与迁移、instance-scoped state、事件总线、worktree 生命周期与命名策略**。

---

## 1. 仓库结构与产品面

### 1.1 顶层定位（README）

`example/opencode/README.md` 与多语言 README 强调：

- 100% 开源
- 不绑定 provider（Claude/OpenAI/Google/本地模型都可）
- out-of-the-box LSP
- 强调 TUI 体验（neovim 用户背景）
- client/server 架构：TUI 只是一个 client，未来可移动端远控

### 1.2 Monorepo packages（按目录名）

通过读取 `example/opencode/packages/*/package.json`，当前 packages 包含：

- `packages/opencode`：核心 CLI/Server（JS/Bun）
- `packages/desktop`：Tauri 桌面 app
- `packages/web`：Web 前端（可能用于 landing/console）
- `packages/ui`：UI 组件库
- `packages/util`：通用工具（错误类型、日志等）
- `packages/plugin`：插件体系
- `packages/slack`：Slack bot（thread → session）
- `packages/function` / `packages/enterprise` / `packages/console` / `packages/docs`：产品化扩展与文档

> 注：部分 package README 是模板文本，但 `packages/opencode/src` 目录非常完整，足以做工程级调研。

---

## 2. “所有能力”盘点（从 `packages/opencode/src/index.ts` 反推 CLI 面）

`example/opencode/packages/opencode/src/index.ts` 注册了大量命令（yargs）：

- `run`：执行/启动 agent 会话
- `generate`
- `debug`
- `auth`
- `agent`
- `upgrade` / `uninstall`
- `serve`：启动 server
- `web`
- `models` / `stats`
- `export` / `import`
- `github`
- `pr`
- `session`
- `mcp`
- `acp`
- `tui thread` / `attach`（与 TUI 相关）

这说明 OpenCode 把能力拆成两个层次：

1. CLI 命令面（开发者/用户入口）
2. 内部模块（session、storage、bus、permission、tool、worktree、provider…）

### 2.1 源码目录能力面（以 `packages/opencode/src/*` 为证）

`example/opencode/packages/opencode/src/` 下的模块目录非常“全家桶”，从目录名就能看出它覆盖的系统面：

- `agent/`：多 agent/角色与运行时（对应 build/plan/general 等）。
- `server/`：服务端（配合 client/server 架构）。
- `session/`、`project/`、`share/`：会话与共享机制（与 `specs/project.md` 对齐）。
- `storage/`：持久化（含迁移）。
- `bus/`：事件总线。
- `permission/`：权限/审批（对应 agent 执行工具时的安全边界）。
- `tool/`、`shell/`、`pty/`：工具执行与终端交互。
- `worktree/`：git worktree 生命周期管理。
- `mcp/`、`plugin/`、`skill/`：扩展生态（工具/插件/技能）。
- `lsp/`、`ide/`：编辑器/LSP 集成（提升代码理解与导航）。
- `snapshot/`、`patch/`、`format/`：变更生成、格式化、快照/回滚（对长任务很关键）。
- `scheduler/`：调度/并发/节流（对应“多任务并行”的系统需求）。

> 对 `CodePM`：这份目录结构可以视为我们未来“全生命周期系统”理想形态的一个参考原型（即使我们不抄代码，也应该抄“边界怎么拆”）。

---

## 3. 核心域建模：Project/Session（与我们高度同构）

`example/opencode/specs/project.md` 直接给出了 API 设计草案（极具参考价值）：

- `Project[]` 管理
- `Session[]` 管理：创建/删除/init/abort/share/compact/revert/permission 等
- `Message` 与 `Part` 列表与详情
- 文件查找/读取、状态查询
- provider/config/agent 等“解析当前目录上下文”API

> 对 `CodePM`：我们提出的“Repo → Session → Task/PR”与 OpenCode 的“Project → Session → Message/Part”是同构的。我们可以直接借鉴其 API 粒度与资源路径风格。

---

## 4. Storage：文件存储抽象 + 锁 + 迁移（强烈建议学习）

核心实现：`example/opencode/packages/opencode/src/storage/storage.ts`

### 4.1 关键能力

- **以 `key: string[]` 表示资源路径**：最终落到 `.../<key...>.json`，非常适合层级数据模型。
- **读写锁**：通过 `Lock.read/Lock.write` 在文件级做并发控制（避免并发写坏 JSON）。
- **`list(prefix)`**：用 glob 扫描列出所有键（便于枚举 session/message）。
- **错误类型化**：ENOENT → `NotFoundError`（对 API 层更友好）。

### 4.2 Migration（一个经常被忽略，但决定长期可维护性的点）

Storage 内置 `MIGRATIONS` 队列：

- 通过一个 `migration` 文件记录当前迁移版本号。
- 每次启动时补跑未执行迁移，并写回版本号。
- 迁移逻辑可以非常“工程化”：复制旧目录结构、重算 projectID、拆分/重写 session summary 等。

> 对 `CodePM`：我们未来一定会扩展 session/pr/task 的 JSON schema。OpenCode 的 migration 机制非常值得从 Day 1 引入，否则后期会被历史数据拖死。

---

## 5. Bus/Event：schema 化事件 + instance-scoped 生命周期

核心实现：

- `example/opencode/packages/opencode/src/bus/bus-event.ts`
- `example/opencode/packages/opencode/src/bus/index.ts`
- `example/opencode/packages/opencode/src/project/instance.ts`

### 5.1 事件定义（强类型）

- 用 `BusEvent.define(type, zodSchema)` 注册事件，并能生成 `discriminatedUnion` 的 payload schema。
- 优点：事件不只是字符串；它带 schema，可用于：
  - API contract
  - runtime validation
  - 文档/代码生成

### 5.2 发布/订阅模型

- 订阅支持具体 type 与 `*` 通配符。
- `publish` 同时向 instance-local subscribers 发送，并通过 `GlobalBus.emit` 广播跨 instance 事件。

### 5.3 Instance：以“目录”为隔离边界的上下文与状态

`Instance` 的几个关键点非常值得我们复刻：

- **按 directory 缓存实例**：同一目录复用 instance context。
- `Instance.provide({directory, init, fn})`：类似动态作用域/上下文注入。
- `Instance.state(init, dispose)`：为每个 directory 创建 state，并在 dispose 时触发事件。
- `containsPath()`：判断路径是否在 project/worktree 边界内，避免“worktree= / 导致所有路径都被认为在边界内”的漏洞（安全细节很到位）。

> 对 `CodePM`：我们也需要 `RepoContext`/`SessionContext` 以及“按 repo 锁、按 session 隔离”的状态管理。Instance 模式能让业务代码不必处处传递 directory/worktree。

---

## 6. Worktree：自动命名 + 唯一性校验 + startCommand（值得直接借鉴）

核心实现：`example/opencode/packages/opencode/src/worktree/index.ts`

### 6.1 命名策略（体验与冲突控制）

- 自动生成 `adjective-noun`（例如 `nimble-rocket`），并支持基于输入 name 的 slug 化。
- 为保证唯一性：
  - 检查目录是否存在
  - 检查 git refs `refs/heads/opencode/<name>` 是否已存在
- 尝试多次（最多 26 次）仍失败则抛出 `NameGenerationFailedError`

### 6.2 生命周期

提供：

- create（`git worktree add -b <branch> <dir>`）
- remove（基于 `git worktree list --porcelain`）
- reset（推测是清理/重建，源码后半段可补齐）
- 可选 `startCommand`：worktree 创建后自动执行启动命令（跨平台：win32 用 `cmd /c`，其它用 `bash -lc`）

> 对 `CodePM`：我们当前更偏向 `/tmp/{repo}_{session}/tasks/{task}/repo` 的 clone 隔离，但 worktree 的“快速创建/低成本/可回填”依然非常有价值；尤其未来我们想做“同 repo 多并发分支”的长运行服务时。

---

## 7. OpenCode 的“特色/巧思”总结

1. **工程系统意识强**：Storage migration、instance 生命周期、事件 schema 化，这些都是“长期维护”必备。
2. **边界与权限**：`containsPath` 的边界判断反映其对安全/权限的现实考虑（worktree 与非 git 项目差异）。
3. **Worktree 体验细节**：自动命名、唯一性校验、可选 startCommand —— 这会显著提升“多任务并行/多 workspace”的易用性。
4. **client/server 视角**：即使我们暂时不做分布式，也应该把控制面与执行面分离，为未来扩展留接口。

补充：`example/opencode/specs/*` 里还有一组“性能与可维护性 roadmap/spec”，非常值得学习其写法（先加 guardrails/flags、再逐步优化）：

- `specs/01-persist-payload-limits.md`：持久化 payload 限制
- `specs/02-cache-eviction.md`：缓存淘汰
- `specs/03-request-throttling.md`：请求节流/去抖
- `specs/04-scroll-spy-optimization.md`：长会话滚动优化
- `specs/05-modularize-and-dedupe.md`：模块化与去重
- `specs/perf-roadmap.md`：分阶段交付与 PR 切分策略（强烈建议我们学习这种“可演进计划”的写法）

---

## 8. 对 `CodePM` 的具体借鉴清单（可落地）

### P0（建议直接照搬/翻译到 Rust）

- Storage：`key[] -> file.json` + 文件锁 + list(prefix) + migration 版本文件。
- EventBus：强类型事件 + 通配订阅 + session/repo 生命周期事件。
- Instance/Context：按 repo/session 的上下文注入 + state create/dispose 钩子。

### P1（结合我们当前目标“临时目录并发 + PR 流水线”）

- Worktree 命名与唯一性校验的策略（可用于分支名生成、PR 名冲突避免）。
- API 粒度（`specs/project.md`）可作为我们 future HTTP API 的雏形。

### P2（未来）

- client/server：把 orchestrator 做成 daemon，CLI/TUI/Web 作为 client。
- LSP 集成：未来 `Reviewer`/`Coder` 可以利用 LSP 提升 patch 质量（但不应成为 MVP 阻塞项）。
