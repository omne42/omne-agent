# Codex PM（Rust）实现计划：本地 Git 服务 + 并发 AI 任务流水线

> 目标：用 Rust 构建一个“自包含”的 Agent 系统，覆盖代码全生命周期（任务拆分→并发开发→校验→提交 PR→AI 合并），重点解决：**/tmp 临时目录隔离**与**并发执行多个 AI task**。

---

## 0. 背景与设计来源（我们要“学什么”）

我们将显式借鉴下列项目的结构化思路（不照搬实现，抽象成可复用的模块边界）：

- `codex-rs`：线程/会话管理（`ThreadManager`）、广播事件、资源生命周期管理、模型/技能管理分层。
- `opencode`：Session → Message → Part 的可追溯记录结构、Storage 抽象、事件总线（Bus）驱动的状态流转。
- `claude-code`：插件/工作流/Hook/权限 guardrails（commands as markdown + allowed-tools）、并行专项 review。
- `CodexMonitor`：`codex app-server` 客户端编排、多 workspace、多 worktree、patch 回填策略、远端 daemon POC。
- `kilocode`：fork 合并策略与“变更 marker”体系、mode 权限模型与 orchestrator workflow 写法。

这些经验会落到两个约束上：

1. **高内聚**：核心域（Session/Task/PR/Repo）不依赖外部实现细节（Git/LLM/HTTP/DB）。
2. **低耦合**：外部依赖（git CLI、HTTP server、LLM provider、GitHub API）全部通过 trait + adapter 接入，可替换、可测试。

### 0.1 调研文档索引（本仓库已沉淀）

所有调研结论与可复用点收录在：

- `docs/research/README.md`
- `docs/research/codex.md`（本期主底座：Responses API + sandbox/approvals/app-server）
- `docs/research/opencode.md`（Storage/Bus/Worktree/instance state/migrations）
- `docs/research/claude-code.md`（workflows + hook/guardrails）
- `docs/research/codexmonitor.md`（worktree & patch 回填、远端 daemon）
- `docs/research/kilocode.md`（fork markers + mode 权限）

### 0.2 关键战略：基于 Codex 魔改（Rust-only，Responses-only）

我们不从零写一个“AI coding agent”，而是**以 `example/codex` 为主底座做 fork/复制式开发**：

- 复用 `codex-rs` 的：
  - Responses API 客户端与 SSE 事件解析（`codex-api`）
  - Thread/Turn/Item 事件模型与 app-server 协议（可被 UI/daemon 驱动）
  - sandbox/approvals/execpolicy/exec-server（命令执行的安全底座）
  - notify hook（turn-level 完成回调能力，未来扩展为 session/task/pr 级）
- 第一阶段只支持 **Responses API**：
  - 默认 `wire_api = "responses"`
  - 不新增 chat/completions 兼容逻辑（避免未来合并成本）
- 我们新增的差异化能力聚焦：
  - 本地 bare repo 托管 + repo 注入
  - `/tmp/{repo}_{session}/tasks/{task}` 并发 workspace 编排
  - fmt/check/test/commit/push/“本地 PR”元数据
  - AI 合并多个 PR（Merger Agent）
  - 完成时 hook 回主流程（HTTP webhook / command），并与 Codex notify 统一

### 0.3 Fork 可维护性：引入变更 markers（借鉴 kilocode）

为了未来持续跟进 codex 上游，我们需要从 Day 1 引入 marker 体系（类似 kilocode 的 `kilocode_change`）：

- `codex_pm_change`（单行）/ `codex_pm_change start/end`（多行）
- 新增文件统一标注（可选）
- 仅对“来自上游 codex 的共享文件”强制 marker；我们自有目录不强制

这会显著降低未来 rebase/merge 上游时的冲突成本，并让审阅者快速定位“我们魔改了什么”。

### 0.4 上游复用规则（允许 copy，但禁止引用依赖）

我们会从 `example/` 中学习并“复制式实现”，但最终产物必须与 `example/` **没有任何依赖关系**（禁止 `Cargo.toml` path/git 依赖、禁止运行时要求 `example/` 存在等）。

详见：`docs/upstream_reuse_policy.md`。

---

## 1. 需求拆解（本期必须实现的能力）

### 必须（MVP）

1. **创建本地 Git 服务**
   - 能承载 bare 仓库（作为“中心仓库”）。
   - 能让 worker 将分支推送回去（最小可用：本地 file remote；增强：Smart HTTP）。
2. **注入仓库（Repo Injection）**
   - 将任意 repo（本地路径或远端 URL）注入/镜像到本地 bare repo。
   - 形成稳定的 repo 名称（`repo_name`）用于后续 session 引用。
3. **并发处理多个 AI task**
   - 一次输入：`prompt（规范/需求/目标）` + `pr_name`。
   - `Architect` 负责拆分 tasks；多个 `Coder` 并发在 `/tmp/{repo_name}_{session_id}/...` 开发。
   - 每个 task 结束：`fmt` + `check` + `commit`，并形成一个 PR（本地 PR 模型）。
4. **用 AI 服务合并多个 PR（Merger Agent）**
   - 对多个 PR 做排序、冲突预判、必要时自动补丁修复，然后合并到目标分支。
5. **完成时 hook 回主流程**
   - 至少支持：回调 HTTP webhook 或执行本地命令（二选一即可，建议两者都做 trait）。

### 暂缓（明确不做 / 后续）

- 真正的 GitHub PR（可做适配层，但 MVP 先实现本地 PR 模型）。
- 多机分布式执行（先单机并发）。
- React UI 包：仅放占位与接口预留。

---

## 2. 核心概念模型（Domain）

### 2.1 实体与关系

- `Repository`：被注入并托管的代码仓库（对应一个 bare repo）。
- `Session`：一次端到端运行（一次 prompt + pr_name 的执行单元）。
- `Task`：`Architect` 拆分出的最小开发单元，可并发执行。
- `PullRequest`（本地 PR）：一个 task 对应一个分支 + 元数据（标题/描述/状态/校验结果）。

关系：

- `Repository 1 ── N Session`
- `Session 1 ── N Task`
- `Task 1 ── 1 PullRequest`

### 2.2 输入/输出契约（最小 API）

```rust
/// 外部调用只需要两个核心字段：prompt + pr_name。
/// repo 可以来自“当前工作目录绑定的默认 repo”或配置，也可作为可选字段提供。
pub struct RunRequest {
    pub repo: Option<String>,   // repo_name 或 URL/path（可选，MVP 允许配置默认值）
    pub pr_name: String,        // 用户给的这次变更名称（会作为 PR 前缀）
    pub prompt: String,         // 规范 + 需求 + 目标 + 约束
    pub hook: Option<HookSpec>, // 完成后回调
}
```

输出（对外）：

- `session_id`
- 生成的 PR 列表（含分支名、状态、校验结果、diff 摘要）
- 合并结果（成功/失败、冲突点、修复 commit）

---

## 3. 临时目录与并发模型（本项目重点）

### 3.1 目录约定

所有开发发生在：

```
/tmp/{repo_name}_{session_id}/
  session.json
  logs/
  tasks/
    {task_id}/
      repo/               # task 独立工作副本（避免并发写同一 .git 元数据）
      artifacts/          # fmt/check 输出、patch、诊断信息
```

关键决策：**每个 task 使用独立 clone**（从本地 bare repo 克隆），而不是共享 worktree。

- 优点：并发安全（避免 `git worktree` 并发写 `.git/worktrees` 元数据的锁竞争）。
- 成本：磁盘/时间增加，但本地 clone 通常很快；后续可优化为 `--shared` / `gix` 直出对象。

### 3.2 并发调度

使用 Tokio：

- `tokio::task::JoinSet` 管理 worker 生命周期。
- `tokio::sync::Semaphore` 控制最大并发（默认 `N = num_cpus` 或配置）。
- `tokio::sync::watch` / `broadcast` 推送状态变更（供 CLI/UI/Hook 订阅）。

并发安全边界：

- `RepoInjection`：同一 repo 的注入/拉取必须串行（`RepoLock(repo_name)`）。
- `Merge`：合并动作必须串行（同一 repo、同一目标分支）。
- `Task`：task 目录完全隔离，可完全并发。

---

## 4. Git 层：本地 Git 服务与 PR 模型

### 4.1 Bare Repo 托管（中心仓库）

目录（默认）：

```
.codex_pm/
  repos/
    {repo_name}.git/      # bare repo（中心）
  data/                   # 元数据（session/task/pr）
  locks/                  # 文件锁（repo 注入/合并）
```

### 4.2 “本地 Git 服务”的两种形态（分阶段）

**Phase 1（最先落地）**：不做网络服务，直接使用本地路径作为 remote

- Worker push：`git push /abs/path/to/.codex_pm/repos/{repo}.git <branch>`
- 优点：实现简单、稳定。

**Phase 2（增强）**：提供 Smart HTTP（git clone/push over http）

- Rust `axum`/`hyper` Server + 子进程 `git http-backend` 作为实现（最省力、兼容 git 协议）。
- 仅绑定 `127.0.0.1`，默认无鉴权（后续可加 token）。
- 同时提供 JSON API（PR 列表、session 状态等）。

> 纯 Rust 实现 Git Smart HTTP 协议工作量巨大，不建议自研；优先复用 git 官方后端。

### 4.3 本地 PR 的定义

本地 PR = 分支 + 元数据：

- `head`：`refs/heads/ai/{pr_name}/{task_id}`
- `base`：`refs/heads/main`（可配置）
- `status`：`Draft | Ready | Merged | Failed`
- `checks`：fmt/check/test 结果、日志路径、关键诊断摘要

“提交 PR”在 MVP 中指：

1. 推送分支到本地 bare repo。
2. 写入 PR 元数据到 storage（可被查询、可被 merger 使用）。

---

## 5. Agent 体系：我们自己的 Agent（角色与职责）

### 5.1 角色矩阵（目标形态）

- `Ideator（构思者）`：把用户 prompt 变成可执行的产品/工程方案摘要（澄清范围、验收标准、风险点）。
- `IdeaCritic（构思审核者）`：质疑必要性、实现难度、竞品/可借鉴点、建议采用的社区库/基建方案。
- `Architect（架构师）`：把目标拆成任务（含依赖关系 DAG）、定义每个 task 的输入/输出、并发边界。
- `Coder（开发者，N 个）`：在 task workspace 实施变更，跑 fmt/check，commit，生成 PR。
- `FrontendStylist（前端美化师）`：仅在存在前端时介入，聚焦 CSS/视觉（MVP 可不启用）。
- `Reviewer（Review 师）`：对 PR 做静态审阅（风格/可维护性/安全/边界条件），给出修订建议或阻断合并。
- `Builder（构建部署师）`：负责 CI/CD、发布、运维脚本（MVP 可不启用）。
- `Merger（合并器）`：综合多个 PR，决定合并策略与顺序，必要时自动修冲突并二次校验。

### 5.2 最小落地顺序（必须先单进程骨架）

为了快速落地并保持高内聚低耦合，采用“逐步替换智能”的策略：

1. Phase 1：`Architect` 先用规则/模板拆分（可 mock），`Coder` 单线程跑通端到端。
2. Phase 2：引入并发 `Coder`（真正的并行开发）。
3. Phase 3：引入 `Architect` 的 AI 拆分（DAG/拓扑排序）。
4. Phase 4：引入 `Merger` AI 合并与冲突修复。
5. Phase 5：补齐 `Ideator/IdeaCritic/Reviewer/Builder/FrontendStylist`，形成全生命周期流水线。

### 5.3 Agent 的统一接口（建议）

```rust
#[async_trait::async_trait]
pub trait Agent<I, O>: Send + Sync {
    async fn run(&self, ctx: AgentContext, input: I) -> anyhow::Result<O>;
}

pub struct AgentContext {
    pub session: Session,
    pub repo: Repository,
    pub event_bus: EventBus,
    pub storage: Arc<dyn Storage>,
    pub ai: Arc<dyn AiProvider>, // 可在 Phase 1 用 MockAiProvider
}
```

---

## 6. 端到端流程（从输入到 hook）

### 6.1 单进程骨架（Phase 1 必须先实现）

```
run(request):
  repo = ensure_injected(request.repo)
  session = create_session(repo, request.pr_name, request.prompt)
  tasks = architect.split(request.prompt)   // 先规则化拆分

  for task in tasks:                        // 串行
    pr = coder.execute(task):
      clone bare -> /tmp/{repo}_{session}/tasks/{task}/repo
      apply changes (AI or scripted)
      cargo fmt + cargo check
      git commit + push branch
      write PR metadata
  merged = merger.merge_all(prs)            // 先简单顺序合并（无 AI）
  dispatch_hook(session, merged)
```

Phase 1 的验收：哪怕没有“聪明”的 AI，也能把一个任务跑完整链路，并留下可追溯的 session/task/pr 记录。

### 6.2 并发 worker（Phase 2）

- `Architect` 输出 `TaskSpec` 列表（后续扩展为 DAG）。
- Orchestrator 使用 `JoinSet` 并发执行 `Coder`。
- 中央 `EventBus` 汇总状态；CLI/UI/Hook 订阅。

---

## 7. 存储、事件与可恢复性

### 7.1 Storage 抽象

MVP 使用文件存储（JSON），后续可替换 SQLite/RocksDB。

```rust
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    async fn put_json(&self, key: &str, value: &serde_json::Value) -> anyhow::Result<()>;
    async fn get_json(&self, key: &str) -> anyhow::Result<Option<serde_json::Value>>;
    async fn list_prefix(&self, prefix: &str) -> anyhow::Result<Vec<String>>;
}
```

### 7.2 EventBus（面向 hook/UI/日志）

事件包括：

- `SessionCreated/Updated`
- `TaskStarted/Progress/Completed/Failed`
- `PrCreated/Updated/Merged`
- `MergeStarted/Completed/Failed`

要求：

- 事件可序列化（便于落盘/回放）。
- session 可恢复：重启后从 storage 读取未完成任务并继续（Phase 3+）。

---

## 8. 关键 crate 选型（避免重复造轮子）

建议依赖（按层次）：

- 基础：`tokio`, `anyhow`, `thiserror`, `serde`, `serde_json`, `uuid`, `time`, `tracing`
- CLI：`clap`
- HTTP Server（可选）：`axum`, `tower`, `hyper`
- 文件锁：`fs2` 或 `fd-lock`（跨进程锁）
- Git：
  - MVP：调用 `git` CLI（最稳，覆盖所有仓库形态）
  - 增强：`gix`（纯 Rust git 实现，适合做更深度操作；但 MVP 不强依赖）
- LLM Provider：
  - 第一阶段：直接复用 `codex-rs` 的 `codex-api`（Responses-only），避免自写 OpenAI client 与 SSE 解析
  - 未来扩展：再引入 provider 抽象/路由（可参考 `claude-code-router` 的 transformers 思路）
- Codex 复用（强烈建议）：
  - `codex-core`（业务与会话/工具编排）
  - `codex-protocol`（Thread/Turn/Item 类型 + app-server types）
  - `codex-api`（Responses API + SSE）
  - `codex-execpolicy` / `codex-exec-server`（命令执行审批/策略）
  - `codex-responses-api-proxy`（隔离密钥场景备用）

---

## 9. Rust 工程结构（建议的 workspace）

推荐结构：**基于 codex fork 的 workspace 增量扩展**（Rust-only）。

```
codex/                          # 我们的 codex fork（或 vendor）
  codex-rs/                     # 上游主体（尽量少改）
    ...                         # 原有 crates
    crates/                     # ✅ 新增：codex_pm 相关 crates（与上游隔离）
      pm-core/                  # domain + orchestration（复用 opencode 的 Storage/Bus 思路）
      pm-git/                   # repo injection / bare repo / push / merge helpers
      pm-agents/                # 角色实现（Architect/Coder/Reviewer/Merger/…）
      pm-cli/                   # `codex pm ...` 或独立 `codex-pm` CLI
      pm-server/                # 可选：HTTP/JSON-RPC 控制面（未来 UI/CI/远控）
  docs/
    implementation_plan.md
    research/
```

强约束：

- 新增 `pm-*` crates 尽量不侵入上游 `codex-*` crates；若必须改上游，使用 `codex_pm_change` markers 标注。
- `pm-core`（domain）不直接依赖网络/子进程；IO 通过 trait 由 `pm-git/pm-server/pm-agents` 实现。
- 所有 LLM 调用第一阶段统一走 `codex-api`（Responses-only），避免引入第二套 client。

---

## 10. 里程碑（按“先骨架后并发再智能”推进）

### Phase 1：单进程骨架（必须先做）

- Repo 注入：`inject(repo_src) -> repo_name`
- Session 创建：写入 `session.json`
- 规则化拆分 task（先固定 1~3 个 task）
- 单 worker 执行：clone → 修改（可先占位）→ fmt/check → commit → push → 写 PR 元数据
- Merger：顺序合并（无冲突处理，只做 fast-forward / squash 的最小实现）
- Hook：完成回调（webhook 或命令）

### Phase 2：并发 Worker（项目重点落地）

- `JoinSet + Semaphore` 并发执行多个 task
- 事件总线 + 状态订阅（CLI 能实时看到进度）
- 失败隔离：单 task 失败不阻塞其它 task；最终合并策略可配置（best-effort / fail-fast）

### Phase 3：智能 Architect + Reviewer

- AI 拆分 task，输出 DAG（含依赖、文件范围、风险级别）
- Reviewer agent 对 PR 做合并前审核（阻断/建议修订）

### Phase 4：智能 Merger（多 PR 合并与冲突修复）

- AI 选择合并顺序、必要时自动 rebase/resolve
- 合并后全量校验（fmt/check/test）

### Phase 5：Builder + UI 占位

- Builder：部署/运维脚本、可选的 CI 集成
- UI：仅做“接口占位与 API contract”（未来可选的 React UI 组件库不在本期实现；本仓库保持 Rust-only）

---

## 11. 风险与对策（提前设计）

- 并发 git push/refs 锁冲突：必要时对 push 加 repo 级互斥锁。
- 任务拆分不合理：先规则化拆分 + 小步迭代，后续再引入 AI DAG。
- 命令注入/路径穿越：repo_name/task_id 必须 sanitize；所有 `Command` 使用参数数组，不拼 shell 字符串。
- 长时间任务/资源泄露：session/task 超时、并发上限、临时目录清理策略（保留失败现场）。
- 非标准提交/不通过校验的提交污染主分支：强制 Conventional Commits + pre-commit 运行 `cargo fmt --check` 与 `cargo check --all-targets`（见 `docs/commit_policy.md` 与 `githooks/`）。

---

## 12. 下一步（本仓库的行动项）

1. 在 Phase 1 先实现“可运行骨架”（哪怕 AI 先 mock），确保注入→临时目录→fmt/check→commit→PR→merge→hook 全链路可演示。
2. 进入 Phase 2，把 worker 改为真正并发，并固化 `/tmp/{repo}_{session}` 目录规范与事件订阅。
