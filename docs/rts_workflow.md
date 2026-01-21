# CodePM vNext 使用流程（RTS 风格 / 目标态）

> 这不是“又一个聊天框”。RTS 的关键是：你同时指挥多个 agent，但系统必须把一切变更收敛成 **可观测、可暂停、可回放、可收口** 的状态机。

---

## 1) 核心对象（UI 只是这些对象的投影）

- `Workspace`：隔离执行单元（目录 + 生命周期）。
- `Run`（或 `Session`）：一次端到端执行（输入 prompt + 约束 + 目标）。
- `Thread/Turn/Item`：事件流与语义（工具执行/文件编辑/审批/产物/状态变化）。
- `Artifact`：产物索引（logs/diff/patch/html/图片…），必须可预览、可定位、可版本化。
- `Approval`：审批请求与决策（必须落盘、可审计）。
- `Attention`：从事件流派生的“需要人介入”的收件箱视图。

补充（已定语义）：

- `resume`：必须能从持久化历史恢复 session/thread 的完整上下文（对话 + 中间态产物 + 子 agent 步骤），并在此基础上继续对话；不承诺“原进程继续跑”，但承诺“历史不丢、可继续推进”。

---

## 2) 主流程（Plan → Approve → Act，默认节奏）

1. **创建 workspace**
   - 系统创建隔离目录（本地 `/tmp` 或 git worktree 等目录级隔离方案是实现细节；**git/workspace 不以 Docker 为前提，但不禁止 agent 自己运行 Docker**）。
   - 自动执行 `setup` 生命周期脚本（复制 env/装依赖/启动外部资源/端口映射）。
2. **Plan（先想清楚）**
   - agent 输出结构化 plan（作为 artifact 落盘）。
   - `Attention` 标记为 `PlanReady`，等待用户确认/修改。
3. **Approve（显式放权）**
   - 用户确认 plan，并选择 sandbox/approval policy（例如 `workspace-write` + `on-request`）。
4. **Act（执行）**
   - agent 通过 tools 与环境交互：读写文件、运行命令、查询网络资源（按 policy 约束）。
   - 每一次工具调用都生成 `Item`，并落盘 stdout/stderr 与相关 artifacts。
5. **Review gate（收口）**
   - 产出 diff/patch（artifact），`Attention` 标记为 `DiffReady`。
   - 测试失败则标记 `TestFailed`，把“修复”当成下一轮 turn（而不是默默继续乱改）。
6. **Deliver（交付）**
   - 默认 `patch-only`：输出可应用 patch + artifacts。
   - 可选 git adapter：把 patch 应用到分支、跑 checks、（可选）创建 PR/合并。
7. **Archive（回收）**
   - 执行 `archive/teardown` 脚本回收外部资源（本地进程/DB branch/端口占用）。
   - 保留 Run 的事件与 artifacts，允许回放与恢复。

---

## 3) RTS 控制面最小操作集（不然你就失控）

- `pause/resume`：暂停/恢复某个 workspace 或 task（让用户能“停战整理战场”）。
- `interrupt`：打断当前工具/turn（防止卡死/循环烧钱）。
- `cancel`：终止任务（并落盘原因与残留 artifacts）。
- `escalate`：当出现越权操作时，必须进入 `NeedApproval`，由人决定放行/拒绝。
- `inspect/attach`：随时查看任何运行中的后台进程/子 agent（状态 + 最近输出 + 完整 artifacts 路径；只读，不提供 stdin 交互）。
- `kill`：强制停止某个后台进程（语义必须事件化并进入审计/回放）。

RTS 的重点是 `Attention`：用户不应该刷屏看日志，而是看“系统现在需要我做哪三件事”。

---

## 4) 中间态 artifacts（必须）

> 结束时给一个 `result.json` 没用。RTS 场景下你需要“随时看中间态”：stdout/stderr、部分 diff、计划草案、测试进度。

- stdout/stderr 必须 **边产出边落盘**（append），并能 `tail/follow`。
- 事件订阅端掉线/lag 不应导致“看不到历史”：以落盘事件为权威来源，`resume + 从 seq 重放` 必须能补齐。
- 每个后台进程都要有 `process_id`，并关联到 `thread/turn/agent`，便于定位。
- “卡住/等待输入/等待批准”必须进入 `Attention`，并触发通知（否则就是黑盒）。
- artifact 只包含“给用户看的文档 + 不进 repo 的临时产物”（repo/workspace 内的代码改动不算 artifact）；建议采用 `*.md + *.metadata.json`（参考 `~/.gemini/antigravity/brain`）。
- 单 session 产物过大时告警，并生成一份 markdown 清理报告，支持一键清理历史记录（不在这里做复杂 eviction）。

---

## 5) 两条硬约束（来自爆发期产品的共识）

1. **workspace 生命周期脚本化**：并行开发真正的冲突点在外部资源（端口/DB/缓存/本地进程），必须靠 `setup/run/teardown` 自动化解决（参考 `docs/research/superset.md` 的“脚本化生命周期”思路）。
2. **artifacts/preview 一等化**：你产出的不是 token，是文件与结果；没有预览与索引，RTS 只会变成“100 个黑盒在跑”（参考 `docs/research/aion-ui.md`）。
