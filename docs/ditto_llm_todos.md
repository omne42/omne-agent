# Ditto-LLM TODOs（从“能用”到“满足需求”）

本文用于记录 `ditto-llm` 距离“完全满足我们需求”仍缺的工作项，并按优先级推进。

## 需求口径（我们到底要什么）

1. **统一 SDK 语义层**：`LanguageModel` / `EmbeddingModel` + 统一 types（messages/content/tools/stream chunks/usage/warnings）。
2. **多 Provider**：OpenAI（Responses）、Anthropic（Messages）、Google（GenAI）已具备；还需要 **OpenAI-compatible（Chat Completions）** 以覆盖 LiteLLM/DeepSeek/Qwen 等。
3. **流式 + tools + embeddings**：跨 provider 行为尽量一致；差异必须以 `Warning` 显式暴露，而不是静默丢字段。
4. **可配置/可复用**：不再出现“配置层一套、SDK 一套”的双轨 API；必须能从 `ProviderConfig/ProviderAuth` 构造可用的 model client。
5. **多模态输入**：至少支持 **图片** 与 **PDF 文件**（base64/url），并在不支持的 provider 上以 `Warning` 显式提示。

## Done（验收标准）

- `cd ../ditto-llm && cargo fmt --check`
- `cd ../ditto-llm && cargo test --all-features`
- `cd ../ditto-llm && cargo clippy --all-targets --all-features -- -D warnings`
- 至少一个 OpenAI-compatible provider 可通过环境变量跑通示例（推荐 LiteLLM）：
  - `OPENAI_COMPAT_BASE_URL`
  - `OPENAI_COMPAT_API_KEY`（或空，取决于 provider）
  - `OPENAI_COMPAT_MODEL`

## Backlog（按优先级）

### P0（必须做）

- [x] **图片 + PDF 文件上传**：`ContentPart::Image` / `ContentPart::File`
  - DoD:
    - types：新增 `ContentPart::File` + `FileSource`
    - OpenAI（Responses）：映射 `input_image` / `input_file`（PDF: url/base64/file_id）
    - OpenAI-compatible（Chat）：映射 `image_url` / `file`（PDF: base64/file_id；URL 明确 `Warning::Unsupported`）
    - Anthropic（Messages）：映射 `image` / `document`（PDF：`anthropic-beta: pdfs-2024-09-25`）
    - Google（GenAI）：映射 `inlineData`/`fileData`（按 `media_type`）
    - tests + examples：新增 `examples/multimodal.rs`

- [x] **OpenAI-compatible（Chat Completions）provider**：补齐 `POST /chat/completions` 的 `generate/stream`（含 tools）
  - DoD:
    - 新增 provider：支持 messages 映射（system/user/assistant/tool）、`tools/tool_choice`、`finish_reason/usage` 映射
    - streaming：SSE `data: {...}` 解析 `delta.content` 与 `delta.tool_calls`
    - 单测：覆盖 request 映射与 stream event 解析（无须真实 API key）
    - examples：给出 LiteLLM/任意 OpenAI-compatible 的最小可跑示例

### P1（强烈建议）

- [x] **配置层 ↔ SDK 层打通**：用 `ProviderConfig/ProviderAuth/Env` 构造具体 provider client
  - DoD:
    - 提供 `OpenAI::from_config` / `Anthropic::from_config` / `Google::from_config` / `OpenAICompatible::from_config`（或等价 API）
    - 统一处理 `api_key_env` / `auth_command` / `base_url` / `default_model`

- [x] **`provider_options` 变成“受控扩展点”**（别把它变成无类型 JSON 垃圾桶）
  - DoD:
    - ✅ 已落地：`ProviderOptions`（`reasoning_effort`、`response_format(json_schema)`）
    - ✅ OpenAI（Responses）：`reasoning.effort` / `response_format` 已映射
    - ✅ Anthropic/Google：对不支持的 options 明确发出 `Warning::Unsupported`

### P2（质量/一致性）

- [x] **统一 stream/generate 的语义一致性**
  - DoD:
    - `warnings` 在流式与非流式路径一致（不丢）
      - ✅ 已实现：stream 会先发出 `StreamChunk::Warnings`
    - `finish_reason` 的映射规则统一且可测试

- [x] **Tool calling streaming 细节对齐**
  - DoD:
    - 增量 arguments（delta）与多 tool_calls 的可靠拼接
    - 对不支持的 provider 必须给 `Warning::Unsupported`

### P3（范围扩展）

- [x] **JSON Schema → OpenAPI schema 转换补齐**（工具 schema 的关键字子集）
  - DoD:
    - 明确支持范围（文档 + tests）
    - 覆盖常见关键字（例如 number/string 约束、object 约束等）

### P4（真实可用性）

- [x] **集成测试（可选 feature）**
  - DoD:
    - `--features integration` 下可用真实 API keys 跑最小回归（默认不在 CI 强制）

### P5（OmneAgent 接入）

- [x] **OmneAgent 主流程接入 ditto-llm（替代部分 `crates/openai` 直连）**
  - DoD:
    - 不改变现有事件/审计语义（tool events / approvals / JSONL 落盘）
    - 迁移路径明确：先 provider/model 选择 → 再调用层替换

- [x] **OmneAgent：`@pdf` 本地附件可选上传为 `file_id`（避免巨量 base64）**
  - DoD:
    - 环境变量门控：`OMNE_AGENT_PDF_FILE_ID_UPLOAD_MIN_BYTES>0` 且达到阈值才上传
    - 上传失败不致命：回退到 base64（并记录 warning）
