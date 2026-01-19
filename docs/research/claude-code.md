# Claude Code（example/claude-code）能力与设计分析

> Snapshot: `example/claude-code` @ `74cc597`
>
> 结论先行：这个仓库在本 snapshot 中**不是 Claude Code CLI 的完整源码**，更像是“发行物 + 文档 + 官方插件合集”。但它仍然极具学习价值：它把一个成熟 coding agent 的**插件/工作流/Hook/权限**体系以可读的方式摊开了，尤其适合我们为 `codex_pm` 设计“角色化流水线 + guardrails + 并行子任务”。

---

## 1. 仓库包含什么（以及缺什么）

### 1.1 包含

- 插件合集：`example/claude-code/plugins/*`（commands/agents/skills/hooks 的组合）。
- 内置命令示例：`example/claude-code/.claude/commands/*`（用于 repo 内工作流）。
- 插件 marketplace 清单：`example/claude-code/.claude-plugin/marketplace.json`。
- 变更日志：`example/claude-code/CHANGELOG.md`（记录大量产品能力与安全修复点）。

### 1.2 缺失

- 没有完整 CLI 实现代码（至少此 snapshot 未包含 Node/TS 源码）。因此“底层如何实现 TUI/沙箱/工具执行”等只能从 changelog + 插件约定推断。

---

## 2. “所有能力”盘点（基于 changelog + 插件/命令约定）

> 以下以“能力域”分类，避免只罗列功能点。

### 2.1 Agent 交互与工作流能力

- 终端/IDE 内的 agentic coding：理解代码库、执行任务、处理 git 流程（README 定位）。
- **Plan/任务管理**：支持 plan 文件目录配置（`plansDirectory`），以及多轮 plan 行为修复（`/clear` 后 plan 文件刷新等）。
- **并行工具调用 / 并行子 agent**：changelog 多次提到并行工具调用导致的 orphan tool_result 问题修复，说明其底层支持“同 turn 多工具并行”。
- **Background tasks**：存在后台任务机制（可用快捷键/配置开关、环境变量禁用）。
- **外部编辑器整合**：如 Ctrl+G 打开外部编辑器输入。
- **会话归因**：对 commit/PR 写入 session URL attribution（利于审计/可追溯性）。
- **会话/运行时参数**：支持通过环境变量控制行为（例如 temp dir、禁用后台任务、插件强制自动更新等，见 changelog）。

### 2.2 插件系统（我们最该学的“扩展点”）

插件结构在 `example/claude-code/plugins/README.md` 给出：

```
plugin-name/
├── .claude-plugin/plugin.json
├── commands/        # slash commands
├── agents/          # specialized agents
├── skills/          # agent skills
├── hooks/           # event handlers
├── .mcp.json        # external tool config
└── README.md
```

能力要点：

- **commands as markdown**：用 markdown + frontmatter 定义 slash command（见 `plugins/commit-commands/commands/commit-push-pr.md`）。
- **allowed-tools 白名单**：在 command frontmatter 中声明允许的工具集合，形成“最小权限”执行面。
- **hooks 事件点**：支持 `PreToolUse`、`SessionStart`、`Setup`、`Stop` 等 hook（changelog & 插件中可见）。
- **skills 体系**：可在 prompt 中 `$skill-name` 触发；技能可用 `${CLAUDE_SESSION_ID}` 做会话级变量替换（changelog）。

> 值得特别注意：changelog 提到 `Setup` hook 可通过 `--init/--init-only/--maintenance` 等 flag 触发，适合做“仓库初始化/维护操作”（这与我们 repo 注入、初始化 hooks 非常接近）。

### 2.3 安全与权限（guardrails）

从 `CHANGELOG.md` 能看出 Claude Code 强调安全与权限控制：

- 权限规则（exec/command allowlist）存在多处漏洞修复：如通配符规则匹配复合命令、shell 行续行绕过等。
- `PreToolUse` hook 能返回 `additionalContext` 给模型（用于安全提醒/限制）。
- 提供 tempdir override：`CLAUDE_CODE_TMPDIR`，说明其内部也依赖临时目录隔离（与我们 `/tmp/{repo}_{session}` 目标一致）。
- 对插件安装有“信任警告”（VSCode 侧）。

### 2.4 MCP（Model Context Protocol）与工具生态

- changelog 多次提到 MCP 连接/重连、HTTP/SSE transports、mcp list/get 残留进程等，说明 Claude Code 在“连接外部工具服务器”方面投入很深。
- **MCP tool search auto 模式**：当 MCP 工具描述过长会 defer，通过 `MCPSearch` 工具在需要时检索（典型的“上下文预算优化”）。

---

## 3. 插件与命令的深度样例（值得复制的“巧思”）

### 3.0 Bundled plugins 清单（能力面覆盖）

`example/claude-code/.claude-plugin/marketplace.json` 列出的官方插件（本 snapshot 内可见）：

- `agent-sdk-dev`：Agent SDK 开发与验证（带 verifier agents）。
- `claude-opus-4-5-migration`：模型/提示词迁移工具（skill）。
- `code-review`：多专项并行 agent 的 PR review（含 false positive 过滤）。
- `commit-commands`：commit / push / PR 创建工作流（slash commands）。
- `explanatory-output-style`：SessionStart hook 注入“解释型输出风格”（教育向）。
- `feature-dev`：结构化 feature 开发 7 阶段 workflow（含 explorer/architect/reviewer agents）。
- `frontend-design`：前端 UI 设计/美化 skill（强调“避免 AI 平庸风格”）。
- `hookify`：从对话或显式指令生成 hooks 规则（对“防止不想要的行为”很实用）。
- `learning-output-style`：学习模式（SessionStart hook）。
- `plugin-dev`：插件开发工具链（agents + skills）。
- `pr-review-toolkit`：6 个专项 review agents（comments/tests/errors/types/code/simplify）。
- `ralph-wiggum`：自循环迭代（Stop hook）。
- `security-guidance`：PreToolUse 安全提醒 hook（文件编辑/命令风险模式）。

这张清单本身就很像我们计划的“角色体系”：Coder/Reviewer/Architect/FrontendStylist/Builder 的雏形都能找到对应参考。

### 3.1 “命令即工作流”：frontmatter + 上下文注入

以 `plugins/commit-commands/commands/commit-push-pr.md` 为例：

- frontmatter 定义 `allowed-tools`（只允许 `git`/`gh pr create` 相关命令）。
- “Context”段落用 `!` 语法注入命令输出：
  - `!`git status`、`!`git diff HEAD`、`!`git branch --show-current`
- “Your task”段落把流程拆成明确步骤，并强制“单消息内完成所有 tool calls”。

这种写法的价值：

- **可审计**：工作流是纯文本文件，可 review、可版本管理。
- **最小权限**：每个 workflow 只开必要工具。
- **稳定可复用**：在不同 repo 下复用同一套 git 流程。

> 对 `codex_pm` 的启示：我们完全可以把“fmt/check/commit/push/pr”的流水线写成可版本化的 workflow spec（甚至直接复刻这个 markdown-frontmatter 约定）。

### 3.2 并行审阅：多 agent 专项分工

`plugins/pr-review-toolkit` 与 `plugins/code-review` 展示了“并行专项 agent”思路：

- comment、tests、silent failures、type design、general code review、simplify 等 6 个 agent。
- 典型收益：把 review 的“高 recall/低 precision”问题，通过分工 + 汇总过滤，减少误报。

> 对 `codex_pm` 的启示：我们的 `Reviewer`/`Merger` 可以复用这种“多视角并行→汇总→阻断/建议”的结构，而不是单模型做全量 review。

### 3.3 Hook 驱动的安全提醒：PreToolUse 守门

`plugins/security-guidance/hooks/security_reminder_hook.py` 是非常直接的“守门钩子”：

- 在文件编辑前检查 path/content 的风险模式（GitHub Actions 注入、`eval`、`innerHTML`、`os.system` 等）。
- 用 session_id 做状态文件（`~/.claude/security_warnings_state_<session>.json`），避免重复弹窗。
- debug log 写到 `/tmp/security-warnings-log.txt`，不影响主流程。

> 对 `codex_pm` 的启示：我们未来要“自动化全生命周期”，不可避免地会执行 git/CI/脚本；建议从第一天就预留类似的 hook/guardrail API。

### 3.4 “自我迭代循环”：Stop hook 拦截退出

`plugins/ralph-wiggum` 提供一个非常产品化的机制：当 agent 想退出时，Stop hook 拦截，强制继续迭代直到完成（或用户 cancel）。

> 对 `codex_pm` 的启示：我们的并发 worker 经常会出现“半成品 PR”。引入“迭代循环 + 完成判定”会显著提高任务完成率（但必须有资源/超时上限）。

---

## 4. 值得学习的设计原则（抽象成可复用的架构点）

### 4.1 Progressive Disclosure（按需加载工具描述）

MCP 工具描述过长会消耗上下文，Claude Code 用“自动 deferred + MCPSearch”解决：

- 默认不把全部工具塞进 system prompt。
- 在需要时用搜索工具“找出相关工具并启用”。

> 这对于 `codex_pm` 的“多 agent 并发”尤其关键：并发意味着上下文/成本成倍上升，我们也要从一开始就做上下文预算管理。

### 4.2 Session-level 可追溯性（commit/PR attribution）

把 session URL 写入 commit/PR（changelog 提到）。对企业合规/回溯极其有用。

> 我们的 PR 模型也应记录：session_id、prompt 摘要、工具调用摘要、校验结果、合并策略。

---

## 5. 对 `codex_pm` 的直接可复用点（建议优先级）

### P0：立刻可复用（几乎不依赖底层实现）

- **workflow as markdown**：用 frontmatter 声明工具白名单 + “上下文注入”模式来定义 git 流程与 review 流程。
- **角色化子 agent 模板**：review 的专项 agent 列表/输出格式，可以直接迁移到我们的角色体系。

### P1：需要我们底层支持（但高价值）

- Hook 机制：`PreToolUse`（安全提醒/策略注入）、`SessionStart`（注入 repo 规范）、`Stop`（迭代循环）、`Setup`（仓库初始化/维护）。
- “并行子 agent + 汇总过滤”框架（对应我们的 `Architect`/`Reviewer`/`Merger`）。

---

## 6. 与我们的方向的对齐点

我们计划基于 `example/codex` 魔改，但 Claude Code 的插件体系为我们提供了一个“产品化的工作流外壳”：

- `codex-rs` 可以提供稳定的执行与安全底座（sandbox/approvals/execpolicy）。
- 借鉴 Claude Code 的做法，我们可以把“全生命周期”能力做成可版本化的 workflow/role 包，而不是把全部逻辑硬编码进 orchestrator。

---

## 7. 建议我们重点抄作业的“细节型能力”（从 changelog 摘要）

以下能力在 changelog 中出现频率高、且与大型系统稳定性高度相关，值得我们在 `codex_pm` 里提前预留：

- **技能热重载**：skills 文件改动无需重启即可生效（对团队协作很重要）。
- **skills 支持 fork context**：允许在 forked sub-agent context 运行（对并发/隔离有启发）。
- **工具描述的上下文压缩策略**：MCP tool search auto-mode（大规模工具生态下必需）。
- **非交互环境兼容**：CI 场景下避免 stdin 挂死、禁用颜色等。
- **权限规则的可诊断性**：不可达规则检测、doctor 中给出可执行修复建议（这类可运维性对“自动化全生命周期”很关键）。
