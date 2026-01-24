# 特殊指令（slash / at）

> 目标：把“用户想表达的控制意图/上下文引用”从纯文本里剥离出来，变成 **结构化、可审计、可回放** 的输入。
>
> 状态：v0.2.0 已实现最小 at 引用（`@file`/`@diff`）→ `turn/start.context_refs`，并将结构化输入落盘到 `ThreadEventKind::TurnStarted.context_refs` 以便可审计/可回放；同时实现最小 attachments（`@image`/`@pdf`）→ `turn/start.attachments` → `ThreadEventKind::TurnStarted.attachments`，用于把图片/文件作为结构化输入注入模型。slash 指令（`/plan`/`/approve` 等）仍以 CLI/RPC 为主（本文保留 TODO 占位）。

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

v0.2.0 最小建议（先钉死 4 个 at，别继续扩面）：

- `@file <path>[:start_line[:end_line]]`
  - 语义：把文件（或片段）作为上下文引用。
  - v0.2.0 实现：CLI 从输入开头解析并剥离该行，转为 `turn/start.context_refs`；agent turn 开始前执行 `file_read`（受 `mode/sandbox/approval` 约束）并把内容注入到模型输入（system message）。
  - 安全边界（写死）：
    - `path` 以 thread cwd（workspace root）为基准解析；禁止绝对路径、`..` 路径穿越与 symlink 逃逸（具体边界由 sandbox 保证）。
    - 行号参数必须是正整数；`end_line` 若存在则必须 `>= start_line`。
    - 文件内容注入前会做脱敏（见 `docs/redaction.md`），并受 `max_bytes` 上限约束。
- `@diff`
  - 语义：把“当前 workspace diff”作为上下文引用。
  - v0.2.0 实现：agent turn 开始前执行 `thread_diff`（固定 argv 的 git diff）生成 diff artifact；默认只向模型注入 artifact 元信息（避免把巨量 diff 直接塞进 prompt），需要全文时可 `artifact_read`。
  - 安全边界（写死）：
    - argv 固定，不接受用户拼接参数。
    - 注入前必须做脱敏（见 `docs/redaction.md`），默认只注入摘要与 artifact 位置。
- `@image <path|url>`
  - 语义：把图片作为输入附件（image attachment）。
  - v0.2.0 实现：CLI 从输入开头解析并剥离该行，转为 `turn/start.attachments`；agent turn 开始前将附件注入为模型输入的 image content part（provider 支持时）。
  - 安全边界（写死）：
    - `path` 的解析与 sandbox 边界与 `@file` 一致；并额外拒绝 `.env`。
    - local path 会在 agent side 读取文件并 base64 编码后发送；必须满足：
      - thread `allowed_tools` 包含 `file/read`；
      - 当前 mode 对该路径的 read 权限为 allow（含 `tool_overrides.file/read` 合并）。
    - size 上限由 `CODE_PM_AGENT_MAX_ATTACHMENT_BYTES` 强制（单文件上限）。
    - count 上限由 `CODE_PM_AGENT_MAX_ATTACHMENTS` 强制（每 turn 上限）。
- `@pdf <path|url>`
  - 语义：把 PDF 作为输入附件（file attachment，`media_type="application/pdf"`）。
  - v0.2.0 实现：同 `@image`，但注入为 file content part。
    - local path 默认会读取文件并 base64 编码后发送；
    - 当 `CODE_PM_AGENT_PDF_FILE_ID_UPLOAD_MIN_BYTES>0` 且 PDF 大小达到阈值时，agent 会优先尝试上传到 provider 的 `/files` 获取 `file_id`，并将附件以 `file_id` 形式注入（上传失败会回退到 base64）。
  - 安全边界（写死）：同 `@image`。

关联规范：

- 文件工具：`docs/modes.md`（read/edit 边界）
- 产物定位：`docs/runtime_layout.md`、`docs/artifacts.md`

---

## 3) 协议表达（v0.2.0 已实现）

已实现：

- `crates/app-server-protocol/src/lib.rs::TurnStartParams` 增加 `context_refs?: Vec<ContextRef>`（向后兼容：旧客户端仍可只发 `input`）。
- `crates/app-server-protocol/src/lib.rs::TurnStartParams` 增加 `attachments?: Vec<TurnAttachment>`（向后兼容：旧客户端仍可只发 `input`）。
- `crates/agent-protocol/src/lib.rs::ThreadEventKind::TurnStarted` 增加 `context_refs`/`attachments`（缺省为 `None`，兼容旧日志回放）。

### 3.1 `turn/start` params（v0.2.0）

- 保留 `input: String`
- 新增 `context_refs?: [ContextRef]`（顺序保留；未知 kind/字段直接拒绝）
- 新增 `attachments?: [TurnAttachment]`（顺序保留；未知 kind/字段直接拒绝）

`ContextRef`（v0.2.0，JSON 形态）：

- `{ "kind": "file", "path": string, "start_line"?: int, "end_line"?: int, "max_bytes"?: int }`
- `{ "kind": "diff", "max_bytes"?: int }`

请求示例（JSON-RPC params）：

```json
{
  "input": "please include context",
  "context_refs": [
    { "kind": "file", "path": "crates/core/src/lib.rs", "start_line": 1, "end_line": 80 },
    { "kind": "diff", "max_bytes": 1048576 }
  ],
  "attachments": [
    { "kind": "image", "source": { "type": "path", "path": "assets/example.png" } },
    {
      "kind": "file",
      "source": { "type": "url", "url": "https://example.com/file.pdf" },
      "media_type": "application/pdf",
      "filename": "file.pdf"
    }
  ]
}
```

### 3.2 回放可审计性（v0.2.0）

- 结构化 `context_refs`/`attachments` 会随 `TurnStarted` 事件落盘；事件流仍是唯一真相（见 `docs/thread_event_model.md`）。

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

## 5) 验收（v0.2.0）

- `pm` CLI 会从输入开头解析并剥离 `@file`/`@diff`，转为 `turn/start.context_refs`（模型不需要做文本解析）。
- `pm` CLI 会从输入开头解析并剥离 `@image`/`@pdf`，转为 `turn/start.attachments`（模型不需要做文本解析）。
- `turn/start` 接收结构化 `context_refs`（旧客户端仍可只发 `input`）。
- `turn/start` 接收结构化 `attachments`（旧客户端仍可只发 `input`）。
- 结构化输入会随 `TurnStarted.context_refs`/`TurnStarted.attachments` 落盘，可回放定位。
- `@file` 会触发一次 `file_read`，受 `mode/sandbox/approval` 约束，并将内容注入模型输入。
- `@diff` 会触发一次 `thread_diff`（固定 argv 的 git diff → process/start + diff artifact），默认只注入 artifact 元信息，避免把整段巨量 diff 直接塞进上下文。
- `@image`/`@pdf` 会把图片/文件作为 attachment 注入模型输入；local path 读取受 `mode/allowed_tools/sandbox` 约束，并强制 `.env` 拒绝与 size 上限。
