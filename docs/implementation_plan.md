# CodePM vNext（Rust）实现计划：Agent CLI + RTS 风格控制面

> 现状：旧实现已存档为 `v0.1.1`（git tag）。`vNext` 重新设计的第一优先级是：**能写代码且能与环境交互的 Agent CLI**；Git/PR 只是可插拔交付适配层。
>
> `v0.2.0` 阶段目标：先完成 “Codex 功能对齐 + 跨项目精华 TODO”（见 `docs/v0.2.0_parity.md`），再叠加 CodePM 的并发编排与交付适配。

---

## 0) 核心判断（先把路走对）

**核心判断：** 值得做。原因：没有“可控执行 + 可观测事件 + 可回放存储”的底座，多 agent 并行（RTS）只会把混乱放大。

**关键洞察：**

- **数据结构与所有权：** UI/daemon 只是投影；唯一真相是 `Thread/Turn/Item`（事件流）+ `Workspace` 生命周期 + `Artifacts` 索引。
- **复杂性：** 并发不是难点，难点是“并发 + 环境隔离 + 可审计 + 可收口”。用脚本化 lifecycle + checklist gate 消灭特殊情况。
- **风险点：** 本地全权限执行（读写文件/跑命令/联网）是默认形态。必须在 Day 1 把 `sandbox/approvals/execpolicy` 做成系统能力，而不是 prompt 里的一句提醒。

---

## 1) 目标与非目标

### 1.1 必须（MVP）

1. **Responses-only**：第一阶段只支持 OpenAI `POST /v1/responses`。
2. **可控执行层**：shell/file/network 都走统一 tool runtime，具备 `Run/Deny/Escalate` 的审批语义。
3. **事件化与回放**：所有 side effects 进入事件流并落盘，能 `list/show/export/replay`。
4. **并发与隔离**：多 task 并发时，每个 task 有独立 workspace（先 `/tmp`，后续可扩展为 worktree 等目录级隔离方案）。
5. **workspace 生命周期脚本化**：至少支持 `setup/run/archive(或 teardown)` 三段，并落盘 stdout/stderr。
6. **RTS 控制面最小集**：`pause/resume/interrupt/cancel` + “Attention/Inbox”视图（列出需要人介入的点）。
7. **交付通道（先不绑定 git）**：默认 `patch-only`（产出可应用 patch + artifacts），git 分支交付放后面。
8. **中间态 artifacts（必须）**：stdout/stderr/plan/diff/test progress 必须边产出边落盘，并能随时查询/预览。
9. **进程可观测（必须）**：后台命令与多子 agent 的运行态必须可随时 inspect/attach/kill（事件化、可审计）。

### 1.2 暂缓（明确后做）

- GitHub PR / merge 自动化（先做 adapter，不要污染核心域）。
- 桌面 GUI（先把控制协议与事件模型钉死，GUI 只是 client）。
- 分布式多机执行（先单机并发）。

---

## 2) 关键设计来源（我们抄什么作业）

### 2.1 执行底座：Codex（主底座）

来自：`docs/research/codex.md`

- `Thread/Turn/Item` 事件模型与 app-server 协议（JSON-RPC + 事件流）。
- `sandbox/approvals/execpolicy/exec-server`：把“命令执行”收敛成可审计的系统能力。
- Responses API 客户端与 SSE 事件解析：错误类型化、token usage/rate limits 一等化。
- 输出约束：JSON schema 生成与校验接口（减少“靠 prompt 正则”）。

### 2.2 存储与迁移：OpenCode

来自：`docs/research/opencode.md`

- 文件存储抽象 + 锁 + migrations（不然历史数据会拖死你）。
- Session/Message/Part 的可追溯结构（我们对应到 Run/Turn/Item）。

### 2.3 工作区生命周期脚本：1Code / Superset（爆发期产品的共识）

来自：`docs/research/onecode.md`、`docs/research/superset.md`

- 用 repo 内配置文件声明 “worktree/workspace 创建后要跑什么”。
- 把外部资源（端口/DB/缓存/本地进程）也 workspace 化，setup/teardown 自动化（**v0.2.0 不依赖 Docker**）。

### 2.4 Artifacts/Preview：AionUi

来自：`docs/research/aion-ui.md`

- 预览面板 + preview history（bounded、可索引）是 RTS 控制台必备部件。
- runner/协议检测与适配层思路（多 CLI 统一入口）。

---

## 3) 核心数据结构（把特殊情况“建模掉”）

> 先定数据结构，再写代码。别靠 `match` 嵌套修补设计缺陷。

最小实体（名字可调整，边界必须清晰）：

- `Workspace`：一个隔离执行单元（路径 + 生命周期 + 可写根目录）。
- `Thread/Turn/Item`：事件流与运行时语义（对应 UI/daemon 的唯一数据源）。
- `Artifact`：产物（log/diff/patch/html/截图…）+ 索引元数据（类型、生成者、时间、路径、可预览）。
- `ApprovalRequest/ApprovalDecision`：审批请求与决定（必须落盘，必须可回放）。
- `AttentionState`：从事件流派生的“需要人介入”的状态机（不是 UI 临时计算）。
  - 已定：artifact 主要指“给用户看的文档 + 不进 repo 的临时产物”；repo/workspace 内的代码改动不算 artifact。

所有权原则（Rust 视角）：

- 不在核心结构里存引用与复杂生命周期；存 `PathBuf/String` 与强类型 id（UUID/新类型）。
- 事件数据尽量 append-only（便于回放与审计）；派生视图单独存缓存/索引。

---

## 4) 执行底座（vNext 的地基）

### 4.1 Tools 与审批

- 工具必须是“语义化 API”，不要接受任意字符串 shell 作为主接口。
- 审批策略：`auto/never/on-request/...` + 细粒度 `Run/Deny/Escalate`。
- 审批点必须和 `execpolicy` 联动：不是“问一下用户”，而是每次 exec 都能被 policy 拦截。

### 4.2 Sandbox policy（最小三档）

- `read-only`：只读文件 + 受限网络（可选）+ 禁止写入。
- `workspace-write`：只允许写入 workspace roots（用于编译/测试/生成 artifacts）。
- `danger-full-access`：调试/本机 trusted 场景（必须显式）。

### 4.3 事件化（RTS 必需）

事件必须覆盖：

- tool start/finish（含 stdout/stderr、exit code、耗时、资源路径）
- file edits（结构化 patch 或 edit script）
- approvals（request/decision）
- artifacts（新增/更新）
- 状态变迁（pause/resume/interrupt/cancel）
- **process 运行态**：running/exited/failed、以及 stdout/stderr 的 streaming 写入与定位信息（支持随时 inspect/attach）
  - 已定：事件落盘是权威来源；流式订阅丢了也能通过 `resume + 重放` 补齐（需要单调序号/offset）。

---

## 5) RTS 控制面（不是 UI，而是协议能力）

最小必备控制操作：

- `pause(resume)`：暂停任务执行（但仍可查看日志/事件）。
- `interrupt`：打断当前 turn/tool（类似“停手”）。
- `cancel`：终止 task（并落盘原因/残留 artifacts）。
- `step`：在 plan/execute 之间做硬切换（默认先 plan）。
- `inspect/attach`：只读查看运行中进程/子 agent 输出（禁止 stdin 交互）；必要时可 `kill`。

Attention/Inbox（派生视图）至少包含：

- `NeedApproval`（阻塞）
- `PlanReady`（等待用户确认）
- `DiffReady`（等待 review）
- `TestFailed`（等待修复/降级）
- `Stuck`（超时/循环检测/无进展）
- `Done`

> RTS 的关键不是“同时跑 100 个 agent”，而是用户能在 30 秒内定位：**哪个在卡、为什么卡、我该按哪个按钮**。

---

## 6) Workspace 生命周期脚本（setup/run/archive）

我们需要一个可版本化的配置入口（形式先别争，功能必须有）：

- `setup`：创建 workspace 后运行（复制 `.env`、装依赖、起外部资源、端口映射）。
- `run`：用户/调度层触发的一键运行（dev server / tests / lint）。
- `archive/teardown`：回收外部资源（本地进程/DB branch/端口占用），并清理 workspace。

脚本执行要求：

- stdout/stderr 必须落盘到 artifacts。
- 每条命令要有 timeout 与可读的失败摘要（别让人翻半天日志）。
- 为脚本注入标准 env：`CODEPM_WORKSPACE_NAME`、`CODEPM_ROOT`、`CODEPM_WORKSPACE_DIR`、`CODEPM_PORT`（占位）等。

参考实现方向：

- 1Code：`.1code/worktree.json` 的 `setup-worktree*` + `ROOT_WORKTREE_PATH` 注入。
- Superset：`.superset/config.json` + `setup.sh/teardown.sh`（外部资源隔离的标准答案；我们只学“生命周期脚本化 + 资源命名/隔离”，不引入 Docker 依赖）。

---

## 7) 交付适配层（最后再谈 git）

### 7.1 patch-only（默认）

- 输出：`unified diff` + artifacts（logs/diagnostics）。
- 验收：用户可以在任意 repo `git apply` 并通过 checks。

### 7.2 git-branch（可选 adapter）

- 把 git 当成“把 patch 应用到分支并跑 checks”的交付通道。
- PR/merge 属于更上层流程（可接 GitHub，也可只做本地 bare repo）。

> Git 不是核心域。核心域是：workspace 里发生了什么变更、运行了什么命令、产出了什么 artifacts、现在需要谁批准。

---

## 8) 复用策略（允许复制，但别做成依赖垃圾）

- 允许：从 `example/*` 手动复制必要代码片段并重构。
- 禁止：任何构建/运行时依赖 `example/`。
- Fork 可维护性：对“来自上游的共享文件”引入变更 marker（参考 `docs/research/kilocode.md` 思路）。
- License：大段复制必须检查上游 license/NOTICE 要求。

详见：`docs/upstream_reuse_policy.md`。

---

## 9) 里程碑（按可验证切片）

### M0：冻结（已完成）

- `v0.1.1` tag。

### M1：事件模型 + 存储 + 回放

- `Thread/Turn/Item` + file storage + `list/show/export/replay`。
- Attention view：能列出 `NeedApproval/TestFailed/...`。

### M2：执行层（tools + approvals + sandbox）

- 统一 tool runtime（shell/file/grep/glob/patch）。
- execpolicy + approval policy 可组合。

### M3：Agent loop（Responses-only）

- tool calling + schema output + 最小 agent 端到端能改代码并产出 patch。

### M4：并发编排（RTS 最小控制面）

- worker pool + per-task workspace。
- pause/resume/interrupt/cancel + 事件流不丢。
- workspace hooks：setup/run/archive。

### M5：交付 adapters（可选）

- patch-only → git-branch →（可选）GitHub PR/merge。

---

## 10) 验证门槛（别把技术债当进度）

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

文档要求：

- 任何新能力都要更新 `CHANGELOG.md` 的 `[Unreleased]`。
- 新增/变更协议或落盘格式时，必须给出迁移策略（哪怕是 “v0 不保证兼容” 也要写清楚）。
