# `./.codepm_data/`：项目级数据根（目录约定）

> 目标：把“项目配置 + 运行时数据”收敛到一个明确位置，便于：
>
> - 多项目 attach/daemon（避免每个目录各起一套后端占内存）
> - 快速清理（tmp/threads/artifacts 一键删）
> - 防止 secrets 误入模型上下文/误提交 git

`pm init` 会在当前目录创建该结构（并生成对应 gitignore）。

---

## 1) 目录结构（v0.2.x 约定）

```
<project_root>/
  .codepm_data/
    config.toml
    config_local.toml
    .env
    .gitignore
    daemon.sock
    spec/
    tmp/
    data/
    repos/
    locks/
    logs/
    threads/
      <thread_id>/
        events.jsonl
        events.jsonl.lock
        artifacts/
          processes/<process_id>/{stdout.log,stderr.log,...}
          user/<artifact_id>.md
          user/<artifact_id>.metadata.json
```

说明：

- `config.toml`：项目级配置（**默认不生效**；见下文开关）。
- `config_local.toml`：本机/本用户的项目级配置（gitignore）。当它存在时，会优先于 `config.toml` 被加载。
- `.env`：项目级 secrets（例如 `OPENAI_API_KEY`）。**永远不提交**，且必须在 file tools 层默认拒绝读取。
- `.gitignore`：只忽略运行时/secret；不忽略 `config.toml` 与 `spec/`（便于提交/review）。
- `daemon.sock`：本机 daemon 的 unix socket（`pm-app-server --listen`）。运行时文件，**永远不提交**。
- `spec/`：项目可提交 spec（modes/workflow/hooks/router…）。具体文件名按后续 spec 定稿。
- `tmp/`：本项目的临时目录（可随时删；不作为正确性前提）。
- `data/`：运行时数据（预留；例如 session/索引/派生视图缓存；不提交）。
- `repos/`：运行时数据（预留；例如 bare repo cache；不提交）。
- `locks/`：运行时数据（预留；例如跨进程锁；不提交）。
- `logs/`：运行时数据（预留；不提交）。
- `threads/`：线程/事件/产物（运行时数据；不提交）。

---

## 2) `config.toml`：项目级配置开关（默认 false）

`config.toml` 必须包含一个显式开关，用于控制“是否启用项目级配置/覆盖”：

```toml
[project_config]
enabled = false
```

约定：

- 加载顺序：优先读取 `.codepm_data/config_local.toml`；不存在时读取 `.codepm_data/config.toml`。
- `enabled=false`：忽略所选 config 文件中除开关本身以外的字段；同时忽略 `.codepm_data/.env`。
- `enabled=true`：允许用 config 文件 + `.env` 覆盖 base_url/model 等配置（secrets 只来自 `.env`）。

OpenAI 配置示例（provider/profile + model-level thinking）：

```toml
[openai]
provider = "openai-codex-apikey"
model = "gpt-4.1"

[openai.providers.openai-codex-apikey]
base_url = "https://api.openai.com/v1"
# 可选：限制可用模型（用于 allowlist；可结合 provider 的 /models 做发现）
model_whitelist = ["gpt-4.1", "gpt-4o-mini"]

[openai.providers.openai-codex-apikey.auth]
type = "api_key_env" # 默认读取 OPENAI_API_KEY / CODE_PM_OPENAI_API_KEY

# 模型级思考强度（默认 medium）：
# unsupported/small/medium/high/xhigh
[openai.models."*"]
thinking = "medium"
[openai.models."codex-mini-latest"]
thinking = "xhigh"
```

---

## 3) `.env`：只放 secrets（建议键名）

建议只放 secrets（示例）：

```dotenv
OPENAI_API_KEY=...
# 可选：
CODE_PM_OPENAI_PROVIDER=openai-codex-apikey
CODE_PM_OPENAI_BASE_URL=https://api.openai.com/v1
CODE_PM_OPENAI_MODEL=gpt-4.1
```

注意：

- `.env` 属于高风险文件：必须被 gitignore，且默认禁止通过 file tools 读取。
- 不要把 token 写进 `config.toml`（避免误提交/误进上下文）。

---

## 4) `pm init` 生成的 gitignore（原则）

只忽略运行时/secret：

- 忽略：`.codepm_data/tmp/`、`.codepm_data/data/`、`.codepm_data/repos/`、`.codepm_data/threads/`、`.codepm_data/locks/`、`.codepm_data/logs/`、`.codepm_data/daemon.sock`、`.codepm_data/config_local.toml`、`.codepm_data/.env`
- 不忽略：`.codepm_data/config.toml`、`.codepm_data/spec/`
