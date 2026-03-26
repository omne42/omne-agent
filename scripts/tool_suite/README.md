# Tool Surface Benchmark (All Tools + Feature Matrix)

这个基准用于评估模型在 `OpenAI Responses` / `Chat Completions` 接口下的工具调用能力，覆盖：

- 当前仓库 `crates/app-server/src/agent/tools/spec.rs` 中的全部工具名（自动读取）
- 每个工具的 `single` 场景（一次调用直接成功）
- 每个工具的 `recovery` 场景（首轮注入错误，模型按提示调整后跑通）
- 每个 feature 的 4 档难度：`simple / normal / complex / advanced`
- facade 工具的**全功能矩阵**（按 `op` 全覆盖）：
  - `workspace`: `read/glob/grep/write/patch/edit/delete/mkdir`
  - `process`: `start/inspect/tail/follow/kill`
  - `thread`: `state/diff/events/usage/hook_run/request_input/spawn_agent/send_input/wait/close`
  - `artifact`: `write/update_plan/list/read/delete`
  - `integration`: `mcp_list_servers/mcp_list_tools/mcp_list_resources/mcp_call/web_search/web_fetch/view_image`

## 文件

- 运行脚本：`scripts/tool_suite/run_tool_surface_benchmark.py`
- provider 配置样例：`scripts/tool_suite/providers.example.json`
- 真实场景版（v2）：`scripts/tool_suite/README.v2.md`

## 运行

### 1) 全量（所有 tool + 所有 feature）

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark.py \
  --providers-file scripts/tool_suite/providers.example.json \
  --modes single,recovery \
  --difficulties simple,normal,complex,advanced \
  --parallel-providers 2
```

### 2) 只跑某个 provider

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark.py \
  --providers codex_responses
```

### 3) 只跑部分工具

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark.py \
  --providers codex_responses \
  --tools workspace,process,thread
```

### 4) 只跑指定难度

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark.py \
  --providers codex_responses \
  --difficulties advanced
```

### 5) 切换模型/接口

直接修改 `providers.example.json`（或传你自己的 JSON 文件）：

- `endpoint`: `responses` 或 `chat`
- `base_url`
- `model`
- `api_key_env`

脚本会从 `--env-file`（默认 `.omne_data/.env`）和系统环境读取密钥。

## 输出

每次运行写入 `docs/reports/tool-surface-benchmark-<timestamp>/`：

- `report.md`: 汇总报告（single first-pass、single eventual、recovery pass）
- `raw_results.json`: 全量结构化数据
- `details/<provider>/<mode>/<case_id>.json`: 每个 feature case 的完整链路
  - `system_prompt`
  - `user_prompt`
  - 每步 `request` / `response`
  - `tool_events`（tool 输入、执行结果、输出文本、执行时延）
  - usage / cache / final output / 质量判定

可选生成易读报告（默认只列非一次通过和失败项）：

```bash
python3 scripts/tool_suite/render_easy_markdown.py \
  --raw docs/reports/<run>/raw_results.json \
  --out docs/reports/<run>/summary.easy.md
```

## 指标定义

- `single_first_pass`: 第一次 tool 调用即成功，且最终回答合规
- `single_eventual_pass`: single 模式中最终跑通（允许第一次失败后修正）
- `recovery_pass`: recovery 模式首轮注入错误后，模型完成修正并最终跑通
