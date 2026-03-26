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
- 解析结果写入 `ditto_core::contracts::Usage.cache_input_tokens`，供上层落盘与统计。

### 2) omne-app-server：稳定会话前缀（按协议差异）

为了让 gateway/上游更容易命中 cache，Omne 会稳定设置会话信息，但是否发送显式 cache 参数按协议区分：

- OpenAI Responses：
  - `ProviderOptions.prompt_cache_key = thread_id`
  - `GenerateRequest.user = thread_id`
- OpenAI-compatible Chat Completions（默认）：
  - `GenerateRequest.user = thread_id`
  - **不默认发送** `prompt_cache_key`（很多 strict OpenAI-compatible 服务器会因未知字段报 400）
  - 如需强制发送，可显式设置环境变量：`OMNE_OPENAI_COMPAT_SEND_PROMPT_CACHE_KEY=true`

> 注：`cached_tokens` 是否命中是**上游行为**。Omne 负责“稳定请求前缀 + 兼容请求字段 + 如实透传 usage”。

### 3) 诊断落盘（可选）

设置 `OMNE_DEBUG_LLM_STREAM=1` 后，Omne 会在 thread runtime 下落盘 debug：

- `runtime/llm_stream/<turn_id>.jsonl`：stream chunk 统计与 usage（已脱敏）
- `runtime/llm_stream/<turn_id>.request_body.json`：OpenAI-compatible 近似请求体（用于离线复现）

## 如何验证

### A) 直接验证上游是否会返回 cached_tokens

以 LiteLLM 网关为例（非 streaming，且 prompt 足够长时更容易触发）：

1. 对同一会话前缀（同一 `user`、同一 `messages`；可选同一 `prompt_cache_key`）连续请求两次。
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

## Provider 默认建议（DeepSeek / Kimi / MiniMax / Qwen / GLM）

- 推荐默认：`capabilities.prompt_cache = true`
- 推荐默认：OpenAI-compatible 路径 `reasoning = false`（走 Chat Completions）
- 推荐默认：不显式发送 `prompt_cache_key`（保兼容，依赖上游自动缓存命中）
- usage 解析需兼容厂商差异字段（尤其 DeepSeek 的 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`）

实测补充（经由第三方聚合路由时常见）：

- GLM：不同路由前缀可能导致缓存行为差异（例如 `glm-*` 与 `ark/glm-*` 可能不一致）。
- Qwen：部分 OpenAI-compatible 路由只返回 `prompt_tokens_details.text_tokens`，不返回 `cached_tokens`；
  这通常表示“缓存命中指标不可观测”，不等价于模型一定没有缓存。

官方文档参考：

- DeepSeek：<https://api-docs.deepseek.com/news/news0802/>
- Kimi（Moonshot）：<https://platform.moonshot.ai/docs/guide/knowledge-base/context-cache>
- MiniMax：<https://platform.minimax.io/docs/guide/text-generation/context-cache>
- Qwen（阿里云百炼）：<https://help.aliyun.com/zh/model-studio/context-cache>
- GLM（智谱）：<https://docs.bigmodel.cn/cn/guide/promptCaching>

## 验收标准（DoD）

- `ditto-llm`：
  - streaming body 必须包含 `stream_options.include_usage=true`
  - 能从 `usage.cached_tokens` 与 `usage.prompt_tokens_details.cached_tokens` 任一位置读出 cached tokens
- `omne-agent`：
  - 每次 turn 都设置稳定的 `user`
  - OpenAI Responses 路径设置稳定的 `prompt_cache_key`
  - OpenAI-compatible 路径默认不发送 `prompt_cache_key`（除非显式开启 `OMNE_OPENAI_COMPAT_SEND_PROMPT_CACHE_KEY=true`）
  - 当上游返回 cached tokens 时，`assistant_message.token_usage.cache_input_tokens` 必须等于上游值
