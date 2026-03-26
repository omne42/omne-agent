# `./.omne_data/`：项目级数据根（目录约定）

> 目标：把“项目配置 + 运行时数据”收敛到一个明确位置，便于：
>
> - 多项目 attach/daemon（避免每个目录各起一套后端占内存）
> - 快速清理（tmp/threads/artifacts 一键删）
> - 防止 secrets 误入模型上下文/误提交 git

`omne init` 会在当前目录创建该结构（并生成对应 gitignore）。

---

## 1) 目录结构（v0.2.x 约定）

```
<project_root>/
  .omne_data/
    config.toml
    config_local.toml
    .env
    .gitignore
    daemon.sock
    spec/
    tmp/
    data/
    repos/
    reference/
    keys/
    locks/
    logs/
    threads/
      <thread_id>/
        events.jsonl
        events.jsonl.lock
        readable_history.jsonl
        artifacts/
          user/<artifact_id>.md
          user/<artifact_id>.metadata.json
        runtime/
          processes/<process_id>/{stdout.log,stderr.log,...}
          llm_stream/<turn_id>.jsonl
          llm_stream/<turn_id>.request_body.json
```

说明：

- `config.toml`：项目级配置（**默认不生效**；见下文开关）。
- `config_local.toml`：本机/本用户的项目级配置（gitignore）。当它存在时，会优先于 `config.toml` 被加载。
- `.env`：项目级 secrets（例如 `OPENAI_API_KEY`）。**永远不提交**，且必须在 file tools 层默认拒绝读取。
- `.gitignore`：只忽略运行时/secret；不忽略 `config.toml` 与 `spec/`（便于提交/review）。
- `daemon.sock`：本机 daemon 的 unix socket（`omne-app-server --listen`）。运行时文件，**永远不提交**。
- `spec/`：项目可提交 spec（modes/workflow/hooks/router…）。具体文件名按后续 spec 定稿。
- `tmp/`：本项目的临时目录（可随时删；不作为正确性前提）。
- `data/`：运行时数据（预留；例如 session/索引/派生视图缓存；不提交）。
- `repos/`：运行时数据（预留；例如 bare repo cache；不提交）。
- `reference/`：Reference repo/snapshot（只读参考；不提交；见 `docs/reference_repo.md`）。
- `keys/`：本地加密密钥材料（例如 Responses raw history 的本地密钥文件）；运行时数据，**永远不提交**。
- `locks/`：运行时数据（预留；例如跨进程锁；不提交）。
- `logs/`：运行时数据（预留；不提交）。
- `threads/`：线程/事件/产物（运行时数据；不提交）。每个 thread 目录包含 `events.jsonl`（真相源）与派生的 `readable_history.jsonl`（用户可读对话），以及 `artifacts/`（用户产物）与 `runtime/`（内部运行时落盘，如 process logs/LLM debug）。

---

## 2) `config.toml`：项目级配置开关（默认 false）

`config.toml` 必须包含一个显式开关，用于控制“是否启用项目级配置/覆盖”：

```toml
[project_config]
enabled = false
```

约定：

- 加载顺序：优先读取 `.omne_data/config_local.toml`；不存在时读取 `.omne_data/config.toml`。
- `enabled=false`：忽略所选 config 文件中除开关本身以外的字段；同时忽略 `.omne_data/.env`。
- `enabled=true`：允许用 config 文件 + `.env` 覆盖 base_url/model 等配置（secrets 只来自 `.env`）。

可选 UI 配置：

```toml
[ui]
# 是否把模型的 thinking/reasoning 流式展示给客户端（默认 true）
show_thinking = true
```

OpenAI 配置示例（provider/profile + model-level thinking）：

```toml
[openai]
provider = "openai-codex-apikey"
model = "gpt-4.1"
# 可选：provider fallback（当 429/5xx/timeout 时依次尝试）
fallback_providers = ["openai-auth-command"]

[openai.providers.openai-codex-apikey]
base_url = "https://api.openai.com/v1"
# 可选：限制可用模型（用于 allowlist；可结合 provider 的 /models 做发现）
model_whitelist = ["gpt-4.1", "gpt-4o-mini"]

[openai.providers.openai-codex-apikey.auth]
type = "api_key_env" # 默认读取 OPENAI_API_KEY / OMNE_OPENAI_API_KEY

# 模型级思考强度（默认 medium）：
# unsupported/small/medium/high/xhigh
[openai.models."*"]
thinking = "medium"

# 可选：覆盖模型的上下文窗口与 compact 阈值。
# `context_window` 表示模型窗口；`auto_compact_token_limit` 表示当前 prompt-load
# 达到该值时触发 compact。若未设置 `auto_compact_token_limit`，运行时会按
# `context_window * OMNE_AGENT_AUTO_SUMMARY_THRESHOLD_PCT / 100` 计算（默认 80%）。
[openai.models."gpt-4.1"]
thinking = "high"
context_window = 1047576
auto_compact_token_limit = 900000

[openai.models."codex-mini-latest"]
thinking = "xhigh"
```

---

## 3) `.env`：只放 secrets（建议键名）

建议只放 secrets（示例）：

```dotenv
OPENAI_API_KEY=...
# 可选：
OMNE_OPENAI_PROVIDER=openai-codex-apikey
OMNE_OPENAI_BASE_URL=https://api.openai.com/v1
OMNE_OPENAI_MODEL=gpt-4.1
# 可选：逗号分隔的 fallback provider 列表（优先级高于 config.toml 的 `openai.fallback_providers`）
OMNE_OPENAI_FALLBACK_PROVIDERS=openai-auth-command,openai-codex-apikey

# 可选：Responses raw history 存储编码（默认 plaintext；如需本地落盘加密可设为 encrypted）
# OMNE_OPENAI_RESPONSES_HISTORY_CODEC=plaintext
OMNE_OPENAI_RESPONSES_HISTORY_CODEC=encrypted
# 可选：显式覆盖 Responses raw history 密钥（建议使用 base64）
OMNE_OPENAI_RESPONSES_HISTORY_KEY_B64=...
```

注意：

- `.env` 属于高风险文件：必须被 gitignore，且默认禁止通过 file tools 读取。
- 不要把 token 写进 `config.toml`（避免误提交/误进上下文）。

---

## 4) `omne init` 生成的 gitignore（原则）

只忽略运行时/secret：

- 忽略：`.omne_data/tmp/`、`.omne_data/data/`、`.omne_data/repos/`、`.omne_data/reference/`、`.omne_data/keys/`、`.omne_data/threads/`、`.omne_data/locks/`、`.omne_data/logs/`、`.omne_data/daemon.sock`、`.omne_data/config_local.toml`、`.omne_data/.env`
- 不忽略：`.omne_data/config.toml`、`.omne_data/spec/`
