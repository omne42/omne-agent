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
    .env
    .gitignore
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
- `.env`：项目级 secrets（例如 `OPENAI_API_KEY`）。**永远不提交**，且必须在 file tools 层默认拒绝读取。
- `.gitignore`：只忽略运行时/secret；不忽略 `config.toml` 与 `spec/`（便于提交/review）。
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

- `enabled=false`：忽略 `.codepm_data/config.toml` 除开关本身以外的字段；同时忽略 `.codepm_data/.env`。
- `enabled=true`：允许用 `config.toml` + `.env` 覆盖项目内的 base_url/model 等配置（secrets 只来自 `.env`）。

---

## 3) `.env`：只放 secrets（建议键名）

建议只放 secrets（示例）：

```dotenv
OPENAI_API_KEY=...
# 可选：
CODE_PM_OPENAI_BASE_URL=https://api.openai.com
CODE_PM_OPENAI_MODEL=gpt-4.1
```

注意：

- `.env` 属于高风险文件：必须被 gitignore，且默认禁止通过 file tools 读取。
- 不要把 token 写进 `config.toml`（避免误提交/误进上下文）。

---

## 4) `pm init` 生成的 gitignore（原则）

只忽略运行时/secret：

- 忽略：`.codepm_data/tmp/`、`.codepm_data/data/`、`.codepm_data/repos/`、`.codepm_data/threads/`、`.codepm_data/locks/`、`.codepm_data/logs/`、`.codepm_data/.env`
- 不忽略：`.codepm_data/config.toml`、`.codepm_data/spec/`
