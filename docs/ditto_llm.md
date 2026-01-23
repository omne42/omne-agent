# Ditto-LLM（独立仓库）- 统一 LLM SDK 方案草案

## 概述

**Ditto-LLM** 是一个轻量级、嵌入式的 Rust 库，用于统一调用各家 LLM 供应商的 API。就像宝可梦中的百变怪 (Ditto) 可以变身成任何宝可梦一样，Ditto-LLM 可以"变身"成任何 LLM 供应商的接口。设计理念借鉴 Vercel AI SDK 的接口抽象模式，专注于接口逻辑转换，不包含日志审计等企业功能。

## 在 CodePM 仓库中的定位（v0.2.x 现状）

> 本仓库依赖的 `ditto-llm`（本地 checkout：`./ditto-llm`）目前包含两层能力：
>
> 1. CodePM v0.2.x 当前用到的“路由/配置层”：provider profile 配置 + OpenAI-compatible `/models` 发现 + `thinking`(reasoning.effort) 配置。
> 2. “统一 LLM SDK 层”：`LanguageModel` / `EmbeddingModel` traits + 多 provider 适配（OpenAI/Anthropic/Google，含 streaming/tools/embeddings）+ examples/tests。
>
> CodePM 主程序已在 `pm-app-server` agent loop 中接入 `ditto-llm::LanguageModel`（默认 OpenAI Responses；可通过 provider capabilities 切换到 OpenAI-compatible Chat Completions）。`crates/openai` 目前仍保留用于 legacy types / SSE 解析等。

已实现（`ditto-llm` crate）：

- 路由/配置层：`ProviderConfig` / `ProviderAuth` / `.env` 解析 / `GET /models` 发现 / model-level `thinking`
- 统一 SDK 层：统一 types + `LanguageModel`/`EmbeddingModel` + OpenAI/Anthropic/Google providers（含 streaming/tools/embeddings）
- 示例与测试：`examples/` + unit tests（以转换/解析为主）

相关使用点：

- `docs/v0.2.0_parity.md`（F. Provider / Model 路由）
- `crates/app-server/src/project_config.rs`
- `crates/app-server/src/main/thread_manage/models.rs`

---

## AI-SDK 如何处理不同端点

通过分析 Vercel AI SDK 源码，我发现其核心设计模式：

### 端点抽象策略

| 端点类型            | OpenAI API  | Anthropic API       | ai-sdk 处理方式                                        |
| ------------------- | ----------- | ------------------- | ------------------------------------------------------ |
| `/chat/completions` | ✅ 原生支持 | ❌ 使用 `/messages` | 每个 provider adapter 独立实现 `doGenerate`/`doStream` |
| `/completions`      | ✅ 原生支持 | ❌ 不支持           | 已废弃，不作为统一接口                                 |
| `/responses`        | ✅ 原生支持 | ❌ 不存在           | OpenAI 新 API，尚未广泛支持                            |
| `/embeddings`       | ✅ 原生支持 | ❌ 不支持           | 独立的 `EmbeddingModel` trait                          |

### 核心设计原则

```
┌─────────────────────────────────────────────────────────────────┐
│                    统一接口层 (ditto-llm)                         │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  doGenerate(options) -> GenerateResult                    │  │
│  │  doStream(options)   -> StreamResult                      │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                                ▲
                                │ 实现
        ┌───────────────────────┼───────────────────────┐
        │                       │                       │
┌───────┴───────┐       ┌───────┴───────┐       ┌───────┴───────┐
│  OpenAI       │       │  Anthropic    │       │  Google       │
│  Adapter      │       │  Adapter      │       │  Adapter      │
├───────────────┤       ├───────────────┤       ├───────────────┤
│ /chat/        │       │ /messages     │       │ /models/.../  │
│ completions   │       │               │       │ generateContent│
└───────────────┘       └───────────────┘       └───────────────┘
```

**关键洞察：AI-SDK 不尝试统一底层 HTTP 端点，而是统一「语义接口」**

---

## Ditto-LLM 技术架构（未来设想）

### 1. 核心 Trait 定义

```rust
/// 语言模型核心 trait (类比 ai-sdk 的 LanguageModelV3)
#[async_trait]
pub trait LanguageModel: Send + Sync {
    /// 供应商标识
    fn provider(&self) -> &str;

    /// 模型标识
    fn model_id(&self) -> &str;

    /// 非流式生成
    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse, DittoError>;

    /// 流式生成
    async fn stream(&self, request: GenerateRequest) -> Result<impl Stream<Item = Result<StreamChunk, DittoError>>, DittoError>;
}

/// Embedding 模型 trait (独立于语言模型)
#[async_trait]
pub trait EmbeddingModel: Send + Sync {
    fn provider(&self) -> &str;
    fn model_id(&self) -> &str;

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, DittoError>;
    async fn embed_single(&self, text: String) -> Result<Vec<f32>, DittoError>;
}
```

### 2. 统一数据结构

```rust
/// 统一消息格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },

    #[serde(rename = "image")]
    Image {
        #[serde(flatten)]
        source: ImageSource
    },

    #[serde(rename = "tool_call")]
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_call_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },

    #[serde(rename = "reasoning")]
    Reasoning { text: String },
}

/// 生成请求
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub messages: Vec<Message>,
    pub model: Option<String>,  // 可覆盖默认模型

    // 通用参数
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,

    // Tool calling
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,

    // 供应商特定选项
    pub provider_options: Option<serde_json::Value>,
}

/// 生成响应
#[derive(Debug, Clone)]
pub struct GenerateResponse {
    pub content: Vec<ContentPart>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
    pub warnings: Vec<Warning>,
    pub provider_metadata: Option<serde_json::Value>,
}

/// 流式响应块
#[derive(Debug, Clone)]
pub enum StreamChunk {
    TextDelta { text: String },
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, arguments_delta: String },
    ReasoningDelta { text: String },
    FinishReason(FinishReason),
    Usage(Usage),
}
```

### 3. 供应商适配器设计

```rust
/// OpenAI 适配器
pub struct OpenAI {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    default_model: String,
}

impl OpenAI {
    pub fn new(api_key: impl Into<String>) -> Self { /* ... */ }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self { /* ... */ }

    pub fn with_model(mut self, model: impl Into<String>) -> Self { /* ... */ }

    /// 转换请求到 OpenAI 格式
    fn to_openai_request(&self, req: &GenerateRequest) -> OpenAIRequest { /* ... */ }

    /// 转换响应到统一格式
    fn from_openai_response(&self, resp: OpenAIResponse) -> GenerateResponse { /* ... */ }
}

#[async_trait]
impl LanguageModel for OpenAI {
    fn provider(&self) -> &str { "openai" }
    fn model_id(&self) -> &str { &self.default_model }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse, DittoError> {
        let openai_req = self.to_openai_request(&request);
        let resp = self.client
            .post(format!("{}/responses", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&openai_req)
            .send()
            .await?;

        let openai_resp: OpenAIResponse = resp.json().await?;
        Ok(self.from_openai_response(openai_resp))
    }

    async fn stream(&self, request: GenerateRequest) -> Result<impl Stream<...>, DittoError> {
        // SSE 流处理...
    }
}
```

### 4. 兼容性警告系统

借鉴 ai-sdk 的 warnings 机制，处理供应商差异：

```rust
#[derive(Debug, Clone)]
pub enum Warning {
    /// 不支持的特性
    Unsupported {
        feature: String,
        details: Option<String>,
    },

    /// 值被钳制
    Clamped {
        parameter: String,
        original: f32,
        clamped_to: f32,
    },

    /// 兼容性提示
    Compatibility {
        feature: String,
        details: String,
    },
}
```

---

## 项目结构（未来设想）

```
ditto-llm/
├── Cargo.toml
├── src/
│   ├── lib.rs                    # 公开 API
│   ├── model.rs                  # LanguageModel trait
│   ├── embedding.rs              # EmbeddingModel trait
│   ├── types/
│   │   ├── mod.rs
│   │   ├── message.rs            # Message, ContentPart, Role
│   │   ├── request.rs            # GenerateRequest
│   │   ├── response.rs           # GenerateResponse, StreamChunk
│   │   ├── tool.rs               # Tool, ToolChoice
│   │   └── error.rs              # DittoError, Warning
│   ├── providers/
│   │   ├── mod.rs
│   │   ├── openai/
│   │   │   ├── mod.rs
│   │   │   ├── client.rs         # OpenAI struct
│   │   │   ├── convert.rs        # 请求/响应转换
│   │   │   └── stream.rs         # SSE 解析
│   │   ├── anthropic/
│   │   │   ├── mod.rs
│   │   │   ├── client.rs
│   │   │   ├── convert.rs
│   │   │   └── stream.rs
│   │   ├── google/
│   │   │   └── ...
│   │   └── openai_compatible/    # DeepSeek, Qwen, etc.
│   │       └── ...
│   └── utils/
│       ├── mod.rs
│       └── sse.rs                # 通用 SSE 解析
└── examples/
    ├── basic.rs
    ├── streaming.rs
    ├── tool_calling.rs
    └── embeddings.rs
```

---

## Feature Flags（未来设想）

```toml
[features]
default = ["openai", "anthropic"]

# 供应商
openai = []
anthropic = []
google = []
openai-compatible = []  # DeepSeek, Qwen, Mistral, etc.

# 可选功能
streaming = []          # 流式支持 (默认开启)
tools = []              # Tool calling 支持
embeddings = []         # Embedding 模型支持
```

---

## 使用示例（未来 API 草案）

```rust
use ditto_llm::{OpenAI, Anthropic, LanguageModel, Message, Role};

#[tokio::main]
async fn main() -> Result<(), ditto_llm::Error> {
    // OpenAI - Ditto 变身!
    let openai = OpenAI::new(std::env::var("OPENAI_API_KEY")?)
        .with_model("gpt-4o");

    // Anthropic
    let anthropic = Anthropic::new(std::env::var("ANTHROPIC_API_KEY")?)
        .with_model("claude-sonnet-4-20250514");

    // 统一的调用方式
    let messages = vec![
        Message::system("You are a helpful assistant."),
        Message::user("What is 2+2?"),
    ];

    // 非流式
    let response = openai.generate(messages.clone().into()).await?;
    println!("OpenAI: {}", response.text());

    // 流式
    let mut stream = anthropic.stream(messages.into()).await?;
    print!("Anthropic: ");
    while let Some(chunk) = stream.next().await {
        if let StreamChunk::TextDelta { text } = chunk? {
            print!("{}", text);
        }
    }

    Ok(())
}
```

---

## 开发计划（未来设想）

| 阶段     | 内容                          | 预估时间    |
| -------- | ----------------------------- | ----------- |
| Phase 1  | 核心 types + OpenAI 适配器    | 2 天        |
| Phase 2  | Anthropic 适配器 + 流式       | 2 天        |
| Phase 3  | Google Gemini 适配器          | 1 天        |
| Phase 4  | OpenAI 兼容层 (DeepSeek/Qwen) | 1 天        |
| Phase 5  | Tool calling 支持             | 1-2 天      |
| Phase 6  | Embedding 模型                | 1 天        |
| **总计** |                               | **8-10 天** |

---

## 与 ai-sdk 的关键差异

| 方面      | AI-SDK (TypeScript) | Ditto-LLM (Rust)    |
| --------- | ------------------- | ------------------- |
| 语言      | TypeScript          | Rust                |
| 运行时    | Node.js             | Native              |
| 异步      | Promise             | async/await + tokio |
| 流式      | AsyncIterable       | futures::Stream     |
| 类型安全  | 编译时 + 运行时     | 编译时强保证        |
| JSON 处理 | 动态                | serde 强类型        |

---

## 验证计划（未来设想）

### 自动化测试

1. **单元测试**

   ```bash
   cargo test
   ```

2. **集成测试** (需要 API keys)
   ```bash
   OPENAI_API_KEY=xxx ANTHROPIC_API_KEY=xxx cargo test --features integration
   ```

### 手动验证

1. 运行 examples 目录下的示例代码
2. 验证流式输出正确性
3. 验证 tool calling 往返正确性

---

## 待确认事项

> [!NOTE]
> ✅ 项目名称已确定：**ditto-llm**
>
> CodePM v0.2.x 口径：
>
> - 仅要求 OpenAI Responses API（其它 provider 先不做）
> - ✅ 支持图片/文件上传：images + PDFs（url/base64；provider 不支持的能力会以 `Warning` 显式暴露）
>
> 已确认/已实现：
>
> 1. ✅ **初始供应商支持**：OpenAI + Anthropic + Google
> 2. ⏸️ **图片/文件上传支持**：暂不支持（先以纯文本 + tool calling 为主）
