# Model / Provider / Router（模型选择与可解释性）（v0.2.0 现状 + TODO）

> 目标：把“用哪个模型、为什么”变成可解释、可回放、可审计的事实；并为后续的 role/keyword/subagent 路由预留最小扩展点。

---

## 0) 范围与非目标

范围（v0.2.0）：

- 当前已实现的 model/base_url 配置来源与覆盖顺序。
- `thread/config/explain` 的输出口径（回答“为什么生效的是这个值”）。

非目标（v0.2.0）：

- 多 provider 抽象仍是 TODO（capability flags 已有最小实现；见 `docs/v0.2.0_parity.md`）。
- role/keyword/subagent 的自动路由（MVP 已实现；见下文 Router）。
- 自动升降级（cheap→strong）已实现最小切片：模型 fallback（见 §5）。
- tool 调用“轻模型”通道已实现（`OMNE_AGENT_TOOL_MODEL`，见 §6）。
- 429/5xx/timeout 的 provider fallback 已实现（见 §4）。

---

## 1) v0.2.0 现状：模型选择（已实现）

### 1.1 生效顺序（写死）

`provider` 的生效顺序（用于派生 base_url/auth/default_model）：

1. project config（当 `project_config.enabled=true`）：`openai.provider`
2. env：`OMNE_OPENAI_PROVIDER`
3. default：`openai-codex-apikey`

当前 turn 的 `model` 生效顺序为：

1. thread config（来自 `ThreadConfigUpdated` 事件）
2. project config（当 `project_config.enabled=true`）：`openai.model`
3. env：`OMNE_OPENAI_MODEL`
4. provider default（当存在）：`openai.providers.<provider>.default_model`
5. default：`model="gpt-4.1"`

当前 turn 的 `openai_base_url` 生效顺序为：

1. thread config（来自 `ThreadConfigUpdated` 事件）
2. project overrides（当 `project_config.enabled=true`）：`.omne_data/.env` 的 `OMNE_OPENAI_BASE_URL`（可选）
3. env：`OMNE_OPENAI_BASE_URL`
4. provider base_url：`openai.providers.<provider>.base_url`（或 builtin default）
5. default：`openai_base_url="https://api.openai.com/v1"`

> 注意：实现中 default 在多个位置硬编码；未来如果改默认值，必须同步，避免 explain 与实际漂移。

Provider runtime 连接缓存的当前口径：

- cache 仍是 server 级复用，用来保持 HTTP 连接粘性。
- 但 cache key 必须同时编码 provider config 与 thread 级 `.omne_data/.env` 输入；不同 thread 的认证/headers/dotenv 不能复用同一个 runtime。
- 因此当 thread 的 provider 凭证或相关 dotenv 变化时，会命中新 key，而不是继续复用旧 client。

### 1.2 落盘（可回放）

- 每次 assistant 输出会落盘 `AssistantMessage { model: Option<String>, response_id, token_usage? }`。
- 每个 turn 会落盘一次 `ModelRouted { selected_model, rule_source, reason?, rule_id? }`，用于回答“这次用哪个模型、为什么”。（实现对照：`omne_protocol::ThreadEventKind::ModelRouted`）

---

## 2) `thread/config/explain`（已实现）

### 2.1 作用

`thread/config/explain` 用于解释一个 thread 的有效配置来自哪里，返回：

- `effective`：最终生效值（approval/sandbox/mode/openai_provider/model/base_url 等）
- `layers`：分层来源（`default` → `env` → `thread`）
- `mode_catalog` + `effective_mode_def`：当前 mode 的定义与来源（builtin/env/project）

`layers` 的重要边界（避免误读）：

- `default` layer 是一个完整快照（包含 approval/sandbox/mode/openai_provider/model/base_url 等）。
- `env` layer 目前只覆盖 `openai_provider/model/openai_base_url`（不是完整快照）。
- `thread` layer 不是单条记录：每次出现 `ThreadConfigUpdated` 事件都会追加一条 layer（包含 `seq/timestamp` + 当时的有效快照）。

### 2.2 CLI（可复制）

```bash
omne thread config-explain <thread_id> --json
```

列出 provider 可用模型（`GET /models` + provider allowlist）：

```bash
omne thread models <thread_id> --json
```

实现对照：

- `crates/app-server/src/main/thread_manage/config.rs`（`handle_thread_config_explain`）

---

## 3) Router（role/keyword/subagent）（MVP 已实现）

> 目标态定义在 `docs/v0.2.0_parity.md`。这里给当前实现口径与后续扩展点，避免漂移。

### 3.1 路由优先级链（已定，写死）

`subagent 强制 > project override > keyword rule > role default > global default`

### 3.2 配置文件位置（建议写死）

约定：Router 属于项目可提交 spec（`./.omne_data/spec/`）；运行时数据位于 `.omne_data/{tmp,threads,...}`，不参与发现/解析。

发现顺序（高 → 低）建议写死：

1. env：`OMNE_ROUTER_FILE`（绝对或相对路径；相对路径按 thread cwd 解析）
2. `./.omne_data/spec/router.yaml`（推荐）
3. `./.omne_data/spec/router.json`（可选；也支持 `router.yml`）

若 env 指向的文件不存在，或 Router 文件解析失败：直接报错（fail-closed），避免“以为生效但其实没生效”。

### 3.3 最小配置结构（草案）

```yaml
version: 1
role_defaults:
  architect: gpt-4.1
  reviewer: gpt-4.1
  coder: gpt-4.1-mini
keyword_rules:
  - id: long-context
    keywords: ["vector database", "embedding", "RAG"]
    model: gpt-4.1
    reason: "needs long-context reasoning"
project_override: null
```

匹配语义建议：

- `keywords` 做大小写不敏感的子串匹配即可（先别上 regex）。
- 命中多条规则时，以配置顺序第一条为准（写死，避免“规则竞赛”）。
- v1 建议对 schema 做严格校验（未知字段直接报错，避免 typo 静默失败；需要扩展就 bump `version`）。

字段语义（v1 建议写死）：

- `version`：整数，当前固定 `1`。
- `project_override`：
  - `null`：不启用
  - `{ model: string, reason?: string }`：强制该项目使用指定 model（`rule_source="project_override"`）
- `role_defaults`：map（key=role/mode 名，例如 `architect/reviewer/coder/builder`；value=model string）
- `keyword_rules`：数组（可选）。每项至少包含：
  - `id: string`（稳定标识，用于 `rule_id`）
  - `keywords: [string]`（任意关键词命中即匹配）
  - `model: string`
  - `reason?: string`

匹配算法（v1 建议写死）：

- `global_default`：使用 `thread/config/explain.effective.model` 作为基础 model（即配置层的最终生效值：thread/env/default 叠加后的结果）。
- 计算顺序固定为（first-match）：
  1. `subagent 强制`（若存在显式的 subagent model override；例如 forked thread 通过 `thread/configure --model ...` 设定）
  2. `project_override`
  3. `keyword_rules`（按配置顺序 first-match）
  4. `role_defaults[role]`（若存在）
  5. `global_default`
- 关键词匹配范围（v1 建议最小）：只对本次 `turn/start.input` 做大小写不敏感子串匹配（避免把大日志/产物纳入匹配导致误触发）。

### 3.4 “可解释性落盘”（关键，别留到最后）

每个 turn 必须记录一次路由决策（已实现：`ModelRouted` 事件）：

- `selected_model`
- `rule_source`：`subagent|project_override|keyword_rule|role_default|global_default`
- `reason`（可选但强烈建议）
- `rule_id`（可选；便于审计）

落盘位置（已实现）：

- 新事件：`ModelRouted { turn_id, selected_model, rule_source, reason?, rule_id? }`

`rule_source="global_default"` 的口径（建议写死）：

- 表示 Router 没有覆盖；`selected_model` 等于当前 `thread/config/explain.effective.model`（即配置层的最终生效 model）。
- `reason` 可选写清楚“来自 thread/env/default 哪一层”（但不要依赖它做机器判定）。

验收（未来实现时）：

- `omne thread events <thread_id> --json` 能看到每个 turn 的 `selected_model + reason + rule_source`。

---

### 3.5 已实现：上下文阈值（long-context）

> 目标：当上下文接近模型上限时，避免“隐式截断/隐式退化”，而是做一次**可解释的路由或压缩**。
>
实现口径（复用 `keyword_rules` 槽位，不新增优先级链）：

- `keyword_rules` 每条规则可选增加 `min_context_tokens`（整数，> 0）。
- `keywords` 现在可省略；当同时配置 `keywords + min_context_tokens` 时，必须同时满足。
- 匹配顺序仍按列表顺序 first-match（写死，避免“规则竞赛”）。
- 命中后：
  - `selected_model = rule.model`
  - `rule_source = "keyword_rule"`（保持既有枚举）
  - `rule_id = rule.id`
  - `reason` 会包含 `context_tokens_estimate` 与阈值（便于审计；落盘在 `ModelRouted.reason`）

配置示例：

```yaml
version: 1
role_defaults:
  architect: gpt-4.1
  reviewer: gpt-4.1
  coder: gpt-4.1-mini
keyword_rules:
  - id: long-context-threshold
    min_context_tokens: 120000
    model: gpt-4.1
    reason: "context too large; prefer long-context model"
```

关于 `context_tokens_estimate` 的估算（写清楚：这是近似值，不是 tokenizer 的精确结果）：

- 现实现状使用 **字符数近似**（`(instructions + input_items).chars()/4`，向上取整）。
- 这是保守的工程折中：目的是避免“隐式截断/隐式退化”，不是做精确计费。
- 若需要更精确的 tokenizer，应作为后续增强点（避免 v0.2.0 过度设计）。

压缩（auto compact/summary）的关系：

- “切 long-context” 与 “compact/summary” 可以先只做一个。
- 如果两者都实现，建议优先级写死为：
  1. 有 long-context model 且命中阈值 → 路由到 long-context
  2. 否则 → 触发 compact/summary（规格草案见 `docs/budgets.md`）
- 当前实现里，Router 与 compact **都使用同一个 `context_tokens_estimate` 口径**（当前 prompt-load 的近似值）。
- `total_tokens_used` 只用于 thread 生命周期 budget / warning / exceeded；不再驱动 compact 决策。

---

## 4) 已实现：失败重试与 provider fallback（v0.2.0）

### 4.1 触发条件（实现口径）

- 只覆盖 **LLM streaming 请求**（`LanguageModel::stream`）。
- 若请求在产生任何输出（`output_text` delta / tool_call chunk）之前失败，且错误属于：
  - HTTP `429`（rate limit）
  - HTTP `5xx`（provider / upstream error）
  - 请求超时（timeout）
  则允许重试。
- 一旦已产生任何输出：**不做静默重试**（避免重复输出），直接失败。

### 4.2 配置与参数

fallback provider 列表来源（高 → 低）：

1. env：`OMNE_OPENAI_FALLBACK_PROVIDERS="p1,p2,p3"`（逗号分隔）
2. project config（需 `project_config.enabled=true`）：`openai.fallback_providers = ["p1", "p2"]`

重试参数（env）：

- `OMNE_AGENT_LLM_MAX_ATTEMPTS`（默认 `3`）
- `OMNE_AGENT_LLM_RETRY_BASE_DELAY_MS`（默认 `200`）
- `OMNE_AGENT_LLM_RETRY_MAX_DELAY_MS`（默认 `2000`）

### 4.3 落盘与可解释性

- 每个 turn 的 `ModelRouted.reason` 会附加 `provider=<name>`。
- 当发生 provider fallback 时，会追加一条 `ModelRouted` 事件，`reason` 形如：
  - `provider_fallback: from=<prev> to=<next>; cause=<error summary>`

---

## 5) 已实现：cheap→strong 模型 fallback（v0.2.0）

### 5.1 触发条件（实现口径）

- 只覆盖 **LLM streaming 请求**（`LanguageModel::stream`）。
- 当请求在产生任何输出（`output_text` delta / tool_call chunk）之前失败，且错误属于部分非重试类 API 错误（目前按 HTTP status 判定：`400/404/413/422`），则允许切换到下一个 fallback model 并重试本次 step。
- 一旦已产生任何输出：**不做静默切换**（避免重复/混杂输出），直接失败。
- 一旦切换到 fallback model：后续 step 默认沿用该 model（不做自动降级）。

### 5.2 配置

fallback model 列表来源：

- env：`OMNE_AGENT_FALLBACK_MODELS="m1,m2,m3"`（逗号分隔；按顺序尝试）

约束：

- fallback models 仍受 provider 的 `model_whitelist` 约束（不在白名单直接跳过/拒绝）。

### 5.3 落盘与可解释性

- 当发生 model fallback 时，会追加一条 `ModelRouted` 事件，`reason` 形如：
  - `model_fallback: from=<prev> to=<next>; cause=<error summary>`

---

## 6) 已实现：tool 调用“轻模型”通道（v0.2.x）

目标：把“tool calling 的多 step 循环”放到更便宜的模型上跑，最终答复仍由 Router 选中的主模型生成。

### 6.1 配置

- env：`OMNE_AGENT_TOOL_MODEL="<model>"`（可选；为空/未设置则不启用）
- 若 thread config 强制了 `model`（例如 subagent 显式指定模型）：忽略 `OMNE_AGENT_TOOL_MODEL`

### 6.2 行为（实现口径）

- tool phase：
  - LLM 请求使用 `OMNE_AGENT_TOOL_MODEL`
  - tools 仍为 `auto`（允许调用工具）
  - 不发送 `item/delta`（避免把中间“碎碎念”流到客户端）
  - tool phase 的 assistant 文本不会注入后续上下文（只保留 tool calls 与 tool results）
- 当某次 tool phase 返回 **无 tool calls**：切回主模型，并在下一次 step 中禁用 tools（`tool_choice=none`）生成最终答复
- 切换会追加 `ModelRouted` 事件：
  - `tool_model: from=<final> to=<tool>; provider=<name>`
  - `tool_model_final: from=<tool> to=<final>; provider=<name>`（仅当发生切换时）

### 6.3 约束

- tool model 仍受 provider 的 `model_whitelist` 约束（不在白名单会直接报错）
