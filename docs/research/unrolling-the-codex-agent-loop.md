# Unrolling the Codex agent loop（外部文章笔记）

> Source: `https://openai.com/index/unrolling-the-codex-agent-loop/`
>
> Published: **2026-01-23**
>
> Accessed: **2026-01-28**
>
> Scope: Codex CLI harness / Responses API agent loop / 性能（prompt caching, compaction）/ ZDR 取舍。

---

## 0) 结论先行：对我们这组仓库的帮助

- 这篇文章把 **Codex CLI 的 agent loop**（模型 ↔ 工具 ↔ 再采样）拆成了可复用的工程约束：**stateless 请求 + prompt 前缀稳定 + context window compaction**。
- 对我们当前目录下的仓库组合来说，它最直接的价值是把“该优化哪里”讲得很硬：
  - 性能主要靠 **prompt caching**（要求 exact prefix）；不要靠“少发点 JSON”来赌收益。
  - 为了缓存命中，要把“会抖动的东西”变成 **追加的新 message item**，而不是重写历史 prompt。
  - 为了长对话持续可用，要有 **可回放的 compaction**（不仅是 UI 上的 summary）。

---

## 1) 文章要点（用于对齐设计，不追求逐字复述）

### 1.1 Unrolled / Stateless 的 agent loop

- 每次调用 `/responses` 都发送完整的 `input` items（包括历史 message、tool call、tool output）。
- 工具输出作为新的 item 追加到 `input`，使旧 prompt 成为新 prompt 的 **exact prefix**，以便利用 prompt caching。

### 1.2 性能取舍：quadratic payload vs prompt caching

- 文章明确指出：这种“不断追加 items”的方式，累计发送的 JSON 是 **quadratic**。
- 虽然 Responses API 提供 `previous_response_id` 来减少 payload，但 Codex 目前不使用它，主要原因：
  - 保持 requests fully **stateless**（对 provider 更简单）
  - 支持 **Zero Data Retention (ZDR)**（避免为 `previous_response_id` 在服务端存储会话数据）
- Codex 把效率重点放在 **prompt caching**：cache hit 时采样成本更接近线性；cache miss 的典型来源包括：
  - tools 列表/顺序变化
  - model 变化
  - sandbox/approval/cwd 变化
- 为减少 cache miss：当配置变更发生在长对话中，倾向于 **追加一个新 message item** 表达变化，而不是修改历史 item。

### 1.3 Context window 管理：compaction + opaque state

- token 逼近阈值时，需要 compaction（压缩 `input` items）。
- Codex 从“手工 `/compact` + summarization”演进到使用 `/responses/compact`：
  - 返回可替代旧 `input` 的新 items
  - 包含 `type=compaction` item，携带 opaque `encrypted_content` 以保留模型的 latent understanding
  - 在 auto compact limit 超过时自动触发

---

## 2) 映射到我们当前目录的仓库（现状）

> 以下描述以“从各仓库根目录运行命令”为准。

### 2.1 `codex_pm/`：已经具备 unrolled loop 的主要骨架

- agent loop：`crates/app-server/src/agent/tool_loop.rs`
  - 流式采样（`ditto_llm::LanguageModel::stream`）
  - 收集 `TextDelta` 与 tool call delta，拼回 OpenAI Responses 的 raw JSON item（`serde_json::Value`）
  - 执行工具后将 `FunctionCallOutput` 追加进 `input_items` 再采样（与文章思路一致）
- 可回放“唯一真相”：`docs/thread_event_model.md`（append-only JSONL events）
- budget/summary：`docs/budgets.md`
  - 已有 token/steps/time/tool_calls budget
  - 已有 auto summary（本质是 compaction 的一个实现，但不是 `/responses/compact` 的 opaque items 路线）
- 工具并发（只读）：`docs/tool_parallelism.md`（并发执行 read-only tool calls）
- 脱敏与 env scrub：`docs/redaction.md`
  - 由于 tool 输出会进入 prompt/事件/产物，脱敏边界需要“写死”

### 2.2 `ditto-llm/`：provider 抽象层，但缺少缓存可观测性

- OpenAI Responses 已集成：`src/providers/openai.rs`
- 当前 `ditto_llm::Usage` 只有 token totals，没有 `input_tokens_details.cached_tokens` 等字段：
  - 这会直接影响我们验证 prompt caching 命中率（文章把它当作主要优化手段）
  - 如果要对齐文章的“缓存是一等性能指标”，需要把 usage details 从 provider 层透传出来

### 2.3 `mcp-kit/`：MCP 动态工具会直接影响 cache hit

- 默认 `TrustMode::Untrusted` 强化了安全边界（这是好事），但也意味着：
  - tools 列表需要稳定、可预期（否则 cache miss）
  - 对 `notifications/tools/list_changed` 的处理策略要明确：是否允许 mid-conversation 更新 tools，如何避免工具枚举顺序抖动

### 2.4 `safe-fs-tools/`：把“工具层安全模型”做成第一等概念

- 文章里的 agent loop 默认假设“模型可调用工具、工具可影响本地环境”；这需要硬边界。
- `safe-fs-tools` 的 `SandboxPolicy`/`Root`/`SecretRules`/`limits` 非常适合成为：
  - CodePM 的默认文件系统工具底座（比“随便读写”更可审计、更易控）

### 2.5 `code-checker/`：可以当作“只读分析类工具”的案例

- 其功能天然偏 read-only（扫描/报告），适合被纳入 “read-only tools 并发” 与 “缓存友好（工具定义稳定）” 的约束体系中。

---

## 3) 差距与风险（相对文章的工程约束）

- **两条 loop 的 item 完整度不同**：
  - 默认 loop（`ditto_llm::LanguageModel::stream`）只会生成 `message/function_call/function_call_output` 等基础 item。
  - Codex parity loop 走 raw `/responses` stream + `/responses/compact`，可以 round-trip 任意 item（含 `compaction`/`encrypted_content`），但目前只在 reasoning provider 且 `CODE_PM_OPENAI_RESPONSES_CODEX_PARITY=1` 时启用。
- **compaction 路线不同**：
  - 我们的 auto summary 是“生成一段 summary 文本并重建 system message + tail items”
  - 文章强调的 `/responses/compact` 会返回 items（含 opaque state）→ 对 OpenAI 的 latent state 保留更强
- **role 语义差异**：
  - 文章强调 `system`/`developer`/`user`/`assistant` 的优先级
  - 我们在 `response_items_to_ditto_messages()` 对未知 role 会降级成 user；若未来要插入 `developer`（例如 permissions/sandbox 变化），需要修正映射
- **缓存可观测性缺失**：
  - 没有 cached_tokens 等指标，就无法判断“我们是否真的在 cache hit 上省钱”，容易把优化做成玄学

---

## 4) 推荐落地路线（按风险与收益排序）

### Option A（先做）：对齐“可观测性 + 约束”

- 把“prompt caching 友好”变成明确约束：
  - tools 的枚举顺序稳定
  - model/sandbox/approval/cwd 变化用“追加 message item”表达（不修改历史 input）
- 在 `ditto-llm`/`codex_pm` 打通 token usage details（至少 cached_tokens）用于观测与回归

### Option B：补齐 Responses items 的 raw round-trip（为 ZDR/compaction 铺路）

- 引入可 round-trip 的 item 表示（例如 raw JSON item），为未来支持 `type=compaction`/`encrypted_content` 留接口。

### Option C：双通道（默认 stateless，可选 `previous_response_id`）

- 非 ZDR/允许 provider 存储会话时走 `previous_response_id` 减少 payload；否则保持 stateless。
- 风险在于分支增多，且会把 provider 状态耦合到客户端逻辑里。

---

## 5) 快速核对（可复制命令）

```bash
# 1) CodePM: agent loop / auto summary / tool 并发
cd codex_pm
rg -n "ToolLoop|run_llm_stream_once|auto_compact_summary" crates/app-server/src/agent -S
rg -n "PARALLEL_TOOL_CALLS|read-only" docs/tool_parallelism.md docs -S

# 2) Ditto-LLM: OpenAI Responses provider + Usage
cd ../ditto-llm
rg -n "responses|/responses/compact|struct Usage" src/providers/openai.rs src/types/mod.rs -S

# 3) MCP: trust + tools 变更
cd ../mcp-kit
rg -n "TrustMode|list_changed|tools" -S
```

---

## 6) CodePM 当前实现：OpenAI Responses Codex Parity（选择 Option B）

为了在 **OpenAI `/responses`** 下做到与 Codex CLI 同一条逻辑链（raw items + prompt caching + `/responses/compact`），我们在 `codex_pm/` 增加了一条“并行历史通道”：

- **Raw history 文件（source-of-truth）**：每个 thread 一个 `openai_responses_history.jsonl`
  - 位置：thread 目录下（由 `pm_core::ThreadStore` 管理）
  - 记录形态：append-only records（`item` / `compacted` replacement）
  - 文件：`crates/app-server/src/agent/openai_history.rs`
- **Raw Responses 流式调用**：直接以 raw `input` items 调用 `/responses`，并保存 `response.output_item.done` 的原样 JSON
  - 文件：`crates/openai/src/responses.rs`
- **工具输出不再强制脱敏（仅限 parity 路径）**
  - `run_tool_call(..., redact_output: bool)`
  - legacy 路径保持 `true`；parity 路径传 `false`
  - 关键文件：`crates/app-server/src/agent/tools/dispatch.rs`、`crates/app-server/src/agent/tool_loop.rs`
- **开关与默认**
  - env：`CODE_PM_OPENAI_RESPONSES_CODEX_PARITY`（默认 `true`）
  - 仅当 provider 使用 Responses（`capabilities.reasoning=true`）时生效

⚠️ 风险提示：parity 路径会把更多“原始上下文”写进本地 history（包括 tool outputs）。目前依赖文件权限（unix 下 `0600`）来降低风险；若要做到 Codex PR #1641 的级别（加密存储 + 内存密钥 + ZDR 生命周期），需要进一步演进本地存储层。
