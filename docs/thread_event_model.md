# Thread/Turn/Item 与事件落盘（v0.2.0 口径）

> 目标：把系统的“发生了什么”建模成可回放的事件流。UI/CLI 只是投影，**落盘事件才是唯一真相**。

---

## 0) 核心原则

- **append-only**：事件只追加，不覆盖；回放（replay）必须能重建状态。
- **单调序号**：同一 thread 内 `EventSeq` 单调递增，用于订阅续传与去重。
- **订阅是优化，不是正确性前提**：掉线/lag 后用 `since_seq` 重放补齐（at-least-once，客户端按 `seq` 去重）。
- **不把大内容塞进事件**：文件内容/patch/diff/长日志属于 artifacts；事件只记录元信息（path/bytes/ids/paths）。
- **允许并发导致的事件交错**：read-only tool calls 可能并发执行，`ToolStarted/ToolCompleted` 的出现顺序可能交错；以 `seq` 排序回放即可（并发口径见 `docs/tool_parallelism.md`）。

---

## 1) 现实现状：ThreadEvent 是最小原子

落盘事件：

- `ThreadEvent { seq, timestamp, thread_id, kind }`
- `kind` 为强类型枚举（见 `crates/agent-protocol/src/lib.rs::ThreadEventKind`）。

Turn 的边界：

- `TurnStarted { turn_id, input }` 开始一个 turn。
- `TurnCompleted { turn_id, status, reason? }` 结束一个 turn。
- 其它与 turn 相关的事件通过 `turn_id: Option<TurnId>` 关联（例如 tool/approval/message/process）。

“Item” 的工作定义（v0.2.0）：

- Item 是 timeline/UI 里的展示单元。
- v0.2.0 实现里 **Item ≈ 可展示的 `ThreadEventKind`**（并辅以 JSON-RPC `item/*` notifications 做流式体验）。

---

## 2) Item 覆盖：映射表（v0.2.0）

| 概念 Item | 事件/来源 | 说明 |
| --- | --- | --- |
| message | `AssistantMessage` | 落盘可回放；包含 `model/response_id/token_usage?` |
| delta（文本流） | JSON-RPC `item/delta` | 来自 Responses SSE `response.output_text.delta`；断线不影响最终 `AssistantMessage` 落盘 |
| tool | `ToolStarted` / `ToolCompleted` | 只记录工具名与参数元信息；结果写入 `result`（避免大 payload） |
| approval | `ApprovalRequested` / `ApprovalDecided` | `ApprovalId` 为 join key；自动决策也落盘 |
| process | `ProcessStarted` / `ProcessExited` / `ProcessInterruptRequested` / `ProcessKillRequested` | stdout/stderr 路径是 artifacts；支持 `tail/follow`（只读 attach） |
| file edit | `ToolStarted/Completed`（`file/write|edit|patch|delete|fs/mkdir`） | 事件记录 `path/bytes/...`，**不记录文件内容**；真实内容在 workspace，可用 diff 工具生成产物 |
| diff | （TODO）artifact + 预览类型 | v0.2.0 没有专门的 diff item；通常通过 `process/start` 运行 `git diff` 并把输出落盘到 stdout artifact |
| reasoning | （TODO）不默认落盘 | 默认不持久化模型推理；如需可用“summary artifact”（脱敏）替代 |

这也是 `docs/v0.2.0_parity.md` 里 “Item 覆盖” TODO 的拆解：**哪些已经有事件表达，哪些只是 UI/preview 的产品化差距**。

---

## 3) 回放与续传语义（最小可用）

- 客户端订阅使用 `since_seq`（从 `since_seq + 1` 推送/重放）。
- 服务端允许重复投递（at-least-once），客户端按 `seq` 去重即可。
- 不引入 ack 作为正确性前提：不丢保证来自落盘 log。

---

## 4) TODO：下一步该补什么（别虚）

- diff/preview：把 “diff 预览”收敛到 artifact（不把大内容塞进事件）：
  - 最小路线：`process/start argv=["git","diff","--"]` → stdout artifact（现状可用，但 UI 很难做强预览）。
  - 目标路线（TODO）：把 diff 输出写成 user artifact，并在 metadata 里标 `preview.kind="diff_unified"`（见 `docs/artifacts.md`）。
- reasoning：如果要落盘，只存**可审计的摘要**（summary/stuck_report 等），并走脱敏（不要把 secrets 送进历史）。
- item id：如要把 UI 的 item/started/completed 与落盘进一步对齐，可引入 `item_id/item_kind`（但先证明有必要）。

---

## 5) 快速自检（实现/文档一致性）

```bash
rg -n \"enum ThreadEventKind\" crates/agent-protocol/src/lib.rs
rg -n \"item/delta\" crates -S
```
