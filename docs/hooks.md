# Hooks（SessionStart / PreToolUse / PostToolUse / Stop）（TODO：规格草案）

> 目标：把“安全提醒/环境提示/自动化守门”从 prompt 里剥离出来，变成可版本化、可审计的 hook。
>
> 状态：本文是 **TODO 规格草案**（v0.2.0 未实现）。
>
> 注意：这不是 `docs/workspace_hooks.md` 的 workspace 生命周期 hooks（v0.2.0 已实现；对应 `thread/hook_run`）。

---

## 0) 核心原则（别做成后门）

- hook **不能绕过** `mode gate → sandbox → execpolicy → approval`（见 `docs/modes.md`、`docs/approvals.md`）。
- hook 默认只做两件事：
  1. 生成/更新 artifacts（可预览、可回放）
  2. 注入 `additional_context`（安全提醒/约束提示），并且必须可审计
- 不引入 stdin/PTY 交互；hook 的执行同样遵循非交互进程约束。
- hook 是 **advisory**：它不是安全边界，也不应被当作“强制守门”；真正的边界仍然是 `mode/sandbox/execpolicy/approval`。
- hook 的失败 **不应阻断** 主流程（不得把 allow/prompt 变成 deny；也不得让一个脚本故障把系统搞瘫）。

---

## 1) 配置文件位置与发现顺序（建议写死）

Project config（可提交/可 review）：

- **Canonical**：`./.codepm_data/spec/hooks.yaml`

发现顺序（v1 建议写死，避免隐式执行未 review 的脚本）：

1. `./.codepm_data/spec/hooks.yaml`
2. 内置默认（无 hooks）

fail-closed（写死）：

- 找到文件但解析失败 / schema 校验失败：记录错误并 **禁用 hooks**（不要 silent fallback 到其它文件；也不要继续执行“可能被误解析”的 hooks）。
- 未配置 hooks 文件：视为“hooks 关闭”（不报错）。

---

## 2) Hook points（最小集合）

先只定义最小四个点，避免一次性铺太大：

- `SessionStart`：thread start 或 resume 后、开始第一个 turn 之前。
- `PreToolUse`：某次工具调用即将执行前（已经拿到 tool name + params；但尚未执行）。
- `PostToolUse`：某次工具调用执行完成后（有 status/result）。
- `Stop`：turn 结束时（`TurnCompleted` 写入后；可拿到 `status/reason`）。

备注：

- `Setup` 这类“环境生命周期”请走 `docs/workspace_hooks.md`（不要和这里混在一起）。

---

## 3) Hook 执行语义（建议实现方式）

### 3.1 形式

hook 本质上是“自动触发的执行 + 可选的上下文注入”。建议实现为一个新工具（占位名；命名口径见 `docs/tool_parallelism.md`）：

- JSON-RPC method：`hook/run`
- agent tool id：`hook_run`

要求：

- hook 的执行必须落盘（等价于普通 tool/process 事件：started/completed + stdout/stderr artifacts）。
- hook 的每一步命令执行仍走 `process/start`，并受 `execpolicy` 约束（别把 hook 当特权通道）。
- `additional_context` 的注入应发生在 **下一次模型请求** 的上下文构建阶段（同一 turn 内也可以），而不是“去改写已经发生的 tool call”。

### 3.2 输入与输出（可审计）

建议把 hook 输入/输出都落到 thread artifacts 下，避免大 JSON 塞进事件；并且写盘前必须先做脱敏（见 `docs/redaction.md`）：

- input：`<thread_dir>/artifacts/hooks/<hook_id>.input.json`
- output：`<thread_dir>/artifacts/hooks/<hook_id>.output.json`

并通过 env 把路径传给 hook 命令：

- `CODE_PM_HOOK_INPUT_PATH`
- `CODE_PM_HOOK_OUTPUT_PATH`

（进程 stdin 默认为空，因此不要指望从 stdin 读 JSON。）

### 3.3 additional_context 注入（必须可回放）

如果 hook 需要注入 `additional_context`（例如安全提醒）：

1. hook 写入 `output.json`：`{"additional_context": "...", "summary": "..."}`。
2. 系统把 `additional_context` 写成一个 user artifact（`artifact_type="hook_context"` 或更具体类型），并在事件里记录该 `artifact_id`。
3. 模型输入只引用这份 artifact 的内容（或其摘要），从而保证：
   - 注入内容可在历史里被定位/审计
   - 注入内容可以走统一脱敏（见 `docs/redaction.md`）
   - 注入内容必须有大小上限（建议 8–16KiB）；超出上限只保留摘要并把全文留在 artifact
   - 注入内容视为不可信输入：只能作为“额外提示/约束”，不能作为权限边界或自动放行依据

### 3.4 递归与失败语义（写死边界）

- **禁止递归**：hook 的执行过程不应再次触发 hooks（否则 `PreToolUse` 很容易把自己递归调用到爆炸）。最小实现可以是：执行 hook 时临时禁用 hook dispatch（只记录事件，不再触发新的 hook）。
- **失败不改权限**：hook 失败只会产生可见的失败记录（事件/日志/可选报告 artifact），不得改变主流程的 allow/prompt/deny 判定结果。
- **失败不阻断**：hook 失败不应阻断当前 tool call/turn（最小：记录失败 + 可选把 thread 标记为需要注意）。

---

## 4) `hooks.yaml`（最小草案）

```yaml
version: 1
hooks:
  session_start:
    - argv: ["echo", "repo rules: ..."]
      ok_exit_codes: [0]
      emit_additional_context: true
  pre_tool_use:
    - when_tools: ["process/start", "file/write", "file/patch", "file/edit"]
      argv: ["python3", ".codepm_data/spec/hooks/security_reminder.py"]
      ok_exit_codes: [0]
      emit_additional_context: true
  stop:
    - when_turn_status: ["stuck", "failed"]
      argv: ["echo", "suggestions..."]
      ok_exit_codes: [0]
      emit_additional_context: true
```

字段约定（建议）：

- `argv`：数组形式 argv（禁止单字符串 shell 拼接）。
  - 建议避免 `sh -c` / `bash -lc` 这类“解释执行字符串”；把逻辑写进脚本文件并直接执行更可审计。
- `ok_exit_codes`：允许的 exit code 列表（可选；默认 `[0]`）。
- `when_tools`：仅对特定工具触发（可选）。
- `when_turn_status`：仅在特定 turn 状态触发（可选）。
- `emit_additional_context`：是否把输出写入 user artifact 并注入模型（可选；默认 false）。
- 未知字段：直接报错（fail-closed）。

---

## 5) 验收（未来实现时）

- hook 触发必须可在事件流里回放：能定位 `hook_kind + hook_id + 相关 artifacts`。
- `PreToolUse` 注入的上下文在 `pm inbox`/`pm thread events` 的历史里可追溯（至少能从 artifact 列表找到）。
- hook 永远不能提升权限：被 `mode/sandbox/execpolicy` 拒绝的动作，不会因为 hook 而变成 allow。
- hook 不会递归触发自身：`hook/run`（以及 hook 内部的 `process/start`）不会再次触发 `PreToolUse/PostToolUse`。
