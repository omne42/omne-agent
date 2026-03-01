# OpenAI-compatible Chat 模式的 history 与复用（用于 cache）

## 背景

OmneAgent 目前存在两类“历史”：

- `readable_history.jsonl`：线程内用户可读的对话文本（仅 `user/assistant` 纯文本），由事件派生写入。
- `openai_responses_history.jsonl`：仅 OpenAI Responses 路径（`capabilities.reasoning=true`）使用的 raw item 历史，可选明文/加密落盘与迁移。

当使用 OpenAI-compatible Chat Completions 路径（`capabilities.reasoning=false`）时，虽然也会产生事件并可据此重建上下文，但如果缺少 `assistant_message` 事件，就会同时破坏：

- CLI 输出（`omne ask` 看不到最终回答）
- `readable_history.jsonl` 的 assistant 行
- 下一轮 turn 的上下文复用（也间接影响 prompt cache 的命中机会）

## 当前问题（阻断）

在 LiteLLM + Gemini（OpenAI-compatible chat）配置下，`omne ask "只回复OK"` 会完成 turn，但没有任何 assistant 文本输出：

- `events.jsonl` 只有 `agent_step` 与 `turn_completed`，没有 `assistant_message`
- `readable_history.jsonl` 只有 user 行

这使得 chat 模式下“用户可读 history + 可复用上下文（用于 cache）”无法成立。

> ✅ 已修复：当 OpenAI-compatible streaming 返回空输出时，OmneAgent 会自动 fallback 到 non-streaming `generate`，从而保证 `omne ask` 有输出、`assistant_message` 落盘、`readable_history.jsonl` 可用。

## 复现方式

前置条件：

- `.omne_data/config_local.toml` 启用并选择 OpenAI-compatible provider（例如 `litellm-talesofai`）
- `.omne_data/.env` 中配置 `LITELLM_API_KEY` 等密钥（不要提交）

复现命令（在 `omne-agent/` 目录执行）：

```bash
cargo run -p omne -- ask "只回复OK"
```

现象：

- 终端只看到 `turn completed`，没有 assistant 输出。
- 对应 thread 的 `.omne_data/threads/<thread_id>/events.jsonl` 无 `assistant_message`。

## 预期行为（对齐目标）

在 OpenAI-compatible chat 模式下也必须满足：

- `omne ask` 能输出最终 assistant 文本（流式或非流式均可）。
- `events.jsonl` 必须落 `assistant_message`（当最终回答非空）。
- `readable_history.jsonl` 必须同时存在 user 与 assistant 文本行，用于用户可读与后续 turn 上下文复用。

（可选增强）若需要“可复用的 raw-history”（用于 compact/审计/更稳定的 cache 键），应为 openai-compatible 增加等价的 raw-history 设计与落盘机制；但这不应阻塞基础可读 history 的正确性。

## 初步根因定位（待验证）

`assistant_message` 的写入点在 `crates/app-server/src/agent/core/run_turn.rs`：

- 仅当 tool loop 的 `last_text` 非空时追加 `assistant_message`。

当前现象说明 tool loop 的 `last_text` 为空。最常见原因是：

- openai-compatible streaming 没有产生 `StreamChunk::TextDelta`，导致最终 `output_text` 为空。

需要通过可控的 debug 方式捕获（已实现，见下文）：

- 实际收到的 SSE `data:` 序列（或 parse 后的 `StreamChunk` 序列）统计
- 实际发出的 chat/completions request body（需脱敏，且不包含 Authorization）

## 根因确认（LiteLLM streaming 空输出）

对同一份请求体（包含我们的 system instructions + 完整 tools list）：

- `stream=true`：LiteLLM 可能只返回一条 `finish_reason=stop` 的 chunk + `[DONE]`，**不包含任何 `delta.content`**。
- `stream=false`：同样请求会返回正常的 `"message.content": "OK"`。

结论：

- 这不是 OmneAgent 的 SSE parse bug，而是 OpenAI-compatible streaming 在该 provider 上的行为缺陷（或限制）。
- 我们仍然需要保留 streaming 支持（用于其它 provider/场景），但必须提供 non-streaming `generate` 路径与 fallback，避免出现“turn completed 但无输出”的 silent failure。

## 计划（分阶段推进）

1. **补齐观测**：在 thread 的 `runtime/` 下落盘 LLM stream debug（默认关闭，环境变量开启），用于定位缺失 `TextDelta` 的原因。
2. **修复输出链路**：
   - 支持 OpenAI-compatible provider 的 non-streaming `generate` 路径（当 `capabilities.streaming=false` 时走 generate）。
   - 当 streaming 返回空输出（无 text/tool deltas）时，自动 fallback 到 non-streaming `generate`，确保 tool loop 生成非空 `last_text`，从而写入 `assistant_message` 与 `readable_history.jsonl`。
   - LiteLLM/Gemini 这类已知会出现空 streaming 的 provider，建议在 provider profile 里显式设置 `capabilities.streaming=false`，避免每次 turn 都多打一枪（stream + generate）。
3. **测试**：
   - 单测：模拟 streaming 只返回 `finish_reason` 且无 `TextDelta`/tool deltas 的情况，断言会 fallback 到 `generate`，并最终写入 `assistant_message`。
   - 端到端：基于 LiteLLM 配置实际跑 `omne ask "只回复OK"`，验证输出与落盘文件。
4. **文档更新**：在 `docs/omne_data.md` / `docs/runtime_layout.md` 中写清 chat 模式与 responses 模式各自的 history 落盘边界与用途。

## 验收标准（DoD）

- `cargo test --workspace` 通过。
- 端到端复现用例通过：
  - `cargo run -p omne -- ask "只回复OK"` 必须输出 `OK`。
  - 对应 thread 的 `events.jsonl` 必须包含 `assistant_message`。
  - 同一 thread 的 `readable_history.jsonl` 必须包含 assistant 行。
- debug 落盘（若实现）必须写入 `runtime/`，禁止写入 `artifacts/`。

## 相关文档

- `docs/omne_data.md`：`.omne_data/` 结构与历史文件约定
- `docs/runtime_layout.md`：runtime 与 artifacts 的目录边界
- `docs/thread_event_model.md`：事件模型与 `assistant_message` 的语义
