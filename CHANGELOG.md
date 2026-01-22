# Changelog

本项目的所有重要变更都会记录在这个文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added
- 新增 Agent-first 重新开发流程文档：`docs/development_process.md`。
- 新增 vNext 目标态“RTS 风格使用流程”文档：`docs/rts_workflow.md`。
- 新增 v0.2.x 文档索引与 v0.2.0 口径补充（approval/attention/event model/runtime/execpolicy 等，详见 `docs/README.md`）。
- 新增项目级数据根与运行时目录约定文档：`docs/codepm_data.md`、`docs/runtime_layout.md`，并补齐 daemon/TUI 设计草案：`docs/daemon.md`、`docs/tui.md`。
- 补齐 Subagents/Fan-out 文档：加入 Codex/OpenCode/Claude Code/Antigravity 对照、v0.2.x 务实 DoD 与 A/B/C 选型建议（`docs/subagents.md`）。
- 新增 Agent GUI 爆发期产品调研：`docs/research/onecode.md`、`docs/research/superset.md`、`docs/research/aion-ui.md`。
- 新增 `v0.2.0` 功能对齐与 TODO 汇总：`docs/v0.2.0_parity.md`。
- 新增 Mode（角色权限边界）规范：`docs/modes.md`（配置发现顺序、`deny/prompt/allow` 语义、`prompt+auto_approve` 的落盘审计规则）。
- `pm-core`/`pm-app-server`：落地 ModeCatalog（`.codepm_data/spec/modes.yaml` / `CODE_PM_MODES_FILE`），并在 `file/*` 与 `process/start` 工具入口强制执行 `mode` 的 `deny` 边界（未知 mode 也会拒绝并返回可用列表）。
- `pm-core`/`pm-app-server`：新增 `subagent.spawn` 权限边界，并在 `agent_spawn` 入口强制执行（默认子 thread `sandbox_policy=read_only` + `mode=reviewer`，并支持 `CODE_PM_MAX_CONCURRENT_SUBAGENTS` 并发上限，超限拒绝并返回原因）。
- `pm-app-server`：支持 per-mode execpolicy rules（`.codepm_data/spec/modes.yaml` 的 `permissions.command.execpolicy_rules`），并在 `process/start` 入口 fail-closed 执行（加载失败即拒绝）。
- `pm-app-server thread/config-explain`：追加 mode 可解释性（mode catalog 来源/路径/加载错误、可用 modes、当前 mode 的权限摘要与 glob 列表）。
- 更新仓库级 `AGENTS.md`：补齐 `crates/*` 结构、Rust gates 与 `pm*` 入口说明。
- 新增 `pm-jsonrpc`：最小 JSON-RPC over stdio client，用于驱动 `pm-app-server`；并支持接收/转发 JSON-RPC notifications（用于 `item/delta` 等流式事件）。
- 新增 `pm-app-server-protocol`：导出 app-server 协议的 TypeScript types 与 JSON Schema；`pm-app-server generate-ts --out <dir>` / `pm-app-server generate-json-schema --out <dir>` 可生成对应产物。
- 新增 `pm-protocol`/`pm-eventlog`：为 v0.2.0 落地 Thread/Turn 事件类型与 append-only JSONL event log（thread_id 一致性校验、`seq` 连续、`since_seq` 断点续读、尾部半行自动截断，并提供 `ThreadState` 纯事件派生）。
- `pm-protocol`/`pm-eventlog`：新增 thread `pause/unpause` 事件（`ThreadPaused/ThreadUnpaused`）与派生状态字段（`paused*`）。
- 新增 `pm-execpolicy`：对齐 Codex `prefix_rule` 子集的执行策略引擎（Starlark 语法 + `match/not_match` 例子校验），并提供 `pm-execpolicy check --rules ... <cmd...>` 输出匹配结果 JSON。
- 新增 `pm-openai`：最小 OpenAI Responses API 客户端与类型（用于 v0.2.0 的 Responses-first agent loop）。
- `pm-openai`：新增 Responses SSE 流式解析与 `Client::create_response_stream`（`response.output_text.delta`/`response.output_item.done`/`response.completed`），为 `item/delta` 与更强可观测性打底。
- `pm-openai`：新增 `reasoning.effort`（`low|medium|high|xhigh`）请求字段支持；`pm-app-server` 可按模型配置下发（见 `openai.model_reasoning_effort`）。
- `pm-openai`/`pm-app-server`：新增 `response_format` 支持（JSON schema），默认关闭，可通过 `CODE_PM_AGENT_RESPONSE_FORMAT_JSON` 启用。
- `pm-openai`：SSE 事件强类型化：`TokenUsage`/`RateLimits`/`ApiError`，并支持 `response.failed` → `ResponseEvent::Failed`；`pm-app-server` agent loop 会消费 typed usage 并把 failed 作为错误返回。
- `pm-app-server`：新增 OpenAI provider 选择（`openai.provider` / `CODE_PM_OPENAI_PROVIDER`），首个 provider `openai-codex-apikey`；并支持 `openai-auth-command`（运行外部命令返回 `{ "api_key": "..." }`，便于 Node 插件化 auth）。
- 新增 `ditto-llm`：以 provider profile 为中心的 `auth/base_url/model whitelist` 配置与 OpenAI-compatible `/models` 发现；并支持 model-level `thinking`（`unsupported/small/medium/high/xhigh`，默认 `medium`），`pm-app-server` 用其派生 `reasoning.effort`。
- `pm-app-server`/`pm`：新增 `thread/models`（`GET /models` + provider whitelist）与 `pm thread models`，用于发现当前 provider 的可用模型。
- 新增 `pm-app-server`：最小 JSON-RPC over stdio 控制面（`initialize` + `thread/*` + `turn/*`），用于验证 v0.2.0 的 thread/turn/interrupt 与落盘回放。
- 新增 `pm` CLI：作为 `pm-app-server` 的人类可用客户端，支持 `ask/watch --bell`、`thread/*`、`approval/*`、`process/*`（只读查看 + interrupt/kill），并在 `ask` 中支持 Ctrl-C 触发 `turn/interrupt`。
- `pm` CLI 新增 `pm init`：初始化 `./.codepm_data/`（创建目录、生成 `config.toml`、可选 `.env` 模板与 `spec/`，并写入 `.codepm_data/.gitignore`）。
- `pm-app-server`：新增 unix socket daemon 模式：`--listen <pm_root>/daemon.sock`（允许多 client attach；client 断线可重连）。
- `pm` CLI：默认优先连接 `<pm_root>/daemon.sock`，失败 fallback 到 spawn `pm-app-server`（保持 JSON-RPC 语义不变）。
- `pm-app-server`：支持 `.codepm_data/config_local.toml`（gitignore）作为本机项目配置；当其存在时会优先于 `.codepm_data/config.toml` 被加载。
- `pm init`：新增 `--create-config-local`（交互模式也可选），用于生成 `.codepm_data/config_local.toml` 模板。
- 仓库内提交 `./.codepm_data/config.toml` 与 `./.codepm_data/.gitignore` 作为默认模板（由 `pm init --yes` 生成；`.codepm_data/.gitignore` 会忽略 `config_local.toml` 与 `.env`）。
- `pm` CLI 新增 `exec`：非交互执行单次 turn（CI/脚本友好），支持 `--json` 输出摘要与 `--on-approval fail|approve|deny` 策略。
- `pm` CLI 新增交互式 REPL：直接运行 `pm`（或 `pm repl`）进入对话/执行环境，支持 `/help` 等指令。
- `pm ask`：消费 `pm-app-server` 的 `item/delta` notifications 并实时输出 assistant 文本流（仅作为 UI 优化；最终仍以 `AssistantMessage` 落盘为准）。
- `pm` CLI 新增 `inbox`：跨 thread 的 RTS 收件箱视图（可 `--watch` + `--bell` 去重提醒），用于快速发现 `need_approval/failed/running`。
- `pm inbox --details`：现在会显示 `failed_processes` 摘要（数量 + 部分 id），便于快速定位后台失败。
- `pm-app-server thread/attention`：新增 `stale_processes`（running process 在 `idle_window` 内无新输出），并支持 `CODE_PM_PROCESS_IDLE_WINDOW_SECONDS` 配置（`0` 禁用；默认 300s）。
- `pm watch --bell` / `pm inbox --bell`：当 `stale_processes` 从空变非空时响铃提醒（避免后台进程“无输出但不退出”时静默卡住）。
- `pm` CLI 补齐更多控制面命令：`pm thread fork/archive/unarchive/delete/clear-artifacts/disk-*` 与 `pm artifact list/read/delete`，便于手动清理与审计。
- `pm-protocol`/`pm-app-server`：artifact metadata 新增可选 `preview`（`kind/language/title`），并在 `artifact/write` 时按 `artifact_type` 自动填充（用于 diff/patch/html/code/log 的预览渲染提示；见 `docs/artifacts.md`）。
- `pm-app-server`：user artifacts 支持 bounded history（`CODE_PM_ARTIFACT_HISTORY_MAX_VERSIONS`；覆盖写入时保存旧内容到 `artifacts/user/history/<artifact_id>/v####.md`，并自动保留最近 N 个旧版本；见 `docs/artifacts.md`）。
- `pm-app-server`/`pm`：新增 `thread/diff`（安全模式 `git diff --no-ext-diff --no-textconv`）生成 diff user artifact（`artifact_type="diff"`）；CLI: `pm thread diff <thread_id>`。
- `pm` CLI 新增 `pm thread spawn`：对 `thread/fork + turn/start` 的便捷封装（可选覆盖 model/openai_base_url），用于并行出发后台 turns。
- `pm-app-server` 新增 `thread/hook_run`：读取 `<thread root>/.codepm_data/spec/workspace.{yaml,yml}` 并按 `setup/run/archive` 启动对应 hook 命令（复用 `process/start` 的 mode/execpolicy/approvals）；`pm` CLI 增加 `pm thread hook-run <thread_id> <setup|run|archive>` 用于触发。
- `pm-app-server` agent loop：新增 `thread_hook_run` tool，允许 agent 直接触发当前 thread 的 workspace hooks（同样复用 mode/execpolicy/approvals）。
- `pm` CLI 新增可解释性与状态查询：`pm thread state`、`pm thread config-explain`、`pm thread loaded`。
- `pm`/`pm-app-server`：thread 配置新增 `mode`（角色/权限边界）字段；`pm ask/exec/thread configure --mode <name>` 可设置；`thread/state`/`thread/config-explain` 会返回当前生效的 `mode`。
- 新增 `TurnStatus::Stuck`：当 agent 超预算/超时（turn 时长、tool call、OpenAI 请求超时等）时显式标记为 `stuck`，并在 `thread/attention` 与 `pm * --bell` 中可见。
- `pm-app-server`：当 turn 结束为 `TurnStatus::Stuck` 时自动写入 `artifact_type="stuck_report"`（provenance 关联 `turn_id`），提供“原因/定位/下一步命令”的最小摘要。
- `pm-app-server` agent loop：新增 `CODE_PM_AGENT_MAX_*` 预算覆盖（steps/tool calls/turn seconds/OpenAI request timeout）。
- `pm-app-server` 在退出前会尽力终止仍在运行的子进程（避免 CLI 关闭后留下孤儿进程）。
- `pm-app-server` 新增 `initialized`（握手确认）与 `thread/loaded`（列出当前已加载 threads）。
- `pm-app-server` 新增 `thread/list_meta`：批量返回 threads 的派生状态（支持 `include_archived`），减少 UI/CLI 人肉遍历与重复读取 event log。
- `pm-app-server` 新增 Responses-first agent loop：`turn/start` 会调用 OpenAI Responses API 执行 tool calling（阻塞等待 approval 决策并复跑同一 tool call），并将 assistant 输出落盘为 `AssistantMessage` 事件以支持 resume。
- `pm-app-server` agent loop：会读取 `<thread cwd>/AGENTS.md` 并追加到 instructions（写入前自动脱敏），让项目级规范从第一天就能约束 agent。
- `pm-app-server` agent loop：支持 instructions layering（base/user/project）与按需加载 skills（`$skill` → `SKILL.md`），并提供 `CODE_PM_USER_INSTRUCTIONS_FILE`/`CODE_PM_SKILLS_DIR`。
- `pm-app-server`：支持项目级 OpenAI 配置覆盖（默认关闭）：当 `.codepm_data/config.toml` 的 `[project_config].enabled=true` 时，从 `.codepm_data/config.toml` + `.codepm_data/.env` 加载 `base_url/model/api_key` 覆盖。
- `pm-app-server` agent loop：构建对话上下文时会把 tool/approval/process/turn-status 事件注入 history（resume 更接近 Codex 语义，减少重复执行与“失忆”）。
- 新增 `pm-core::threads`：`ThreadStore` + `ThreadHandle`，基于 JSONL event log 实现 thread 创建/列举/resume（resume 会修复未完成 turn/进程并落盘）。
- 新增 `pm-core::sandbox`：rooted path 解析与边界校验（拒绝 `..` 穿越与 symlink 逃逸）。
- `pm-app-server` 新增 `thread/state`：返回 thread 派生状态（active turn、`last_seq`、interrupt 标记）。
- `pm-app-server` 新增 `thread/fork`：复制 thread 的对话/审批/配置事件到新 thread（跳过 tool/process 事件与 thread 专属 artifact 路径，并自动跳过进行中的 active turn），用于多子 agent 并行。
- `pm-app-server` 新增 `thread/archive`/`thread/unarchive`：归档 thread（默认拒绝含 active turn/running process；`force=true` 可中断 turn 并终止进程）。
- `pm-app-server` 新增 `thread/pause`/`thread/unpause`：暂停/恢复 thread（pause 会尽力中断 active turn 并终止该 turn 启动的进程），并在 `thread/attention` 与 `thread/state` 中暴露 `paused` 状态；`turn/start` 现在会拒绝对 `archived/paused` thread 启动新 turn；`pm thread pause/unpause` 提供对应 CLI。
- `pm-app-server thread/events`：支持 `max_events` 分页，并返回 `has_more`/`thread_last_seq` 便于订阅端处理 lag 与续读。
- `pm-app-server thread/subscribe`：长轮询读取 thread events（`wait_ms` 超时），用于实现“不断线不丢”的订阅式消费（`since_seq` + `seq` 去重）。
- `pm-app-server`：追加 `ThreadEvent` 时会同时发送 JSON-RPC notifications（`thread/event`、`turn/*`、`item/*`），用于 UI/客户端实时渲染；掉线可用 `thread/subscribe` 从 `since_seq` 重放补齐。
- `pm-app-server` agent loop：Responses SSE 流式执行（`response.output_text.delta`）并转发为 `item/delta` JSON-RPC notifications（文本流）；最终仍以 `AssistantMessage` 落盘为准（断线不丢）。
- `pm-app-server` 新增 thread 清理 API：`thread/delete(force?)` 与 `thread/clear_artifacts(force?)`，用于一键清除 history 与中间态产物。
- `pm-app-server` 新增 approvals 控制面：`thread/configure(approval_policy,sandbox_policy?)`、`approval/list`、`approval/decide`。
- `pm-protocol`/`pm-app-server`：新增 `ApprovalPolicy::AutoDeny`（仍会落盘 `ApprovalRequested/ApprovalDecided`，但会自动拒绝并返回 `denied=true`），便于非交互/保守模式下避免卡在 NeedApproval。
- `pm-app-server` 新增 `thread/config/explain`：返回最小 config layer stack（当前覆盖 `approval_policy`/`sandbox_policy`/`model`/`openai_base_url`），用于回答“为什么生效的是这个值”。
- `pm-protocol`/`pm-eventlog` 新增 thread-level 模型配置：`ThreadConfigUpdated.model/openai_base_url` + `ThreadState.model/openai_base_url`。
- `pm-app-server` 新增 `thread/attention`：派生 RTS “收件箱”视图（pending approvals + running processes），减少 UI/CLI 人肉扫描 event log。
- `pm-app-server` 新增 `thread/disk_usage` 与 `thread/disk_report`：返回 thread 目录磁盘占用，并可生成 `disk_report` markdown artifact 便于清理。
- `pm-app-server`：订阅 `thread/subscribe` 时会按阈值检测 thread 磁盘占用并生成 `disk_report` 告警 artifact（默认 10GiB；`CODE_PM_THREAD_DISK_WARNING_BYTES=0` 可关闭；频率可用 `CODE_PM_THREAD_DISK_CHECK_DEBOUNCE_MS`/`CODE_PM_THREAD_DISK_REPORT_DEBOUNCE_MS` 控制）。
- `pm-app-server` 新增 `process/*`：`process/start`（落盘 stdout/stderr）、`process/list`、`process/inspect`（元信息 + tail）、`process/tail`（只读查看）、`process/follow`（增量查看）、`process/interrupt`（软中断）、`process/kill`（终止后台进程）。
- `pm-app-server process logs`：stdout/stderr 自动分片 rotate（默认 `8MiB`，可用 `CODE_PM_PROCESS_LOG_MAX_BYTES_PER_PART` 覆盖），`process/tail`/`process/follow` 会跨分片读取（offset 语义保持连续）。
- `pm-app-server` 新增 `file/*`：`file/read`、`file/glob`、`file/grep`、`file/write`、`file/patch`、`file/edit`、`file/delete`（带 rooted path 校验，并记录 `ToolStarted/ToolCompleted` 事件）。
- `pm-app-server` 新增 `fs/*`：`fs/mkdir`（带 rooted path 校验，并记录 `ToolStarted/ToolCompleted` 事件）。
- `pm-app-server` 新增 `artifact/*`：`artifact/write`（`.md + .metadata.json` 落盘并自动脱敏）、`artifact/list`、`artifact/read`、`artifact/delete`。
- `pm-app-server` agent loop tool 覆盖：补齐 `file/edit`、`file/delete`、`process/tail`、`process/follow`、`artifact/list`、`artifact/read`、`artifact/delete`。
- `pm-app-server agent loop`：新增 `agent_spawn`（fork + 启动子 agent turn）与 `thread_state`/`thread_events`（fan-in 读状态与事件）。
- `pm-app-server`：`file/read|glob|grep` 支持 `root="reference"`（读取 `.codepm_data/reference/repo` 的只读快照），并确保 workspace 的 `glob/grep` 默认不扫描 `.codepm_data/reference`。
- `pm-app-server`：新增 `repo/search` 与 `repo/index`：将搜索结果/文件清单写入 `repo_search`/`repo_index` user artifact（结果可引用/可回放；tool 事件只记录摘要 + `artifact_id`）。
- `pm` CLI：新增 `pm repo search/index`（支持 `--approval-id` 重试）。
- `pm` CLI：新增 `pm reference import/status`（导入本地目录为 reference repo；导入时不复制 `.git/`、默认跳过单文件 `> 10MB` 并生成 `manifest.json`），`pm init` 同步创建 `.codepm_data/reference/` 并写入 `.codepm_data/.gitignore`。

### Changed
- `pm`/`pm-app-server`：`pm_root` 默认目录改为 `./.codepm_data/`（可用 `--pm-root` 或 `CODE_PM_ROOT` 覆盖）。
- （breaking）project spec 目录固定为 `./.codepm_data/spec/`（modes/workspace hooks/skills 等），不再支持 legacy `.codepm/`/`.code_pm/` 路径。
- 重写 `docs/implementation_plan.md`：以 Agent CLI（tool/sandbox/approvals + 事件流）为核心基建，Git 降级为交付适配层，并明确 RTS 控制面最小能力集。
- 更新 `docs/development_process.md`：补齐 RTS 风格交互要求（attention/inbox、pause/interrupt/step），并把 workspace hooks（setup/run/archive）与 artifacts/preview 明确进里程碑。
- `docs/workflow.md` 标注为 `v0.1.1` legacy，`docs/start.md` 增加 vNext 文档导航。
- Rust workspace：重命名 `crates/*` 目录以去掉 `pm` 前缀并增强语义（例如 `pm-app-server` → `app-server`、`pm-core` → `core`、`code-pm` → `legacy-cli`）；crate/package 名称保持不变，仅路径变更。
- `pm-app-server process/start`：当 `sandbox_network_access=deny` 时，拒绝明显网络命令（best-effort 防呆；非 OS 级网络沙箱），需要联网可显式配置 `sandbox_network_access=allow`。
- `pm-app-server` agent loop：支持 read-only tool calls 并发执行与结果聚合（默认关闭；`CODE_PM_AGENT_PARALLEL_TOOL_CALLS=1` 启用，`CODE_PM_AGENT_MAX_PARALLEL_TOOL_CALLS` 限制并发数）。
- `pm-app-server` agent loop：支持 token budget（`CODE_PM_AGENT_MAX_TOTAL_TOKENS`；超限标记为 `stuck`）。
- `pm-openai`/`pm-app-server`：OpenAI Responses 请求 URL 改为 `base_url + /responses`（不再固定拼 `/v1/responses`）；默认 `openai_base_url` 统一为 `https://api.openai.com/v1`。
- 更新 `docs/research/README.md`：补齐新增调研条目并调整落地方向表述。
- 更新 `docs/v0.2.0_parity.md`：同步 `item/* notifications` 与通知去重/节流的落地状态（`pm watch|inbox --debounce-ms`）。
- 更新 `docs/v0.2.0_parity.md`：补齐 “Item 覆盖” 勾选状态（见 `docs/thread_event_model.md`）。
- `githooks/pre-commit`：强制每次提交同时包含 `CHANGELOG.md` 与实际变更（禁止 changelog-only / non-changelog commit）。
- `pm-app-server agent loop`：增加最小长任务预算（`max_steps`/`max_tool_calls`/`max_turn_seconds` + 单次 OpenAI 请求超时）；超限会使 turn 失败并写入失败原因，避免无限循环烧钱/卡死。
- v0.2.0 方向明确：git/workspace 使用 `/tmp`/worktree 等目录隔离，不把 Docker/容器当作实现前提（但不禁止 agent 自己运行 Docker）；实现文档中移除/替换相关表述。
- `pm-eventlog ThreadState`：记录 thread 的 `cwd`；`pm-app-server thread/state` 返回 `cwd` 便于后续 sandbox/root 约束。
- `pm-eventlog ThreadState`：增加 `approval_policy`（默认 auto-approve）；`pm-app-server thread/state` 返回当前策略。
- `pm-eventlog ThreadState`：增加 `sandbox_policy`（默认 `workspace_write`）；`pm-app-server thread/state`/`thread/attention` 返回当前策略。
- `pm-eventlog ThreadState`：增加 `model`/`openai_base_url`（默认跟随 env/default）；`pm-app-server thread/state`/`thread/attention`/`thread/config/explain` 返回当前值。
- `pm-eventlog ThreadState`：增加 `archived/archived_at/archived_reason` 与 `last_turn_id/last_turn_status/last_turn_reason`；`pm-app-server thread/state`/`thread/attention` 返回归档与 last turn 信息，并派生 `attention_state`（含 `archived`）。
- `pm-app-server thread/configure`：`approval_policy` 现在可选（省略时沿用当前），并支持设置 `model`/`openai_base_url`（thread override）。
- `pm-protocol`/`pm-eventlog`/`pm-app-server`/`pm`：thread config 新增 `sandbox_writable_roots`/`sandbox_network_access`；`file/write|patch|edit|delete` 与 `fs/mkdir` 在非 `danger-full-access` 下支持写入额外 roots（仍硬防 `..`/symlink 逃逸）。
- `pm-protocol`/`pm`/`pm-app-server approvals`：`approval_policy` 新增 `on_request`/`unless_trusted`（`unless_trusted` 对 `process/start` 在 execpolicy=allow 时自动批准，否则要求人工批准）。
- `pm-app-server`：当 `approval_policy=manual` 时，`file/write`/`file/delete`/`fs/mkdir`/`process/start` 会返回 `needs_approval` 并写入 `ApprovalRequested`；提供 `approval_id` 且已 `approval/decide` 后才会执行。
- `pm-app-server`：当 `sandbox_policy=read_only` 时，`file/write`/`file/patch`/`file/edit`/`file/delete`/`fs/mkdir`/`process/start` 会直接拒绝（ToolStatus=Denied）。
- `pm-app-server approvals`：`approval/decide` 支持 `remember=true`（session 内记忆 approve/deny），同类操作无需重复弹审批；拒绝也会被记住并直接拦截。
- `pm-app-server process/start`：引入 `pm-execpolicy` gate（`prefix_rule`）：`forbidden` 直接拒绝并写入 `ToolStatus::Denied`；`manual` 策略下仅当 `prompt`/未匹配时才要求 approval（用 allowlist 降低骚扰）。
- `pm-app-server turn/interrupt`：会先对同一 turn 下仍在运行的后台进程发送 `process/interrupt`（SIGINT，best-effort），随后再 fallback `process/kill`（避免直接硬杀导致环境残留）。
- `pm-app-server turn/interrupt`：当 turn 被中断时，`TurnCompleted` 会携带 `reason`（与 `TurnInterruptRequested` 一致），便于 resume 拼合历史与审计。
- `pm-app-server process/start`：默认 cwd 改为 thread 的 `cwd`，并对 `cwd` 做 root + symlink 边界校验（见 `pm-core::sandbox`）。
- 明确 v0.2.0 的“运行中可观测性”：中间态 artifacts 必须流式落盘；任意后台进程/多子 agent 进程必须可随时 inspect/attach/kill（文档层先固化要求）。
- 细化 v0.2.0 “不会丢”的事件流语义：订阅端 `since_seq` 重放、允许重复（at-least-once）+ `seq` 去重；补齐 approval 记忆范围、artifact metadata/分片、通知节流与 process registry 最小字段（见 `docs/v0.2.0_parity.md` / `docs/rts_workflow.md`）。
- `pm-protocol`：`TurnCompleted` 事件增加可选 `reason` 字段（便于 resume 修复与审计）。
- `pm-protocol`：新增 `ProcessId` 与 `ProcessStarted/ProcessInterruptRequested/ProcessKillRequested/ProcessExited` 事件，作为 process registry 的可回放真相来源。
- `pm-protocol`：新增 `ToolId` 与 `ToolStarted/ToolCompleted` 事件，作为 tool runtime 的可审计边界。
- `pm-protocol`：新增 `ApprovalId`、`ApprovalRequested/ApprovalDecided` 与 `ThreadConfigUpdated(approval_policy)`，为 approvals 做事件化与回放打底。
- `pm-protocol`：新增 `AssistantMessage` 事件，用于把模型输出落盘并支撑 resume 拼合对话上下文。
- workspace `tokio` 特性启用 `io-std`（支持 app-server 的 stdio 读写）。

### Fixed
- `pm-app-server`/`pm`/`code-pm`/`pm-core::orchestrator`：拆分超大 Rust 源文件（保持行为不变），避免单文件超过 1000 行，降低 review/IDE 压力。
- `pm-app-server`：进一步拆分接近上限的模块（`agent/tools`、`process_control`），为后续扩展 tools/hooks 留出空间并保持单文件 < 1000 行。
- `pm-app-server`：拆分 JSON-RPC router（`main/app.rs`）为按域 handler 的小文件（`main/app/*.rs`），避免入口路由继续膨胀。
- `pm-app-server`：拆分 `thread_manage.rs` 为 `thread_manage/*.rs`（保留 include! 同模块作用域），降低单文件体积与后续变更冲突概率。
- `pm-app-server`：拆分 `process_stream.rs` 为 `process_stream/*.rs`，隔离 inspect/tail/follow 与 rotate/scan helpers，避免日志与工具逻辑继续缠在一起。
- `code-pm`：拆分 `main/tasks.rs`（测试与实现拆开），避免接近 1000 行的单文件继续膨胀。
- `pm-core::modes`：clippy cleanups（`needless_question_mark`）。
- `pm-eventlog`：`read_events_since` 忽略并发写入时可能出现的尾部半行（避免 reader 在 writer 追加期间误报 parse error）。
- `pm-app-server file/*`：失败路径也会写入 `ToolCompleted`（避免工具卡在 “started but never finished”）。
- `pm-app-server approvals`：当 mode/execpolicy 判定为 `prompt` 时，现在统一走 approvals gate：必落盘 `ApprovalRequested`；`approval_policy=auto_approve` 会追加 `ApprovalDecided(Approved, reason="auto-approved by policy")` 并继续执行；`approval_policy=manual` 返回 `needs_approval` 等待 `approval/decide` 后复跑同一 tool（覆盖 `file/*`、`fs/mkdir`、`process/start`）。
- `pm-app-server process/kill|interrupt`：现在也会执行 mode gate（`process.kill` + per-tool override）并在 `prompt` 下走 approvals；`pm` CLI 会在 `needs_approval/denied` 时给出可复制的处理提示。
- `pm-app-server-protocol`：`file/read`/`file/glob`/`file/grep` 追加可选 `approval_id`，用于 `needs_approval` 后的重试调用。
- `pm-app-server-protocol`：补齐 `process/interrupt` 方法与参数，并为 `process/kill` 追加可选 `turn_id/approval_id`（与其它 tools 对齐）。
- `pm-app-server process/inspect|tail|follow`：现在会执行 mode gate（`process.inspect` + per-tool override），并在 `prompt` 下走 approvals；`pm-app-server-protocol` 为这些方法追加可选 `turn_id/approval_id`，`pm` CLI 在 `tail/follow` 下会正确提示 `needs_approval/denied`。
- `pm-app-server artifact/write|list|read|delete`：现在也会执行 mode gate（`artifact` + per-tool override），并在 `prompt` 下走 approvals；`pm-app-server-protocol` 为这些方法追加可选 `turn_id/approval_id`，`pm` CLI 在 `artifact list/read/delete` 下会正确提示 `needs_approval/denied`。
- `pm` CLI：当 `process/*`/`artifact/*` 返回 `needs_approval` 时，现在可以通过 `--approval-id` 复跑同一命令（避免手动审批后陷入无限“再次请求 approval”）。
- `pm-app-server process logs`：stdout/stderr rotate 文件命名从 `*.part-0001.log` 改为 `*.segment-0001.log`（仍兼容读取 legacy `*.part-*.log`），避免产生大量 “part” 文件名。
- `pm-core::redaction`：修正 token 脱敏正则（Bearer/Google key），避免漏打码。
- `pm-core::sandbox`/`pm-app-server`：`sandbox_policy=danger_full_access` 现在会使用 unrestricted 路径解析（允许绝对路径与系统 symlink，如 macOS `/tmp`），不再误报 “escapes root”。
- Rust workspace：修复 `cargo clippy -- -D warnings` 下的告警（`pm-jsonrpc` 提取 pending type alias、`pm-openai` 使用 `std::io::Error::other`、`pm-protocol` 的 id newtype 实现 `Default`、`pm-eventlog` lockfile 显式 `truncate(false)`、以及 `pm-app-server` 若干 clippy cleanups）。
- `pm ask`/`pm exec`：只会处理当前 turn 触发的 `ApprovalRequested`（避免误处理历史遗留 approval）。
- `githooks/pre-commit`：默认禁止修改已发布版本的 changelog（仅允许改 `[Unreleased]`；发布时可设置 `CODE_PM_ALLOW_CHANGELOG_RELEASE_EDIT=1`）。
- `thread/list_meta`：派生 `attention_state` 时现在会考虑 pending approvals 与 running processes（`pm inbox --watch --bell` 能正确提示 `need_approval`）。
- `thread/list_meta`/`thread/attention`：后台进程以非零 exit code 退出时会派生 `attention_state=failed`（失败优先于 `running`），并在新 turn 开始时清空历史失败集合（避免一次失败导致 thread 永久处于 `failed`）；`pm watch --bell` 也会在 `ProcessExited` 失败时触发提醒。
- `pm-app-server approvals`：当同类操作被 `remember=true` 记住为 `deny` 时，`file/write|patch|edit|delete`、`fs/mkdir`、`process/start` 现在会返回结构化 `denied` 结果并写入 `ToolStatus=Denied`（不再走内部 error 路径）。
- `pm-core threads resume`：现在会修复 “ToolStarted 没有对应 ToolCompleted” 的中间态，自动补写 `ToolStatus=Cancelled`（避免崩溃/中断后留下悬空 tool）。

### Security
- `pm-core::threads`：落盘事件前自动脱敏（Turn input/argv/approval params/tool results 等），避免 secrets 进入 event log；`pm-app-server process/tail`/`process/follow` 返回内容也会脱敏展示。
- `pm-app-server`：`process/start` 默认从子进程环境中移除常见 provider key（`OPENAI_API_KEY` 等），降低“任意命令读取/回显密钥”的泄露面。
- `pm-app-server`：`file/*` 默认硬拒绝 `.env` 的读取与写入（write/patch/edit/delete），并在 `file/glob`/`file/grep` 扫描时跳过 `.codepm_data/{tmp,threads,data,repos,locks,logs}/`。

## [0.1.1] - 2026-01-20

### Added
- Hook 执行现在会把 stdout/stderr 写入 session artifacts：`/tmp/<repo>_<session_id>/logs/hook.{stdout,stderr}.log`，便于排查失败原因。
- 新增端到端 Runbook：`docs/workflow.md`（repo 注入/run/serve/api/hooks/session 回溯）。
- `--pm-root` / `CODE_PM_ROOT`：允许覆盖 `.code_pm` 数据目录位置（相对路径按 repo root 解析）。
- 兼容提示：当默认使用 `.code_pm` 且检测到旧目录 `.codex_pm` 存在（但 `.code_pm` 尚未创建）时，CLI 会输出 warning，提示手动迁移或通过 `--pm-root .codex_pm` / `CODE_PM_ROOT=.codex_pm` 复用旧数据。
- `code-pm repo list --json/--verbose`：支持 JSON 输出与 verbose 输出（包含 bare/lock 路径），便于脚本/调试。
- `code-pm init --json` / `code-pm repo inject --json`：支持 JSON 输出，便于脚本/集成。
- `code-pm run --repo <name>.git` / `code-pm repo inject --name <name>.git`：repo 名参数支持可选 `.git` 后缀，避免生成重复 `.git.git` 目录。
- `code-pm session list/show`：从本地 `.code_pm/data/` 查询 session（默认优先输出 `result`；`--all` 输出 session/tasks/prs/merge/result）。
- `code-pm session list [--limit N] [--json]`：按 session id 顺序列出 sessions；`--json` 输出 JSON 数组。
- `code-pm session list --verbose [--limit N] [--json]`：按 `created_at`（RFC3339）倒序输出 session 元信息（不含 prompt）；`--json` 输出 JSON 数组。
- `code-pm run --json`：以 pretty JSON 输出 `RunResult`（便于脚本/集成）。
- `code-pm run --strict`：当存在 task 失败或 merge error 时返回非零退出码。
- `code-pm run --no-merge`：跳过合并步骤，只生成/推送 PR 分支并写入 session 数据（不会修改 base）。
- `code-pm run --cargo-test`：对 Rust repo 在提交前额外执行 `cargo test --workspace --all-targets`（输出写入 task artifacts）。
- `code-pm run --stream-events-json`：以 JSON Lines（NDJSON）格式输出 `RunEvent` 到 stderr（每行包含 `type` 字段），便于实时消费事件流。
- `code-pm run --hook-url <url>`：完成后向 webhook `POST` JSON（`session_id/repo/pr_name/base_branch/pm_root/session_dir/tmp_dir/result_json/merged/merge_error`）。
- `pm-http GET /api/v0/sessions`：新增 `?verbose=true`（返回 `SessionMeta[]`）与 `?limit=N`（截断结果）；默认仍返回 `SessionId[]`。
- `pm-http GET /api/v0/repos`：新增 `?verbose=true`（返回 `[{name,bare_path,lock_path}]`）；默认仍返回 `RepositoryName[]`。
- `pm-http GET /api/v0/sessions/:id/meta`：返回 `SessionMeta`（不含 prompt）。

### Changed
- `code-pm run`：在 git repo 内运行且未显式提供 `--repo/--repo-src` 时，默认以当前 `repo_root` 作为 `--repo-src` 注入并运行（repo 名默认为目录名（去掉可选 `.git` 后缀后）sanitize）。
- `pm-http`：query flag 支持无值形式（`?verbose` / `?all` 等价于 `=true`）。
- `pm-http GET /api/v0/sessions/:id`：默认返回 `result`；`?all=true` 返回 session/tasks/prs/merge/result bundle。
- `PrName/TaskId` 的 sanitize 规则收紧为 git-ref 安全：不再保留 `.`（例如 `.hidden` → `hidden`、`a..b` → `a-b`），避免生成无效分支名导致运行失败。
- `RepositoryName/PrName/TaskId` 增加长度上限（`64`），避免生成超长目录名/branch ref 导致运行失败。
- Orchestrator：改进并发调度（以 worker-pool 方式限流，不再一次性 spawn 所有 tasks；不会再因为并发限流导致 `TaskFinished` 事件延迟），并在 `--max-concurrency 1` 场景下也会把 task 的 panic/cancel 转为 `Failed` PR（避免直接崩溃整个 session）。
- session 列表查询改为直接枚举 `.code_pm/data/sessions/` 目录（更快，且只返回合法 UUID）。
- `Session/SessionMeta.created_at` 的 JSON 表达改为 RFC3339 字符串（读仍兼容旧 tuple/unix timestamp）。
- session 元信息新增独立存储 `sessions/<id>/meta.json`（`list_session_meta` 优先读取，避免为列表读入大 prompt）。

### Fixed
- `code-pm run --max-concurrency`：现在会校验为 `>= 1`（拒绝 `0`，避免静默回退到 `1`）。
- `code-pm run`：隐式 `--repo-src` 模式现在会严格要求处于真实 git worktree（基于 `git rev-parse` 判断），避免仅凭 `.git` 路径误判导致后续 clone 失败。
- `code-pm` CLI：`--repo/--repo-src/--pr-name/--base` 以及 `repo inject` 的 `source/--name` 现在会拒绝空值（包括仅空白字符），避免静默回退到默认 sanitize 值。
- `code-pm run`：现在会拒绝空/纯空白的 prompt（`--prompt` 或 `--prompt-file`），避免生成无意义 session。
- `code-pm run --auto-tasks`：现在会拒绝与 `--task/--tasks-file` 同时使用（避免 task 来源冲突）。
- `pm-http git`：405 响应包含 `Allow: GET, POST`，便于客户端/调试工具正确处理。
- `pm-git`：repo lock 文件缺失父目录时会自动创建（避免 `.code_pm/locks` 被手动删除后运行失败）。
- `pm-git`：默认 repo 名推导更健壮（支持 `git@host:repo`/Windows 路径等输入）。
- `pm-git`：identity 配置步骤现在会严格校验每一步的 `ok`（避免静默忽略 `git config --get` 的异常失败）。
- `code-pm run --task/--tasks-file`：显式空 task id 现在会被拒绝（避免意外回退到 `task` 造成重复/混乱）。
- `CODE_PM_TMP_ROOT` 为空（例如 `export CODE_PM_TMP_ROOT=`）时将被忽略，避免 session artifacts 意外落到当前工作目录。
- `code-pm repo inject` 与 `code-pm run --repo-src` 现在支持使用相对路径引用本地仓库（不会再相对 `.code_pm/repos` 解析导致 clone 失败）。
- `code-pm repo inject` 复用已存在的注入仓库时会更新 `origin` 到本次传入的 source，避免同名重新注入仍从旧源 fetch。
- `code-pm repo inject` 复用已存在的 bare repo 时会以 mirror refspec 拉取到 `refs/*`（而不是只更新 `refs/remotes/origin/*`），避免后续 clone 看不到 `refs/heads/*`。
- storage 读取损坏 JSON 时增加错误上下文（包含具体文件路径），便于定位问题数据。
- storage 写入 JSON 失败时会尽力清理临时文件（`*.json.tmp.*`），避免脏文件堆积。
- hook 执行失败的错误现在会包含 session id，便于定位失败运行与对应 artifacts。
- `pm-http` 构造 fallback 响应时现在会正确设置 HTTP status（避免极端情况下误返回 200）。
- `code-pm run --stream-events-json` 在序列化异常时也会输出 JSON 行，避免污染 NDJSON 流。
- `code-pm run --stream-events-json` 在 consumer 落后导致事件被丢弃时会输出 error JSON 行（`event_lagged`），避免静默丢事件。

### Security
- `pm-http`：支持 `CODE_PM_HTTP_MAX_BODY_BYTES`（默认 1GiB）限制 git smart-http 请求体大小；超限返回 413。
- `pm-http git`：`CODE_PM_HTTP_MAX_BODY_BYTES` 现在会对实际请求体字节数强制生效（不再仅信任 `Content-Length` header），避免超大 body 绕过限制。
- `pm-http serve`：强制 loopback-only（拒绝绑定非 `127.0.0.1`/`::1` 地址），避免无鉴权服务被意外暴露到公网。
- `pm-http git`：拒绝除 GET/POST 之外的 HTTP method（返回 405），减少非预期攻击面。
- `pm-git`/`pm-http`：repo 列表与访问现在只接受目录形式的 bare repo（忽略同名文件），避免误识别与更早失败。
- `pm-git`/`pm-http`：repo 列表与访问现在会校验 bare repo 的最小结构（`HEAD`/`config`/`objects/`），避免空目录/垃圾目录被当成仓库。
- `pm-http` 的 session API（`/api/v0/sessions/:id/*`）现在只接受 UUID 格式的 session id，避免非法路径段触发 storage key 校验错误导致 500。

## [0.1.0] - 2026-01-20

### Added
- 初始 CodePM Rust workspace（`code-pm` CLI + `pm-core`/`pm-git`/`pm-http`），落地 Phase 1 骨架：repo 注入、/tmp session/task 目录、任务 clone/commit/push、本地顺序合并。
- `code-pm init`：初始化 `.code_pm/` 数据目录布局。
- Repo 管理：`code-pm repo inject/list`，注入本地路径或远端 URL 到 `.code_pm/repos/<name>.git` bare repo。
- Run 编排：`code-pm run` 创建 session，产出 `/tmp/<repo>_<session_id>/...` artifacts 与 `.code_pm/data/sessions/<id>/` JSON 数据（session/tasks/prs/merge/result）。
- 并发 task：每 task 独立 clone 到 `/tmp/.../tasks/<task_id>/repo`，可用 `--max-concurrency` 并行；事件流 `--stream-events`。
- 任务拆分：`--task/--tasks-file` 覆盖任务列表；`--auto-tasks` 从 prompt Markdown 列表规则化拆分。
- Git coder 流水线：clone →（可选）`git apply` →（Rust repo）`cargo fmt`/`cargo check`（`CARGO_TARGET_DIR` 指向 artifacts）→ commit → push 分支 `ai/<pr_name>/<session_id>/<task_id>`。
- Merge：顺序合并 Ready PR 分支回 base（默认 `main`），并保留每一步日志到 `merge/artifacts/*.log`。
- Smart HTTP（增强）：`code-pm serve`（loopback-only）提供 `git clone/push` over HTTP（`git http-backend`）。
- Hook：完成后可执行命令 hook，并通过 `CODE_PM_*` 环境变量传递 session 上下文；临时目录根可用 `CODE_PM_TMP_ROOT` 覆盖。
