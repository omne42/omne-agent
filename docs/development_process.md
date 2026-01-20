# CodePM vNext 重新开发流程（Agent-First）

> 现状：当前实现已存档为 `v0.1.1`（见 git tag）。接下来要做的是 **重新设计并重写**：以“能写代码且能与环境交互的 Agent CLI”为核心基建，Git 只是可插拔交付适配层。

---

## 0. 总原则（少废话）

1. **Agent 执行底座优先**：先把「工具执行 + 沙箱 + 审批 + 可追溯事件」做扎实，再谈 PR/merge。
2. **数据结构先行**：先定 `Thread/Turn/Item` 与 `Session/Task` 的边界、所有权与落盘格式；避免靠一堆 `match` 修补糟糕建模。
3. **复用要“复制式”而非“依赖式”**：允许从 `example/*` 手动复制代码片段并重构，但最终产物不得依赖 `example/`（见 `docs/upstream_reuse_policy.md`）。
4. **Modern Rust**：默认 `rustfmt`，`clippy --deny warnings`，无 `unsafe`（除非能写出不可反驳的不变量文档）。

---

## 1. 目标形态（我们要交付什么）

### 1.1 核心产物：可编排的 Agent CLI

必须具备（优先级从上到下）：

1. **可控的环境交互**：命令/文件/网络访问都走统一执行层，具备 `Run / Deny / Escalate` 的审批语义。
2. **Sandbox policy**：至少实现 `read-only / workspace-write / danger-full-access` 三档。
3. **事件可追溯**：所有 side effects 事件化（item 化）并落盘，能回放、能审计（UI/daemon 只是消费这些事件）。
4. **Responses-only**：第一阶段只支持 OpenAI `POST /v1/responses`。
5. **控制面**：优先对齐 `app-server` 这类“可被 IDE/客户端驱动”的协议形态（JSON-RPC + 事件流）。

对照来源（只取精华）：

- Codex：`sandbox/approvals/execpolicy/exec-server` + `app-server` + `Thread/Turn/Item`（见 `docs/research/codex.md`）
- OpenCode：Storage/Bus/Worktree 的工程化边界（见 `docs/research/opencode.md`）
- Claude Code / Kilo：workflow/skills/guardrails 的“纯文本可版本化”形态（见 `docs/research/claude-code.md`、`docs/research/kilocode.md`）

### 1.2 Git 的定位（降级为适配层）

Git 不再是核心域模型，而是一个可插拔 adapter：

- 核心域只关心：`workspace` 里发生了哪些变更（files/patch）、checks 结果、产物路径。
- 是否 commit/push/merge（以及是否 GitHub PR）属于交付通道，放在后续里程碑。

### 1.3 RTS 风格交互（高 APM，但必须可控）

RTS 不是“开更多并发”。它要求控制面具备下面这些硬能力（不然就是失控的 token 烧钱机）：

1. **事件流是一等 API**：终端输出/文件编辑/工具调用/审批/产物（artifacts）全部事件化，客户端只消费事件流（artifact 指“给用户看的文档 + 不进 repo 的临时产物”，repo/workspace 内的代码改动不算 artifact；参考 `docs/research/codex.md` 的 `Thread/Turn/Item`）。
2. **注意力队列（Attention / Inbox）**：把“需要人介入”的点变成可枚举状态：`NeedApproval` / `PlanReady` / `DiffReady` / `TestFailed` / `Stuck` / `Done`。
3. **可暂停/可打断/可步进**：至少要能 `pause/resume/interrupt/cancel`；更进一步要支持 turn 级 step（默认 plan → approve → act）。
4. **workspace 生命周期脚本化**：`setup/run/archive(or teardown)` 必须是约定俗成的 hook（参考 `docs/research/onecode.md`、`docs/research/superset.md`）。
5. **artifacts/preview 一等化**：生成物必须可索引、可预览、可追溯版本（参考 `docs/research/aion-ui.md` 的 preview history 思路）。

---

## 2. 开发里程碑（按“可验证”切片）

### M0：冻结与回溯（已完成）

- git tag：`v0.1.1`（可随时回看旧实现与旧流程）。

### M1：事件模型 + 存储（先把“可追溯”钉死）

- 定义强类型：`Thread/Turn/Item`（或等价结构），并把 “shell/file edit/approval request/tool result” 全部 item 化。
- Storage：文件落盘即可，但必须支持 `list/show/export`，并能稳定反序列化（避免未来迁移地狱）。
- 至少实现一个 “Attention view”：从事件流派生出 `NeedApproval/TestFailed/...` 的可查询状态（别让 UI/用户自己 grep 日志）。

验收：跑一次最小 turn，能在本地落盘并完整重放事件序列。

### M2：执行层（真正的基建）

- 实现执行策略：approval policy + execpolicy（prefix rules）。
- 执行后端：统一走一个 “可拦截 execve 的 shell/runner”（不要直接 `Command::new("bash -lc ...")` 然后祈祷安全）。
- Sandbox：默认最小权限；需要写入时显式声明 writable roots。

验收：在 `workspace-write` 下能安全运行 `rg/cargo test` 等；越权操作必须走 `Escalate`。

### M3：Agent loop（能写代码 + 能用工具）

- 接入 Responses API（只做这一条线），支持工具调用并发、结构化输出（JSON schema）。
- 抽象出 role：`Architect/Worker/Reviewer` 的最小接口，但先落地一个 worker 端到端跑通。

验收：给一个 repo + prompt，agent 能用工具完成修改并产生 patch（不要求 git）。

### M4：任务编排（并发与隔离）

- 并发：worker pool + per-task workspace（`/tmp` 隔离是结果，不是目的）。
- 事件总线：把 session/task 的状态流式推给 CLI/app-server client。
- workspace hooks：支持 `setup/run/archive` 的脚本化生命周期（最小可用就是“跑一串命令 + 落盘 stdout/stderr”）。

验收：并发执行 N 个 task，事件流不丢、不乱序到不可消费。

### M5：交付适配（最后再谈 Git）

- 把 git 变成 “apply patch → checks → commit/push/merge” 的可选 pipeline。
- 支持至少两条交付通道：`patch-only` 与 `git-branch`。

验收：不开 git 也能产出可应用的 patch；开 git 才做分支与 merge。

---

## 3. 代码复用与变更约束（允许 copy，但别变成依赖垃圾）

- 允许：从 `example/*` 手动复制必要代码片段，重构为本仓库 crate，并补齐测试。
- 禁止：任何构建/运行时依赖 `example/`（见 `docs/upstream_reuse_policy.md`）。
- 许可证：大段复制必须检查上游 license/NOTICE 要求（别自找麻烦）。

建议（降低未来同步成本）：

- 对“来自上游思想/代码的文件”引入 marker 体系（参考 `docs/implementation_plan.md` 的建议）。

---

## 4. 质量门槛（别把技术债当进度）

建议作为默认 gate（本仓库已支持）：

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

文档要求：

- 任何新能力都要更新 `CHANGELOG.md` 的 `[Unreleased]`。
- “怎么跑”必须给出可复制执行的命令（不要写空话）。
