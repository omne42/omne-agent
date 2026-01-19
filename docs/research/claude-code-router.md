# Claude Code Router（example/claude-code-router）能力与设计分析

> Snapshot: `example/claude-code-router` @ `c73fe0d`
>
> 结论先行：CCR 的核心价值是把“一个强绑定单 Provider 的客户端（Claude Code）”变成“**可路由、可变换、可插拔**”的多 Provider 网关，并且把“模型选择/配置管理/日志观测”产品化。对 `codex_pm` 来说，它提供了两类可借鉴点：**(1) 路由与多模型角色分工**，**(2) 请求/响应变换管线（Transformers）**。

---

## 1. 它解决什么问题

- Claude Code 客户端默认面向 Anthropic API（`/v1/messages`），但用户希望：
  - 选择不同供应商（OpenRouter/DeepSeek/Ollama/Gemini/…）。
  - 按场景路由（便宜模型跑 background、强模型跑 think、长上下文模型跑 longContext）。
  - 统一处理 tool use / reasoning 等字段差异。
- CCR 通过本地服务把客户端请求接入：**Anthropic-format in → CCR → Provider-format out**。

---

## 2. “所有能力”盘点（按能力域）

### 2.1 Server：统一入口 + API 能力

从 `docs/docs/server/intro.md` 与 API 文档可归纳：

- 兼容 Anthropic Messages API：
  - `POST /v1/messages`（支持 SSE streaming）。
  - `POST /v1/messages/count_tokens`（token 计数）。
- 管理 API：
  - `GET/POST /api/config`：读写配置，自动备份旧配置（保留最近 3 份）。
  - `GET /api/transformers`：列出已加载 transformers。
  - `GET/DELETE /api/logs`、`GET /api/logs/files`：日志查看/清理。
  - `POST /api/restart`：重启服务。
  - `GET /ui`：Web UI。
- 安全与部署：
  - API Key 鉴权（`x-api-key`）。
  - 如果未设置 APIKEY，host 强制 `127.0.0.1`（防止裸奔暴露）。
  - Docker 部署（文档中给了 team shared service 场景）。

### 2.2 CLI：把服务运维/配置产品化

从 `docs/docs/cli/intro.md` + 根 README 可归纳：

- `ccr start/stop/restart/status`：服务控制。
- `ccr code`：启动 Claude Code 并自动把请求路由到 CCR。
- `ccr ui`：打开 Web UI 管理配置。
- `ccr model`：交互式模型管理（查看/切换默认、background、think、longContext 等）。
- `ccr preset`：预设导入/导出（导出时自动把敏感字段替换为占位符）。
- `ccr activate`：输出环境变量（`ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN` 等），让用户可以直接运行 `claude` 命令而无需 `ccr code` 包装。
- `NON_INTERACTIVE_MODE`：CI/Actions 场景下避免 stdin 卡住，并自动设置 `CI=true`、`FORCE_COLOR=0` 等。

### 2.3 路由能力：按场景/阈值选模型

从根 README 与 routing 文档可归纳：

- Router 配置支持：
  - `default/background/think/longContext/webSearch/image` 等场景路由。
  - `longContextThreshold`：超过阈值时自动切 longContext 模型。
  - `CUSTOM_ROUTER_PATH`：自定义 JS 路由函数（接收 req + config，返回 `"provider,model"` 或 null）。
- **Subagent Routing**：通过在子 agent prompt 头部插入 `<CCR-SUBAGENT-MODEL>provider,model</CCR-SUBAGENT-MODEL>` 强制该子 agent 使用指定模型。

补充（project-level routing）：

- CCR 支持“按项目覆盖路由规则”：`~/.claude/projects/<project-id>/claude-code-router.json`（见 `docs/docs/cli/config/project-level.md`）。
- 路由优先级（高→低）：
  1. `CUSTOM_ROUTER_PATH`
  2. project-level config
  3. global config（`~/.claude-code-router/config.json`）
  4. built-in rules

### 2.4 Transformers：请求/响应变换管线（核心巧思）

CCR 架构明确写到它基于 `@musistudio/llms`（统一 LLM API transformation 库）：

- 定义统一格式：`UnifiedChatRequest/UnifiedChatResponse`，把 provider-specific 差异收敛到 transformer 层。
- Transformer 接口提供 4 个主要钩子：
  - `transformRequestOut`：incoming（Anthropic）→ Unified
  - `transformRequestIn`：Unified → provider request（可附加参数/headers）
  - `transformResponseIn`：provider response → Unified（可做 streaming 兼容/字段归一）
  - `transformResponseOut`：Unified → outgoing（Anthropic）
- 内置 transformers 覆盖：
  - anthropic/openai/gemini/deepseek/openrouter/groq/…
  - `maxtoken`（限制 max_tokens）
  - `tooluse`（tool call 适配）
  - `reasoning`（reasoning 字段适配）
  - `enhancetool`（对 tool call 参数做容错；代价是 tool call 信息不再 streaming）
  - `vertex-gemini`、以及一些“非官方 CLI 模拟”适配（qwen-cli/rovo-cli 等）
- 支持加载自定义 transformer 插件（配置 `transformers: [{ path, options }]`）。

补充（图像与工具兼容的产品化处理）：

- README 提到 image(beta) 路由与“内置 image agent”；当目标模型不支持 tool calling 时，可用 `forceUseImageAgent` 强制走 image agent（本质是“能力缺口补丁层”，很有工程价值）。

---

## 3. 值得学习的“产品化细节”

### 3.1 配置的可迁移性：preset + 敏感信息脱敏

预设导出会把 apiKey 等敏感信息变成 `{{field}}`，安装时再收集输入。这是“可分享/可团队化”的关键。

### 3.2 激活机制：最小侵入集成现有客户端

`ccr activate` 通过环境变量把“代理能力”注入到既有 CLI（Claude Code / Agent SDK app），用户体验好、兼容面广。

### 3.3 双日志系统

README 提到 server-level logs 与 app-level logs 分离：

- server：HTTP 请求、provider API 调用、server 事件（pino），落在 `~/.claude-code-router/logs/ccr-*.log`
- app：路由决策与业务逻辑日志

这能避免“只看 access log 看不懂路由决策”的问题。

---

## 4. 对 `codex_pm` 的直接启示（可落地的借鉴点）

### 4.1 多角色多模型：Architect/Coder/Reviewer/Merger 的模型路由

我们在 `codex_pm` 有明确的角色体系。CCR 的“场景路由”可直接映射：

- `think` → Architect / Merger（强推理）
- `background` → 辅助检索/整理/格式化建议（便宜模型）
- `longContext` → 大 repo 的全局 review/合并冲突分析

即使第一阶段我们只支持 OpenAI Responses API，仍然可以在**同一 Provider 内做模型路由**，把成本与效果分层。

### 4.2 Transformers 作为“未来扩展槽位”

本期我们只做 Responses，但未来扩展到多 Provider 时，CCR 证明了：

- 不要把 provider 差异散落到业务逻辑里；
- 要么像 CCR 一样在网关层做变换；
- 要么在 `codex_pm` 内做 trait + adapter（Rust 版 transformer 管线）。

### 4.3 管理面（Observability & Operability）

我们要做并发多任务流水线，必然需要：

- request/task 的日志归档与检索；
- “当前配置是什么”的可视化；
- session/task/pr 的状态面板；

CCR 的 config/logs API + Web UI 是一个成熟参考。

---

## 5. 与我们“基于 codex 魔改”的关系

- CCR 的定位是“把 Claude Code 接到别的模型”，而我们要“把 Codex 变成并发 PR 工厂”。
- 两者共同点在于：都需要把 agent 能力产品化成“可配置、可观测、可扩展”的系统。
- **因此我们不需要复制 CCR 的全套网关**，但应该学习其：
  - 路由抽象（role→model 的策略层）
  - 可插拔变换管线的边界（未来多 provider）
  - 配置/预设/日志 API 的产品化方式
