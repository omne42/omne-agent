# Workflow / Commands（Markdown + frontmatter）

> 目标：把“常用工作流”做成可版本化、可 review、可复用的 spec 文件，而不是散落在 prompt 里。
>
> 状态：v0.2.x 已落地 **v1 最小子集**（`omne command {list,show,validate,run}`）与 `--fan-out` 最小并发拆分；最小联动已落地（见第 4 节），并已支持“依赖 + 任务优先级（high/normal/low）+ aging 公平调度（最小版）”。

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
- 不做复杂调度（例如抢占式/配额式公平调度、失败重试策略等）；v1 只提供最小可用的 fan-out/fan-in。

---

## 1) 文件位置与发现顺序（建议写死）

Project config（可提交/可 review）：

- v1 **只支持**：`./.omne_data/spec/commands/<name>.md`（不做 fallback/模糊搜索；找不到即报错）
- `omne init` 默认会创建 `./.omne_data/spec/commands/` 并写入最小模板（`plan.md`、`fanout-review.md`）；可用 `omne init --no-command-templates` 跳过。
- 同一次 `omne init` 还会默认生成 `./.omne_data/spec/workspace.yaml`、`./.omne_data/spec/hooks.yaml` 与 `./.omne_data/spec/modes.yaml` 空模板，便于后续补齐 workspace/hook/mode 配置；可用 `--no-workspace-template` / `--no-hooks-template` / `--no-modes-template` 单独跳过，或使用 `--no-spec-templates`（等价 `--minimal`）一键全部跳过。

CLI 形态（已实现）：

```bash
omne command list
omne command show <name>
omne command validate [--name <name>] [--strict] [--json]
omne command run <name> --var key=value
```

---

## 2) Frontmatter（最小字段）

文件必须以 YAML frontmatter 开头：

```yaml
---
version: 1
name: commit-push-pr
mode: coder
show_thinking: true
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
- `mode`：默认 mode（`architect/coder/reviewer/builder/debugger/merger`），用于选择权限边界（见 `docs/modes.md`）。
- `show_thinking`：是否展示模型 thinking/reasoning 流式（可选；默认 true；适合某些“只想看结论”的 workflow 关闭）。
- `allowed_tools`：额外的“最小权限”收口（可选）：
  - 语义是 **再收紧**：不在列表里的工具一律 `deny`（即使 mode 允许，也要拒绝）。
  - 与 mode 的合并语义：先算 mode gate，再与 `allowed_tools` 取交集（deny 优先）。
  - `allowed_tools: []` 表示 deny all（用于“只渲染文本，不允许任何工具/命令”的 workflow）。
  - 列表包含未知工具名（或包含 mode 本就不允许的工具）应直接报错（fail-closed），避免“看起来允许，实际 silently ignore”。
  - 实现落点：在 frontmatter sanitize 阶段前置校验（`command list/validate/run` 共用），不是运行时静默过滤。
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

## 4) Orchestrator 并发拆分（v0.2.x 最小落地：`omne command run --fan-out`）

当要把一个 workflow 拆成并发子任务时，我们只支持**一个简单约定**（别发明 DSL）：

- `## Task: <id> <title>` 作为 task 边界。

v0.2.x 行为（已实现）：

- CLI：`omne command run <name> --fan-out`
- 解析：对渲染后的 command body 扫描 `## Task:` 段落，提取 `task_id/title/body`。
  - 依赖（最小语法）：若 task body 的第一条非空行是 `depends_on: <id1,id2,...>`（或 `depends-on:`），则该 task 仅在依赖任务完成后才会启动；该行不会传给子任务 prompt。
  - 优先级（最小语法）：在 task body 的前导指令区支持 `priority: high|normal|low`；未声明默认 `normal`。可与 `depends_on` 组合使用。
  - 公平调度（最小实现）：fan-out 调度会维护 ready task 的等待轮次（aging），等待越久会逐步提升有效优先级，降低低优先级任务长期饥饿风险；同有效优先级按声明顺序。
    - 参数：`OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS`（默认 `3`；每等待 N 轮提升一级优先级；范围 `1..=10000`，无效值回退默认）。
    - 对齐：同一参数也用于服务端 `agent_spawn` 调度路径，保证 CLI fan-out 与服务端 fan-out 的公平语义一致。
- fan-out：每个 task 会通过 `thread/fork` 创建子 thread，并强制配置为 `sandbox_policy=read_only` + `mode=reviewer`，然后 `turn/start` 并发执行。
  - 并发上限：复用 `OMNE_MAX_CONCURRENT_SUBAGENTS`（默认 `4`；`0` 表示不限制）。
  - turn priority：fan-out 子任务使用 `priority=background`；全局 LLM worker pool 会为 foreground 预留并发额度（`OMNE_MAX_CONCURRENT_LLM_REQUESTS`/`OMNE_LLM_FOREGROUND_RESERVE`）。
  - 重要限制：由于 v0.2.x 还没有 workspace 隔离，fan-out 子任务只能做并发只读分析（读文件/索引/事件），不能写代码/跑命令。
- fan-in：父 thread 会先创建一个 `artifact_type="fan_in_summary"`，fan-out 期间持续更新进度（含 rough ETA）；待所有子任务 `TurnCompleted` 后写入最终汇总（包含每个 task 的 `thread_id/turn_id/artifact_id/artifact_error/error_artifact_id/status` 与最后一条 `AssistantMessage`），并为失败任务提供 `omne artifact read <thread_id> <artifact_id>` 快捷查看命令。
  - fan-out 进度/汇总 artifact 都会包含调度参数段：`env_max_concurrent_subagents`、`effective_concurrency_limit`、`priority_aging_rounds`，便于复盘“为何这样调度”。
- 当 fan-out 在主 turn 运行期间更新这些 artifact 时，会把主 turn 的 `turn_id` 一并写入 `artifact/write`，提升 provenance 可追溯性（离线重放时可直接反查来源 turn）；该口径覆盖 `fan_in_summary` 的 progress/final 更新，以及 `fan_out_linkage_issue` / `fan_out_linkage_issue_clear`。
- 若子任务结果锚点 `artifact_type="fan_out_result"` 写入失败，会在父 thread 额外写 `artifact_type="fan_out_result_error"` 记录失败上下文（`task_id/thread_id/turn_id/status/error`），并把错误回填到 fan-in 汇总的 `artifact_error`。
  - 当该错误发生在主 turn 运行期间（如 `--fan-out-early-return`），`fan_out_result_error` 也会携带主 turn 的 `turn_id`，便于统一 provenance 追溯。
- 对 `fan_out_result_error`，fan-in 汇总会额外给出 `result_error_read_cmd` 与 “Result Artifact Error Quick Reads” 快捷命令区。
- 若运行中触发联动中断（linkage issue），fan-in 汇总会追加 “Fan-out Linkage Issue” 段落，记录阻断原因文本。
- linkage issue 也会单独写 `artifact_type="fan_out_linkage_issue"`（触发 `AttentionMarkerSet{marker=fan_out_linkage_issue}`），便于 `thread/attention` / `thread/list_meta` 机器可读消费。
- fan-out 成功收口后会写 `artifact_type="fan_out_linkage_issue_clear"`，触发 `AttentionMarkerCleared{marker=fan_out_linkage_issue}`，避免旧 marker 残留。
- 主 turn：fan-in 完成后，仍会继续执行原 `omne command run` 的主 turn，并在输入中附带 `fan_in_summary` 的定位信息（便于后续 `omne artifact read`）。
- “提前返回”策略：`omne command run <name> --fan-out --fan-out-early-return` 会在子任务未全部完成时先启动主 turn，并持续更新 `fan_in_summary`。
- 强联动（已实现，默认开启）：
  - 非 early-return：若任一子任务以非 `Completed` 结束，主 turn 会被阻断（直接失败并提示 `fan_in_summary` 定位信息；若存在 `fan_out_result_error`，错误中会附 `artifact_error_read_cmd`）。
  - early-return：若子任务出现 `need_approval` 或以非 `Completed` 结束，系统会自动 `turn/interrupt` 主 turn，并在收口后返回错误（若存在 `fan_out_result_error`，错误中会附 `artifact_error_read_cmd`）。
  - 可通过 `OMNE_COMMAND_FAN_OUT_REQUIRE_COMPLETED=0` 关闭“必须 completed”约束（改为仅做提示，不阻断）。
  - 子任务触发 `ApprovalRequested` 时，父 thread 的 `fan_in_summary` 会立即写入可操作句柄（`task_id/thread_id/turn_id/approval_id/action`）与推荐命令：`omne approval decide <thread_id> <approval_id> --approve`。
  - 依赖传播：若某 task 的依赖以非 `Completed` 结束，则该 task 不会启动，并在 `fan_in_summary` 中标记为 `Cancelled` + `dependency_blocked=true`（`thread_id/turn_id` 为空）；Structured Data JSON 会额外带 `dependency_blocker_task_id` 与 `dependency_blocker_status` 便于脚本直接消费，Markdown 明细段也会显示这两个字段；`omne artifact read` 的 CLI notice 也会带出首个 blocker 的 `task_id/status` 摘要。

非目标（仍 TODO）：

- 更复杂公平调度（例如配额、抢占、跨 turn 的全局公平）；当前只实现了 task 级 aging。

---

## 5) 验收（v0.2.x 现状）

- `omne command list` 能发现 `./.omne_data/spec/commands/*.md`，并显示 `name/mode/version`；若某些 command frontmatter 非法，不会阻断其它条目列举，并会汇总 parse 错误。
  - `--json` 输出包含 `ok`、`item_count`、`command_count`、`error_count` 与 `commands_dir`，用于脚本快速判读与定位。
  - `--json.errors[*].error_code`（可选）为机器可读错误分类；`modes_load_error`（可选）用于暴露 mode 配置加载异常（不影响 builtin mode 兜底时的继续执行）。
- `omne command validate` 支持全量或 `--name` 单文件校验；任一校验失败返回非 0；`--strict` 会把重复 command name 视为错误（适合 CI gate）。
  - `--json` 输出包含稳定字段：`target`（`"all"` 或指定 command 名）、`commands_dir`（扫描目录）、`item_count`、`validated_count`、`error_count`，便于脚本消费。
  - `--json.errors[*].error_code`（可选）为机器可读错误分类；`modes_load_error`（可选）用于暴露 mode 配置加载异常。
  - fail-closed：`mode` 不存在、`allowed_tools` 含未知工具、或 `allowed_tools` 与 mode（含 tool_overrides）不兼容时，必须直接报错。
- `omne command run <name>` 必须把最终生效的 `mode/allowed_tools` 记录到 thread config（可解释性见 `thread/config/explain`）。
  - 若 mode 配置加载失败但 builtin mode 兜底成功，CLI 会输出 `[command/run modes load warning] ...` 告警，避免静默降级。
  - 执行失败时，CLI 会额外输出 `[command/run error_code] <code>` 机器可读错误分类（例如 `command_var_missing_required`、`context_step_failed`、`fan_out_linkage_issue`）。
- `omne command show <name>` 在 `--json` 下会附带可选字段 `modes_load_error`（仅在 mode 配置加载失败时出现）；非 JSON 输出也会打印 `[command/show modes load warning] ...`。
- `context` 步骤必须全部事件化（`ProcessStarted/Exited`），且输出可从 artifacts 定位（不塞进事件）。
- fail-closed：任一 context step exit code 不在 `ok_exit_codes` 内时必须终止执行，且错误原因可在事件/日志中定位。
