# 特殊指令（slash / at）（TODO：规格草案）

> 目标：把“用户想表达的控制意图/上下文引用”从纯文本里剥离出来，变成 **结构化、可审计、可回放** 的输入。
>
> 状态：本文是 **TODO 规格草案**（v0.2.0 未实现）。现状 `turn/start` 只有 `input: String`，无法表达结构化指令。

---

## 0) 范围与非目标

范围（未来实现）：

- 定义最小一套 **slash 指令** 与 **at 引用** 的语义与协议表示。
- 明确它们与现有 JSON-RPC 的映射（不要让模型“猜”）。

非目标（先别碰）：

- 不设计一门完整语法/DSL；只是最小约定。
- 不承诺 UI/TUI 交互细节；这里只定义控制面语义。
- 不做复杂冲突解析/优先级系统（保持简单）。

---

## 1) 为什么要做（别让模型当解析器）

如果把 `/plan`、`@file` 这类东西塞进用户文本：

- 服务端/客户端只能靠“文本解析”猜意图，容易歧义；
- 更糟：你会逼模型去理解并执行这些语义（不可审计、不可强制）。

正确的方向是：

- **客户端解析 → 结构化表达 → 服务端执行/落盘**（见 `docs/thread_event_model.md`）。

---

## 2) 指令分类（最小集）

### 2.1 slash：控制面指令（不进模型）

这类指令是“控制面动作”，应直接映射到 JSON-RPC（结构化、可审计），不应作为 `turn/start` 文本输入的一部分。

最小建议：

- `/approve <approval_id> [remember=true|false]`
  - 映射：`approval/decide { approval_id, decision=approved, remember? }`
  - 关联规范：`docs/approvals.md`

> 备注：`/deny`、`/interrupt`、`/pause` 等可以作为扩展（优先用现有 RPC：`turn/interrupt`、`thread/pause`…）。

### 2.2 at：上下文引用（进模型，但不靠文本解析）

这类指令是“上下文引用”，本质是让系统把外部对象拉进上下文构建流程，而不是让模型靠字符串猜文件路径。

最小建议（只定义两个，别扩面）：

- `@file <path>[:start_line[:end_line]]`
  - 语义：把文件（或片段）作为上下文引用。
  - 实现建议：服务端执行 `file/read`（受 `mode/sandbox` 约束），并把内容（或摘要）加入模型输入。
  - 安全边界（写死）：
    - `path` 以 thread cwd（workspace root）为基准解析；禁止绝对路径、`..` 路径穿越与 symlink 逃逸（具体边界由 sandbox 保证）。
    - 行号参数必须是正整数；`end_line` 若存在则必须 `>= start_line`。
    - 建议对读取的最大 bytes/行数做上限：超过上限时写入 artifact 并只注入“摘要 + path + 定位信息”，避免把大文件直接塞进上下文。
- `@diff`
  - 语义：把“当前 workspace diff”作为上下文引用。
  - 实现建议：通过 `process/start argv=["git","diff","--"]` 生成 stdout artifact，再把摘要/路径注入上下文（避免把巨量 diff 直接塞进 prompt）。
  - 安全边界（写死）：
    - argv 固定，不接受用户拼接参数。
    - 注入前必须做脱敏（见 `docs/redaction.md`），默认只注入摘要与 artifact 位置。

关联规范：

- 文件工具：`docs/modes.md`（read/edit 边界）
- 产物定位：`docs/runtime_layout.md`、`docs/artifacts.md`

---

## 3) 协议表达（TODO：建议路线）

现状：

- `crates/app-server-protocol/src/lib.rs::TurnStartParams` 只有 `input: String`
- `crates/agent-protocol/src/lib.rs::ThreadEventKind::TurnStarted` 只有 `input: String`

这迫使系统走“文本解析”或“模型理解”，不符合可审计要求。

### 3.1 最小扩展（建议）

建议扩展 `turn/start` 的 params（向后兼容）：

- 继续保留 `input: String`（旧客户端可用）
- 增加可选字段（v1 最小只需要 context refs）：
  - `context_refs?: [ContextRef]`
  - `directives` 预留为未来扩展（v1 不实现；避免过早锁死 /plan 等语义）

只写死最小约定的 `kind`（v1）：

- `context_refs.kind="file"` / `"diff"`

建议把 payload 结构写死（避免 `any` 扩散到实现里；fail-closed）：

- `context_refs.kind="file"`：`payload={ path: string, start_line?: int, end_line?: int }`（行号从 1 开始）。
- `context_refs.kind="diff"`：`payload={}`。

结构定义（v1，建议）：

- `ContextRef`：
  - `{ kind: "file", payload: { path: string, start_line?: int, end_line?: int } }`
  - `{ kind: "diff", payload: {} }`

校验与错误处理（写死）：

- 未知 `kind`：直接拒绝（返回可诊断错误；不要静默忽略）。
- payload 出现未知字段：直接拒绝（避免 typo 静默）。
- `context_refs` 的处理顺序必须保留（不做隐式去重/重排）。

请求示例（占位，JSON-RPC params）：

```json
{
  "input": "please include context",
  "context_refs": [
    { "kind": "file", "payload": { "path": "crates/core/src/lib.rs", "start_line": 1, "end_line": 80 } },
    { "kind": "diff", "payload": {} }
  ]
}
```

未知 `kind` 的处理建议：

- 默认拒绝并返回可诊断错误（避免静默忽略导致误解）。

### 3.2 回放可审计性（必须）

仅在请求参数里带 `directives/context_refs` 还不够——必须能回放。

建议二选一（TODO），v1 推荐选 1（最小改动）：

1. 扩展 `ThreadEventKind::TurnStarted`，把结构化输入一起落盘。
2. 在 `TurnStarted` 前追加一个新事件（例如 `TurnInputStructured`），用 `turn_id` 关联。

无论选哪条，原则不变：

- **事件流是唯一真相**（见 `docs/thread_event_model.md`）。

---

## 4) `/plan`（intent）最小语义（TODO）

问题：`/plan` 不是“跑一个 RPC 就完事”，它是“下一轮 turn 的意图”。

最小建议（TODO）：

- 这需要未来启用 `turn/start` 的结构化 `directives` 字段（见 3.1）。
- `directives.kind="plan"` 表示：本 turn 的目标是“生成 plan artifact”，并尽量避免 side effects。
- 服务端可选做两件事（别过度设计）：
  1. 临时将 mode 切到更保守的 `architect`（或等效规则）
  2. 强制关闭并行与 side-effect tools（取决于 modes/allowed-tools 的落地）

产物口径：

- plan 作为 user artifact：`artifact_type="plan"`（见 `docs/artifacts.md`）。

---

## 5) 验收（未来实现时）

- `turn/start` 能接收结构化 `directives/context_refs`（旧客户端仍可只发 `input`）。
- 结构化输入能在事件流里回放定位（不能只存在于瞬时请求）。
- `@file` 会产生一次 `file/read` 的工具事件，并且受 `mode/sandbox` 限制。
- `@diff` 会产生一次 `process/start`（git diff），stdout 落盘到 artifacts，并且上下文注入是“摘要 + 路径”，不是整段巨量 diff。
