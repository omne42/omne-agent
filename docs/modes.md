# Mode（角色权限边界）规范（v1）

> 目标：把“角色/Mode”从 prompt 里的软约束，变成 **可落盘、可审计、可强制执行** 的权限边界。
>
> 原则：Mode 只解决“能不能做什么”；不解决“怎么做得更好”（那是 router/workflow/skills 的工作）。

---

## 1) 术语

- **Mode**：一个命名角色（例如 `architect/coder/reviewer/builder`），定义该角色允许使用的能力边界。
- **Role（角色）**：人类语义上的职责（Architect/Coder/Reviewer/Builder…）；在系统里以 `mode=<name>` 落盘并生效。
- **Capability（能力组）**：对工具能力的稳定抽象（例如 `read/edit/command/process/artifact/browser`；`browser` 字段 v0.2.0 已预留，但 web tools 仍是未来扩展）。
- **Decision**：权限判定结果：`allow | prompt | deny`
  - `deny`：硬拒绝（无论 approval policy 如何都不可执行）。
  - `prompt`：必须产生审批事件（`ApprovalRequested`），由 approval policy 决定“停下来等人”还是“自动决策并继续”。
  - `allow`：Mode 不拦截（后续仍受 `sandbox/execpolicy/approval policy` 约束）。

合并语义固定为：`deny > prompt > allow`。

### 1.1 Capability → tool 映射（v0.2.0 口径）

> 这是“规范”和“实现”交汇的地方：Capability 是稳定抽象，tool/method 是实现细节；但需要一张表避免讨论跑偏。

- `read`：`file/read`、`file/glob`、`file/grep`、`thread/state`、`thread/events`
- `edit`：`file/write`、`file/patch`、`file/edit`、`file/delete`、`fs/mkdir`
- `command`：`process/start`
- `process.inspect`：`process/inspect`、`process/tail`、`process/follow`（只读 attach）
- `process.kill`：`process/interrupt`、`process/kill`
- `artifact`：`artifact/write`、`artifact/list`、`artifact/read`、`artifact/delete`
- `browser`：`web/*`（未来）

---

## 2) 配置文件位置与发现顺序（写死）

### 2.1 Project config（仓库内，可提交/可 review）

- **Canonical**：`./.omne_agent_data/spec/modes.yaml`
- **Fallback**：`./.omne_agent_data/spec/modes.yml`

### 2.2 发现顺序（高 → 低）

1. CLI 显式参数（未来实现，例如 `omne-agent thread config set --modes <path>` 或 `omne-agent --modes <path>`）
2. env：`OMNE_AGENT_MODES_FILE`
3. `./.omne_agent_data/spec/modes.yaml`
4. `./.omne_agent_data/spec/modes.yml`
5. user/global（可选；默认可不启用）
6. 内置默认 modes（兜底）

> 注意：`./.omne_agent_data/` 同时承载运行时数据（threads/artifacts）与项目 spec（`.omne_agent_data/spec/*`）。secrets 放在 `.omne_agent_data/.env`，且默认禁止被 file tools 读取。
>
> `OMNE_AGENT_MODES_FILE`：
>
> - 支持绝对路径。
> - 相对路径按 **thread cwd（workspace root）** 解析（不是按当前进程 cwd）。
> - v0.2.0 默认按需从磁盘加载 modes 文件：修改后会在下一次相关调用（例如 `thread/configure` 校验或工具执行）生效，无需重启。

---

## 3) 与现有策略的组合顺序（写死）

执行时的判定链路顺序固定为：

`mode gate → sandbox → execpolicy → approval handling`

解释：

- **mode gate**：按能力组/路径规则/per-tool override 做第一层硬裁决（`deny/prompt/allow`）。
- **sandbox**：路径与可写根等硬边界（例如 `read-only/workspace-write`）。
- **execpolicy**：命令前缀规则（allow/prompt/forbidden）。
- execpolicy 规范与用法见：`docs/execpolicy.md`（v0.2.0 支持 global `--execpolicy-rules` + per-mode `command.execpolicy_rules`）。
- **approval handling**：当需要审批时（来源可能是 mode 或 execpolicy），由 `ApprovalPolicy` 决定停/自动决策。

合并规则：任一层 `deny` 即 deny；否则任一层 `prompt` 即 prompt；否则 allow。

---

## 4) `prompt` 在 `ApprovalPolicy=auto_approve` 下的语义（写死）

- `prompt` 永远表示：**必须落盘 `ApprovalRequested`**（审计/回放的事实）。
- 当 `ApprovalPolicy=manual`：进入 `NeedApproval`，等待人类 `ApprovalDecided`。
- 当 `ApprovalPolicy=auto_approve`：系统必须立刻落盘 `ApprovalDecided(Approved, reason="auto-approved by policy")` 并继续执行。

`deny` 永远不可被 auto_approve 覆盖。

---

## 5) 落盘格式（YAML/JSON 等价）

### 5.1 顶层

- `version: 1`
- `modes: { <mode_name>: ModeDef }`

### 5.2 `ModeDef`

- `description: string`
- `permissions: Permissions`

### 5.3 `Permissions`（能力组 + per-tool override）

v0.2.0 MVP 已支持字段：

- `read: { decision }`
- `edit: { decision, allow_globs?: [string], deny_globs?: [string] }`
- `command: { decision, execpolicy_rules?: [string] }`
- `process: { inspect: {decision}, kill: {decision}, interact: {decision} }`
- `artifact: { decision }`
- `browser: { decision }`（字段已支持；`web/*` 工具属于未来扩展）
- `subagent: { spawn: { decision, allowed_modes?: [string] } }`：fan-out / 子 agent 权限边界（对应 `agent_spawn`）
- `tool_overrides?: [{ tool: string, decision: Decision }]`（少数例外；避免把规则写成一坨）

未来扩展（TODO；v0.2.0 未实现）：

- `execpolicy_rules`：per-thread execpolicy 覆盖（TODO 草案见 `docs/execpolicy.md`）

约束：

- `process.interact` **必须为** `deny`（只读 attach，不做 stdin/PTY）。
- `edit.allow_globs/deny_globs` 为相对 workspace root 的 glob；建议默认把 `.git/**`、`.omne_agent_data/config.toml`、`.omne_agent_data/config_local.toml`、`.omne_agent_data/spec/**`、`.omne_agent_data/{tmp,data,repos,threads,locks,logs}/**`、`**/.env` 放入 `deny_globs`，避免 agent 自举修改边界/运行时数据或读取 secrets。

### 5.4 示例（节选）

```yaml
version: 1
modes:
  coder:
    description: "实现代码变更；允许 edit + command；仍禁止交互式进程。"
    permissions:
      read: { decision: allow }
      edit:
        decision: prompt
        allow_globs: ["**"]
        deny_globs:
          [
            ".git/**",
            ".omne_agent_data/config.toml",
            ".omne_agent_data/config_local.toml",
            ".omne_agent_data/spec/**",
            ".omne_agent_data/tmp/**",
            ".omne_agent_data/data/**",
            ".omne_agent_data/repos/**",
            ".omne_agent_data/threads/**",
            ".omne_agent_data/locks/**",
            ".omne_agent_data/logs/**",
            "**/.env",
          ]
      command:
        decision: prompt
      process:
        inspect: { decision: allow }
        kill: { decision: prompt }
        interact: { decision: deny }
      artifact: { decision: allow }
      tool_overrides:
        - tool: "file/delete"
          decision: deny
```

> JSON 也是 YAML 的子集：如果你更喜欢 JSON 语法，可以写成 JSON 但仍保存为 `.yaml`，或用 `OMNE_AGENT_MODES_FILE` 指向 `.json`。

---

## 6) 内置默认 modes（最小集）

v0.2.0 内置最小模式（作为没有 project config 时的兜底）：

- `architect`：读为主；仅允许改 `docs/**` + 少数根文件（默认 `prompt`）；可写 artifacts。
- `coder`：实现变更；`edit/command` 默认 `prompt`，依赖 execpolicy 细化白名单。
- `reviewer`：只读；`edit=deny`；只允许安全的检查类命令（execpolicy）。
- `builder`：跑 fmt/check/test/clippy 等 gates；`edit=deny`；命令受 execpolicy 严格限制。

---

## 7) 未来扩展（不在 v0.2.0 强做）

- `prompt_strict`/`escalate`：即使 `auto_approve` 也必须停下来的人类确认（单独的 decision，不要污染 `prompt`；见 `docs/approvals.md` 的 Escalate 草案）。
- per-tool 参数约束（例如 `file/write` 限制路径/大小，`process/start` 限制 cwd/env）。
- 全量的 config layering 与 explain（回答“为什么现在生效的是这个 mode/这个决策”）。

---

## 8) v0.2.0 MVP：已强制执行的边界（实现状态）

> 只写“已经做到了什么”，避免文档把自己写成梦想清单。

- `file/*` 与 `fs/mkdir`：当 `mode` 的 `edit` 对目标路径判定为 `deny` 时，工具调用会被拒绝并落盘 `ToolStatus=Denied`。
- `process/start`：当 `mode` 的 `command=deny` 时，工具调用会被拒绝并落盘 `ToolStatus=Denied`（后续仍会叠加 `sandbox/execpolicy/approval`）。
- `file/read|glob|grep`：当 `mode.read=deny` 或目标路径命中默认 deny globs（如 `.git/**`、`.omne_agent_data/threads/**`）时会被拒绝；`.env` 永远拒绝（避免 secrets 入上下文）。
