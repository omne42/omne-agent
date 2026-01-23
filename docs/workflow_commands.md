# Workflow / Commands（Markdown + frontmatter）

> 目标：把“常用工作流”做成可版本化、可 review、可复用的 spec 文件，而不是散落在 prompt 里。
>
> 状态：v0.2.x 已落地 **v1 最小子集**（`pm command {list,show,run}`）；并发拆分/Orchestrator 读取仍是 TODO（见第 4 节）。

---

## 0) 范围与非目标

范围（未来实现）：

- 用一个 Markdown 文件描述一次“命令/工作流”（类似 Claude Code 的 commands）。
- frontmatter 声明最小权限（`allowed_tools`）与默认 `mode`（复用 `docs/modes.md`）。
- 允许把 “git status/diff/rg 输出” 等上下文抓取步骤做成**可审计**的标准注入（输出写入 artifacts，而不是塞进事件）。

非目标（别自找麻烦）：

- 不把 workflow 当成一门新 DSL；只做“可读的 Markdown + 少量结构化 frontmatter”。
- 不引入隐式执行：任何命令执行都必须走 tool（`process/start`）并落盘审计。
- 不提供 stdin/PTY 交互；保持非交互执行约束（见 `docs/v0.2.0_parity.md`）。
- 不做并发拆分/子任务依赖（v1 仅定义文件格式与顺序执行语义）。

---

## 1) 文件位置与发现顺序（建议写死）

Project config（可提交/可 review）：

- v1 **只支持**：`./.codepm_data/spec/commands/<name>.md`（不做 fallback/模糊搜索；找不到即报错）

CLI 形态（已实现）：

```bash
pm command list
pm command show <name>
pm command run <name> --var key=value
```

---

## 2) Frontmatter（最小字段）

文件必须以 YAML frontmatter 开头：

```yaml
---
version: 1
name: commit-push-pr
mode: coder
allowed_tools:
  - process/start
  - process/inspect
  - process/tail
  - process/follow
  - artifact/write
  - artifact/list
  - artifact/read
  - artifact/delete
context:
  - argv: ["git", "status", "--porcelain=v1"]
    summary: "git status"
    ok_exit_codes: [0]
  - argv: ["git", "diff", "--"]
    summary: "git diff"
    ok_exit_codes: [0]
inputs:
  - name: branch
    required: true
---
```

字段语义（建议写死）：

- `version`：整数，当前固定为 `1`。
- `name`：显示名（可选；默认用文件名）。
- `mode`：默认 mode（`architect/coder/reviewer/builder`），用于选择权限边界（见 `docs/modes.md`）。
- `allowed_tools`：额外的“最小权限”收口（可选）：
  - 语义是 **再收紧**：不在列表里的工具一律 `deny`（即使 mode 允许，也要拒绝）。
  - 与 mode 的合并语义：先算 mode gate，再与 `allowed_tools` 取交集（deny 优先）。
  - `allowed_tools: []` 表示 deny all（用于“只渲染文本，不允许任何工具/命令”的 workflow）。
  - 列表包含未知工具名（或包含 mode 本就不允许的工具）应直接报错（fail-closed），避免“看起来允许，实际 silently ignore”。
- `context`：上下文抓取步骤（可选）。每项至少包含：
  - `argv`：等价于一次 `process/start` 的 argv（不得用单字符串 shell 拼接）。
  - `summary`：一行描述（用于 artifact 列表与审计）。
  - `ok_exit_codes`：允许的 exit code 列表（可选；默认 `[0]`）。
- `inputs`：变量声明（可选）。每项包含：
  - `name`：变量名
  - `required`：是否必填（可选；默认 false）
  - 未声明但被引用的变量应直接报错（fail-closed）。

---

## 3) 渲染与注入（最小可审计语义）

建议实现约束（写死边界，别搞黑箱）：

- `context` 中的每个步骤必须通过 `process/start` 执行，并落盘 stdout/stderr（见 `docs/runtime_layout.md`）。
- workflow runner 不应把完整输出“内联塞进 prompt”（容易泄露 secrets + 爆 token）。
  - 最小可行做法：写入 process log artifacts，并只把 **脱敏后的摘要 + 路径** 注入 prompt（见 `docs/redaction.md`）。
- 失败语义（建议写死为 fail-closed）：
  - frontmatter 解析失败、变量缺失、权限不匹配、context exit code 不在 `ok_exit_codes` 内、artifact 写入失败等：应立即失败并停止后续步骤（避免“半执行”导致难以回放/审计）。

变量替换（建议）：

- 只支持 `{{var}}` 形式。
- `--var key=value` 提供值；未提供且 `required=true` 直接报错。
- 替换作用域：workflow 正文 + `context[*].argv` + `context[*].summary`。
- 传入未声明的变量应直接报错（fail-closed），避免拼写错误静默变成空字符串。

---

## 4) Orchestrator 并发拆分（TODO：后续再做）

当要把一个 workflow 拆成并发子任务时，建议只支持**一个简单约定**（别发明 DSL）：

- `## Task: <id> <title>` 作为 task 边界。
- 每个 task 作为一个子 thread/turn 执行；依赖关系（如果要做）先用最小字段 `depends_on: [a,b]` 放在 task 段落的局部 frontmatter（或先不支持）。
- fan-out/fan-in 的正确性来源仍然是事件落盘（见 `docs/thread_event_model.md`）。

---

## 5) 验收（v0.2.x 现状）

- `pm command list` 能发现 `./.codepm_data/spec/commands/*.md`，并显示 `name/mode/version`。
- `pm command run <name>` 必须把最终生效的 `mode/allowed_tools` 记录到 thread config（可解释性见 `thread/config/explain`）。
- `context` 步骤必须全部事件化（`ProcessStarted/Exited`），且输出可从 artifacts 定位（不塞进事件）。
- fail-closed：任一 context step exit code 不在 `ok_exit_codes` 内时必须终止执行，且错误原因可在事件/日志中定位。
