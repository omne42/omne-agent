# Prompt Cache：cached_tokens 的观测与验证

## 目标

把 OpenAI-compatible `chat/completions` 的 prompt cache（`cached_tokens`）作为一等可观测指标：

- **能看到**：落盘到 thread `events.jsonl`（`assistant_message.token_usage.cache_input_tokens`）。
- **能复现**：同一 prompt 的二次请求能看到 `cached_tokens > 0`（当上游支持且满足触发条件时）。
- **不掩盖**：如果上游返回 `cached_tokens=0`，Omne 也如实记录为 0（不做“假命中”）。

## 我们做了什么

### 1) ditto-llm：请求与解析

- Streaming 时总是带 `stream_options: { include_usage: true }`，确保能拿到 usage。
- cached token 解析兼容两种常见位置：
  - `usage.cached_tokens`
  - `usage.prompt_tokens_details.cached_tokens`
- 解析结果写入 `ditto_llm::Usage.cache_input_tokens`，供上层落盘与统计。

### 2) omne-app-server：稳定 cache key

为了让 gateway 侧有稳定的“会话分桶”，Omne 对每个 turn 都设置：

- `ProviderOptions.prompt_cache_key = thread_id`
- `GenerateRequest.user = thread_id`

> 注：`cached_tokens` 是否命中是**上游行为**。我们能做的是“稳定请求前缀 + 传对参数 + 把结果透传出来”。

### 3) 诊断落盘（可选）

设置 `OMNE_DEBUG_LLM_STREAM=1` 后，Omne 会在 thread runtime 下落盘 debug：

- `runtime/llm_stream/<turn_id>.jsonl`：stream chunk 统计与 usage（已脱敏）
- `runtime/llm_stream/<turn_id>.request_body.json`：OpenAI-compatible 近似请求体（用于离线复现）

## 如何验证

### A) 直接验证上游是否会返回 cached_tokens

以 LiteLLM 网关为例（非 streaming，且 prompt 足够长时更容易触发）：

1. 对同一 `prompt_cache_key`、同一 `messages` 连续请求两次。
2. 检查第二次响应的 `usage.prompt_tokens_details.cached_tokens` 是否大于 0。

### B) 验证 Omne 是否正确落盘 cache 指标

1. 连续跑两次 `omne ask` 并复用同一 `--thread-id`。
2. 查看 `.omne_data/threads/<thread_id>/events.jsonl`：

```bash
jq -r 'select(.type=="assistant_message") | [.turn_id, (.token_usage.input_tokens//""), (.token_usage.cache_input_tokens//""), (.token_usage.output_tokens//"")] | @tsv' \
  .omne_data/threads/<thread_id>/events.jsonl
```

## 已知限制（重要）

1) **cached_tokens 可能为 0，并不一定是 Omne 的 bug**

- prompt cache 由上游实现，常见存在“最小 prompt tokens 阈值”或“只缓存前 N tokens”的策略。
- 因此，即使二次请求前缀完全相同，`cached_tokens` 也可能仍为 0（上游未触发缓存）。

2) **部分网关在 streaming 下不返回有效 cached_tokens**

我们已确保 streaming 请求包含 `include_usage`，但仍可能出现：

- non-streaming：`cached_tokens > 0`
- streaming：`cached_tokens = 0`

这种情况属于上游 OpenAI-compatible 实现差异，Omne 只能如实记录。

## 验收标准（DoD）

- `ditto-llm`：
  - streaming body 必须包含 `stream_options.include_usage=true`
  - 能从 `usage.cached_tokens` 与 `usage.prompt_tokens_details.cached_tokens` 任一位置读出 cached tokens
- `omne-agent`：
  - 每次 turn 都设置稳定的 `prompt_cache_key` 与 `user`
  - 当上游返回 cached tokens 时，`assistant_message.token_usage.cache_input_tokens` 必须等于上游值

