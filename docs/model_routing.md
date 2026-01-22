# Model / Provider / Router（模型选择与可解释性）（v0.2.0 现状 + TODO）

> 目标：把“用哪个模型、为什么”变成可解释、可回放、可审计的事实；并为后续的 role/keyword/subagent 路由预留最小扩展点。

---

## 0) 范围与非目标

范围（v0.2.0）：

- 当前已实现的 model/base_url 配置来源与覆盖顺序。
- `thread/config/explain` 的输出口径（回答“为什么生效的是这个值”）。

非目标（v0.2.0）：

- 多 provider 抽象与 capability flags（仍是 TODO，见 `docs/v0.2.0_parity.md`）。
- role/keyword/subagent 的自动路由（仍是 TODO）。
- 自动升降级、fallback、长上下文阈值切换（仍是 TODO）。

---

## 1) v0.2.0 现状：模型选择（已实现）

### 1.1 生效顺序（写死）

`provider` 的生效顺序（用于派生 base_url/auth/default_model）：

1. project config（当 `project_config.enabled=true`）：`openai.provider`
2. env：`CODE_PM_OPENAI_PROVIDER`
3. default：`openai-codex-apikey`

当前 turn 的 `model` 生效顺序为：

1. thread config（来自 `ThreadConfigUpdated` 事件）
2. project config（当 `project_config.enabled=true`）：`openai.model`
3. env：`CODE_PM_OPENAI_MODEL`
4. provider default（当存在）：`openai.providers.<provider>.default_model`
5. default：`model="gpt-4.1"`

当前 turn 的 `openai_base_url` 生效顺序为：

1. thread config（来自 `ThreadConfigUpdated` 事件）
2. project config（当 `project_config.enabled=true`）：`openai.base_url`（legacy；建议迁移到 provider profile）
3. env：`CODE_PM_OPENAI_BASE_URL`
4. provider base_url：`openai.providers.<provider>.base_url`（或 builtin default）
5. default：`openai_base_url="https://api.openai.com/v1"`

> 注意：实现中 default 在多个位置硬编码；未来如果改默认值，必须同步，避免 explain 与实际漂移。

### 1.2 落盘（可回放）

- 每次 assistant 输出会落盘 `AssistantMessage { model: Option<String>, response_id, token_usage? }`。
- v0.2.0 **不会**落盘 `reason/rule_source`（因为还没有 Router），这在 `docs/v0.2.0_parity.md` 里是 TODO。

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
pm thread config-explain <thread_id> --json
```

列出 provider 可用模型（`GET /models` + provider allowlist）：

```bash
pm thread models <thread_id> --json
```

实现对照：

- `crates/app-server/src/main/thread_manage/config.rs`（`handle_thread_config_explain`）

---

## 3) TODO：Router（role/keyword/subagent）最小规格草案

> 目标态定义在 `docs/v0.2.0_parity.md`。这里给一个“可实现的最小规格”，避免未来实现跑偏。

### 3.1 路由优先级链（已定，写死）

`subagent 强制 > project override > keyword rule > role default > global default`

### 3.2 配置文件位置（建议写死）

约定：Router 属于项目可提交 spec（`./.codepm_data/spec/`）；运行时数据位于 `.codepm_data/{tmp,threads,...}`，不参与发现/解析。

发现顺序（高 → 低）建议写死：

1. env：`CODE_PM_ROUTER_FILE`（绝对或相对路径；相对路径按 thread cwd 解析）
2. `./.codepm_data/spec/router.yaml`（推荐）
3. `./.codepm_data/spec/router.json`（可选）

若 env 指向的文件不存在或解析失败：建议直接报错（fail-closed），避免“以为生效但其实没生效”。

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

每个 turn 必须记录一次路由决策（TODO）：

- `selected_model`
- `rule_source`：`subagent|project_override|keyword_rule|role_default|global_default`
- `reason`（可选但强烈建议）
- `rule_id`（可选；便于审计）

落盘位置建议二选一（都行，但必须稳定）：

1. 新事件：`ModelRouted { turn_id, selected_model, rule_source, reason?, rule_id? }`
2. 扩展 `TurnStarted`：新增 `routing` 字段（避免事件爆炸，但会扩协议）

`rule_source="global_default"` 的口径（建议写死）：

- 表示 Router 没有覆盖；`selected_model` 等于当前 `thread/config/explain.effective.model`（即配置层的最终生效 model）。
- `reason` 可选写清楚“来自 thread/env/default 哪一层”（但不要依赖它做机器判定）。

验收（未来实现时）：

- `pm thread events <thread_id> --json` 能看到每个 turn 的 `selected_model + reason + rule_source`。

---

### 3.5 TODO：上下文阈值（long-context）最小规格

> 目标：当上下文接近模型上限时，避免“隐式截断/隐式退化”，而是做一次**可解释的路由或压缩**。
>
> 注意：v0.2.0 还没有 Router 实现；这里只定义口径，避免未来实现漂移。

最小建议（不扩优先级链，复用 `keyword_rules` 槽位）：

- 允许 `keyword_rules` 的一条规则用 “上下文阈值” 触发，而不是关键词触发：
  - `min_context_tokens`：整数（估算的上下文 tokens）
  - `keywords`：可选；若同时存在，必须同时满足
- 匹配顺序仍按列表顺序第一条命中为准（写死，避免竞赛）。
- 命中后：
  - `selected_model = rule.model`
  - `rule_source = "keyword_rule"`（保持已有枚举口径）
  - `rule_id = rule.id`
  - `reason` 建议包含阈值与估算值（便于审计）

配置示例（草案）：

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

关于 “context_tokens” 的估算（先别过度设计）：

- MVP 允许粗估（例如用 provider 的 tokenizer 工具或字符长度近似），但必须把 “估算方式/阈值/原因”写入 `reason`，避免黑箱。

压缩（auto compact/summary）的关系：

- “切 long-context” 与 “compact/summary” 可以先只做一个。
- 如果两者都实现，建议优先级写死为：
  1. 有 long-context model 且命中阈值 → 路由到 long-context
  2. 否则 → 触发 compact/summary（规格草案见 `docs/budgets.md`）
