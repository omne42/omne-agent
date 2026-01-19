# Kilo Code（example/kilocode）能力与设计分析

> Snapshot: `example/kilocode` @ `e4ced0062c`
>
> 结论先行：这个 snapshot **不包含完整的扩展源代码**（未见预期的 `src/`、`webview-ui/` 等目录），但包含大量“工程治理层”的材料：`AGENTS.md`、工作流、技能、模式权限、变更日志、贡献规范等。对 `codex_pm` 来说，Kilo Code 的最大价值在于：它展示了一个大型 agent 平台如何做**角色/模式权限**与**fork 维护策略（merge 上游）**。

---

## 1. 仓库在本 snapshot 中包含什么

- 规范与治理：
  - `example/kilocode/AGENTS.md`（项目结构、质量规则、fork 合并策略、markers）
  - `example/kilocode/CONTRIBUTING.md`、`example/kilocode/CHANGELOG.md` 等
- “模式/权限”配置：
  - `example/kilocode/.kilocodemodes`
- “技能/工作流”：
  - `example/kilocode/.kilocode/skills/*`
  - `example/kilocode/.kilocode/workflows/*`
- docs app（但内容偏模板）：
  - `example/kilocode/apps/kilocode-docs/*`

> 缺失：与 `AGENTS.md` 描述的真实目录（`src/`、`webview-ui/`、`cli/`、`packages/` 等）在本 snapshot 中未包含，因此“工具实现细节/实际 UI/具体 provider 代码”无法完整调研，只能基于 README 与规范推断。

---

## 2. “所有能力”盘点（基于 README/AGENTS.md/配置推断）

来自 `README.md` 与 `AGENTS.md` 的定位：

- VS Code 端的 agentic engineering platform：
  - 自然语言生成代码
  - 自检（checks its own work）
  - 运行终端命令
  - 浏览器自动化
  - inline autocomplete
  - 多模型/多 provider（宣称 500+ models）
  - MCP server marketplace
  - Multi Mode（Architect/Coder/Debugger + 自定义 modes）
- 工程结构（`AGENTS.md` 描述）：
  - monorepo（pnpm + Turbo）
  - `src/`（扩展核心）、`webview-ui/`（React UI）、`cli/`、`packages/`、`jetbrains/` 等
- 发布与变更治理：
  - changesets（每个 PR 需要 changeset，除 docs/内部工具）
  - `CHANGELOG.md` 极长，说明更新频繁且功能面很广

补充（工程质量与协作）：

- `AGENTS.md` 明确了测试/lint 的“强规则”：
  - 必须补测试覆盖；vitest；并强调**必须在正确 workspace 目录运行测试**（避免 monorepo root 找不到 vitest）。
  - 禁止随意关闭 lint rule。
- 依赖治理：
  - dev 环境支持 native / devcontainer / nix flake 三套方案（`DEVELOPMENT.md`）。
  - `DEVELOPMENT.md` 要求 Git LFS（处理 GIF/MP4 等大资源）。

---

## 3. 最值得学习的部分（巧思与可迁移点）

### 3.1 Fork 维护策略：`kilocode_change` markers（强烈推荐我们复刻）

`example/kilocode/AGENTS.md` 的关键点：

- Kilo Code 是 Roo Code 的 fork，会周期性合并上游。
- 为减少冲突，在共享代码中用 `kilocode_change` 标记 Kilo 自定义修改：
  - 单行：`// kilocode_change`
  - 多行：`// kilocode_change start ... end`
  - 新文件：`// kilocode_change - new file`
- 并明确哪些目录不需要 markers、哪些必须有。

> 对 `codex_pm`：我们明确要“基于 codex 魔改”。如果我们未来要跟随 codex 上游更新，这套 marker 体系几乎是必需品。建议我们引入类似：
>
> - `codex_pm_change` / `codex_pm_change start/end`
> - 并在 CI 或脚本里检查“修改核心上游文件是否缺 marker”

补充：Kilo 还提到会用 `scripts/kilocode/` 周期性 merge 上游。对我们来说，哪怕第一阶段先手工同步，也建议把“上游同步”当作一个正式工作流来设计（否则魔改越久越难跟）。

### 3.2 Mode = 角色 + 权限边界（对我们角色体系非常有参考价值）

`example/kilocode/.kilocodemodes` 展示了“自定义 mode”的最小模型：

- `slug/name/roleDefinition/customInstructions`
- `groups` 定义权限能力：
  - `read`
  - `browser`
  - `command`
  - `edit` 且用 `fileRegex` 限定可编辑文件范围

这是一种非常务实的“最小权限”实现方式：

- 不是全局 sandbox，而是**按角色限制“能做什么 + 能改哪里”**；
- 且模式本身可配置、可版本化、可分享。

> 对 `codex_pm`：我们提出了 Ideator/IdeaCritic/Architect/Coder/Reviewer/Merger/Builder/FrontendStylist 等角色。Kilo 的 mode 权限模型非常适合映射：
>
> - `Reviewer`：read-only + 允许运行有限的检查命令
> - `FrontendStylist`：只允许 edit `*.css`/`*.scss`/`tailwind config` 等
> - `Builder`：允许 command（build/deploy），但 edit 权限受限
>
> 在 codex 侧，可用 sandbox + approvals + tool allowlist 实现同等效果。

### 3.3 Workflow = Orchestrator 指令集（多 agent 并行拆分的明确范式）

`example/kilocode/.kilocode/workflows/add-missing-translations.md`：

- 要求 Orchestrator mode。
- 先在 Code mode 跑脚本找缺失翻译。
- 对每个语言 + JSON 文件，启动一个 Translate mode 的“单独子任务”，禁止在同一子任务处理多个语言/文件。

这是一种非常清晰的“拆分原则”：

- **拆分维度明确**（language × file）
- **并发粒度可控**
- **每个子任务输入/输出边界清晰**（减少上下文污染、便于重试）

> 对 `codex_pm`：这与我们“临时目录 + 并发 task + PR 产出”几乎是同构的；只是 Kilo 的子任务是逻辑拆分，我们的子任务还要落到 `/tmp/{repo}_{session}/tasks/{task}` 的物理隔离。

### 3.4 Skills = 长期可维护的团队规范沉淀

`example/kilocode/.kilocode/skills/translation/SKILL.md` 非常长，覆盖：

- 支持语言集合
- 风格/语气/术语规则
- placeholders 约束
- 具体工作流步骤与校验脚本
- zh-CN/zh-TW/de 的语言特定规范

> 对 `codex_pm`：如果我们要做“全生命周期”，最终一定会积累大量“组织级规范”（commit message、PR 模板、目录约定、安全规范）。Skills 文档化 + 可自动触发，是最可维护的方式之一。

---

## 4. 对 `codex_pm` 的落地建议（结合我们“基于 codex 魔改”）

### 4.1 先引入 fork markers（P0）

在我们开始复制/魔改 `example/codex` 的代码前，就建立：

- `codex_pm_change` marker 规范
- 合并上游时的脚本流程（类似 `scripts/kilocode/` 的概念）

### 4.2 建立“角色 → 权限策略”的配置层（P0/P1）

把我们角色体系（Ideator/IdeaCritic/Architect/…）落到可配置权限上：

- 文件编辑范围（regex / path allowlist）
- 可执行命令范围（execpolicy）
- sandbox policy（read-only / workspace-write）

### 4.3 把工作流写成“可版本化 spec”（P1）

Kilo 的 workflows/skills 是纯文本，天然可 review/可复用。我们可以：

- 在 `codex_pm` 中引入 `workflows/` 目录（或 `docs/workflows/`）
- 用统一 frontmatter 定义 allowed tools / inputs / outputs
- orchestrator 读取 spec，生成并发 task

---

## 5. 后续调研需要补齐的点

由于本 snapshot 缺少核心源代码，后续若要更完整对齐 “所有能力”，建议在可获取完整代码时重点补齐：

- provider/模型路由的实现细节（尤其是 tool calling 与 reasoning 的适配）
- 浏览器自动化与安全隔离（browser sandbox、权限提示）
- checkpoint/回滚机制（与我们多 PR 合并的可恢复性强相关）
- MCP marketplace 的实际接入与管理机制
