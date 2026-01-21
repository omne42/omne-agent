# Changelog

本项目的所有重要变更都会记录在这个文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added
- 新增 Agent-first 重新开发流程文档：`docs/development_process.md`。
- 新增 vNext 目标态“RTS 风格使用流程”文档：`docs/rts_workflow.md`。
- 新增 Agent GUI 爆发期产品调研：`docs/research/onecode.md`、`docs/research/superset.md`、`docs/research/aion-ui.md`。
- 新增 `v0.2.0` 功能对齐与 TODO 汇总：`docs/v0.2.0_parity.md`。
- 新增 `pm-jsonrpc`：最小 JSON-RPC over stdio client，用于驱动 `pm-app-server`。
- 新增 `pm-protocol`/`pm-eventlog`：为 v0.2.0 落地 Thread/Turn 事件类型与 append-only JSONL event log（thread_id 一致性校验、`seq` 连续、`since_seq` 断点续读、尾部半行自动截断，并提供 `ThreadState` 纯事件派生）。
- 新增 `pm-execpolicy`：对齐 Codex `prefix_rule` 子集的执行策略引擎（Starlark 语法 + `match/not_match` 例子校验），并提供 `pm-execpolicy check --rules ... <cmd...>` 输出匹配结果 JSON。
- 新增 `pm-openai`：最小 OpenAI Responses API 客户端与类型（用于 v0.2.0 的 Responses-first agent loop）。
- 新增 `pm-app-server`：最小 JSON-RPC over stdio 控制面（`initialize` + `thread/*` + `turn/*`），用于验证 v0.2.0 的 thread/turn/interrupt 与落盘回放。
- 新增 `pm` CLI：作为 `pm-app-server` 的人类可用客户端，支持 `ask/watch --bell`、`thread/*`、`approval/*`、`process/*`（只读查看 + kill），并在 `ask` 中支持 Ctrl-C 触发 `turn/interrupt`。
- `pm` CLI 新增 `inbox`：跨 thread 的 RTS 收件箱视图（可 `--watch` + `--bell` 去重提醒），用于快速发现 `need_approval/failed/running`。
- `pm` CLI 补齐更多控制面命令：`pm thread fork/archive/unarchive/delete/clear-artifacts/disk-*` 与 `pm artifact list/read/delete`，便于手动清理与审计。
- `pm` CLI 新增可解释性与状态查询：`pm thread state`、`pm thread config-explain`、`pm thread loaded`。
- 新增 `TurnStatus::Stuck`：当 agent 超预算/超时（turn 时长、tool call、OpenAI 请求超时等）时显式标记为 `stuck`，并在 `thread/attention` 与 `pm * --bell` 中可见。
- `pm-app-server` 在退出前会尽力终止仍在运行的子进程（避免 CLI 关闭后留下孤儿进程）。
- `pm-app-server` 新增 `initialized`（握手确认）与 `thread/loaded`（列出当前已加载 threads）。
- `pm-app-server` 新增 `thread/list_meta`：批量返回 threads 的派生状态（支持 `include_archived`），减少 UI/CLI 人肉遍历与重复读取 event log。
- `pm-app-server` 新增 Responses-first agent loop：`turn/start` 会调用 OpenAI Responses API 执行 tool calling（阻塞等待 approval 决策并复跑同一 tool call），并将 assistant 输出落盘为 `AssistantMessage` 事件以支持 resume。
- `pm-app-server` agent loop：会读取 `<thread cwd>/AGENTS.md` 并追加到 instructions（写入前自动脱敏），让项目级规范从第一天就能约束 agent。
- 新增 `pm-core::threads`：`ThreadStore` + `ThreadHandle`，基于 JSONL event log 实现 thread 创建/列举/resume（resume 会修复未完成 turn/进程并落盘）。
- 新增 `pm-core::sandbox`：rooted path 解析与边界校验（拒绝 `..` 穿越与 symlink 逃逸）。
- `pm-app-server` 新增 `thread/state`：返回 thread 派生状态（active turn、`last_seq`、interrupt 标记）。
- `pm-app-server` 新增 `thread/fork`：复制 thread 的对话/审批/配置事件到新 thread（跳过 tool/process 事件与 thread 专属 artifact 路径，并自动跳过进行中的 active turn），用于多子 agent 并行。
- `pm-app-server` 新增 `thread/archive`/`thread/unarchive`：归档 thread（默认拒绝含 active turn/running process；`force=true` 可中断 turn 并终止进程）。
- `pm-app-server thread/events`：支持 `max_events` 分页，并返回 `has_more`/`thread_last_seq` 便于订阅端处理 lag 与续读。
- `pm-app-server thread/subscribe`：长轮询读取 thread events（`wait_ms` 超时），用于实现“不断线不丢”的订阅式消费（`since_seq` + `seq` 去重）。
- `pm-app-server` 新增 thread 清理 API：`thread/delete(force?)` 与 `thread/clear_artifacts(force?)`，用于一键清除 history 与中间态产物。
- `pm-app-server` 新增 approvals 控制面：`thread/configure(approval_policy,sandbox_policy?)`、`approval/list`、`approval/decide`。
- `pm-app-server` 新增 `thread/config/explain`：返回最小 config layer stack（当前覆盖 `approval_policy`/`sandbox_policy`/`model`/`openai_base_url`），用于回答“为什么生效的是这个值”。
- `pm-protocol`/`pm-eventlog` 新增 thread-level 模型配置：`ThreadConfigUpdated.model/openai_base_url` + `ThreadState.model/openai_base_url`。
- `pm-app-server` 新增 `thread/attention`：派生 RTS “收件箱”视图（pending approvals + running processes），减少 UI/CLI 人肉扫描 event log。
- `pm-app-server` 新增 `thread/disk_usage` 与 `thread/disk_report`：返回 thread 目录磁盘占用，并可生成 `disk_report` markdown artifact 便于清理。
- `pm-app-server` 新增 `process/*`：`process/start`（落盘 stdout/stderr）、`process/list`、`process/inspect`（元信息 + tail）、`process/tail`（只读查看）、`process/follow`（增量查看）、`process/kill`（终止后台进程）。
- `pm-app-server` 新增 `file/*`：`file/read`、`file/glob`、`file/grep`、`file/write`、`file/patch`、`file/edit`、`file/delete`（带 rooted path 校验，并记录 `ToolStarted/ToolCompleted` 事件）。
- `pm-app-server` 新增 `fs/*`：`fs/mkdir`（带 rooted path 校验，并记录 `ToolStarted/ToolCompleted` 事件）。
- `pm-app-server` 新增 `artifact/*`：`artifact/write`（`.md + .metadata.json` 落盘并自动脱敏）、`artifact/list`、`artifact/read`、`artifact/delete`。
- `pm-app-server` agent loop tool 覆盖：补齐 `file/edit`、`file/delete`、`process/tail`、`process/follow`、`artifact/list`、`artifact/read`、`artifact/delete`。
- `pm-app-server agent loop`：新增 `agent_spawn`（fork + 启动子 agent turn）与 `thread_state`/`thread_events`（fan-in 读状态与事件）。

### Changed
- 重写 `docs/implementation_plan.md`：以 Agent CLI（tool/sandbox/approvals + 事件流）为核心基建，Git 降级为交付适配层，并明确 RTS 控制面最小能力集。
- 更新 `docs/development_process.md`：补齐 RTS 风格交互要求（attention/inbox、pause/interrupt/step），并把 workspace hooks（setup/run/archive）与 artifacts/preview 明确进里程碑。
- `docs/workflow.md` 标注为 `v0.1.1` legacy，`docs/start.md` 增加 vNext 文档导航。
- 更新 `docs/research/README.md`：补齐新增调研条目并调整落地方向表述。
- `githooks/pre-commit`：强制每次提交同时包含 `CHANGELOG.md` 与实际变更（禁止 changelog-only / non-changelog commit）。
- `pm-app-server agent loop`：增加最小长任务预算（`max_steps`/`max_tool_calls`/`max_turn_seconds` + 单次 OpenAI 请求超时）；超限会使 turn 失败并写入失败原因，避免无限循环烧钱/卡死。
- v0.2.0 方向明确：基建不依赖 Docker（git/workspace 实现不以 Docker 为前提）；实现文档中移除/替换相关表述。
- `pm-eventlog ThreadState`：记录 thread 的 `cwd`；`pm-app-server thread/state` 返回 `cwd` 便于后续 sandbox/root 约束。
- `pm-eventlog ThreadState`：增加 `approval_policy`（默认 auto-approve）；`pm-app-server thread/state` 返回当前策略。
- `pm-eventlog ThreadState`：增加 `sandbox_policy`（默认 `workspace_write`）；`pm-app-server thread/state`/`thread/attention` 返回当前策略。
- `pm-eventlog ThreadState`：增加 `model`/`openai_base_url`（默认跟随 env/default）；`pm-app-server thread/state`/`thread/attention`/`thread/config/explain` 返回当前值。
- `pm-eventlog ThreadState`：增加 `archived/archived_at/archived_reason` 与 `last_turn_id/last_turn_status/last_turn_reason`；`pm-app-server thread/state`/`thread/attention` 返回归档与 last turn 信息，并派生 `attention_state`（含 `archived`）。
- `pm-app-server thread/configure`：`approval_policy` 现在可选（省略时沿用当前），并支持设置 `model`/`openai_base_url`（thread override）。
- `pm-app-server`：当 `approval_policy=manual` 时，`file/write`/`file/delete`/`fs/mkdir`/`process/start` 会返回 `needs_approval` 并写入 `ApprovalRequested`；提供 `approval_id` 且已 `approval/decide` 后才会执行。
- `pm-app-server`：当 `sandbox_policy=read_only` 时，`file/write`/`file/patch`/`file/edit`/`file/delete`/`fs/mkdir`/`process/start` 会直接拒绝（ToolStatus=Denied）。
- `pm-app-server approvals`：`approval/decide` 支持 `remember=true`（session 内记忆 approve/deny），同类操作无需重复弹审批；拒绝也会被记住并直接拦截。
- `pm-app-server process/start`：引入 `pm-execpolicy` gate（`prefix_rule`）：`forbidden` 直接拒绝并写入 `ToolStatus::Denied`；`manual` 策略下仅当 `prompt`/未匹配时才要求 approval（用 allowlist 降低骚扰）。
- `pm-app-server turn/interrupt`：会尝试终止同一 turn 下仍在运行的后台进程（best-effort）。
- `pm-app-server turn/interrupt`：当 turn 被中断时，`TurnCompleted` 会携带 `reason`（与 `TurnInterruptRequested` 一致），便于 resume 拼合历史与审计。
- `pm-app-server process/start`：默认 cwd 改为 thread 的 `cwd`，并对 `cwd` 做 root + symlink 边界校验（见 `pm-core::sandbox`）。
- 明确 v0.2.0 的“运行中可观测性”：中间态 artifacts 必须流式落盘；任意后台进程/多子 agent 进程必须可随时 inspect/attach/kill（文档层先固化要求）。
- 细化 v0.2.0 “不会丢”的事件流语义：订阅端 `since_seq` 重放、允许重复（at-least-once）+ `seq` 去重；补齐 approval 记忆范围、artifact metadata/分片、通知节流与 process registry 最小字段（见 `docs/v0.2.0_parity.md` / `docs/rts_workflow.md`）。
- `pm-protocol`：`TurnCompleted` 事件增加可选 `reason` 字段（便于 resume 修复与审计）。
- `pm-protocol`：新增 `ProcessId` 与 `ProcessStarted/ProcessKillRequested/ProcessExited` 事件，作为 process registry 的可回放真相来源。
- `pm-protocol`：新增 `ToolId` 与 `ToolStarted/ToolCompleted` 事件，作为 tool runtime 的可审计边界。
- `pm-protocol`：新增 `ApprovalId`、`ApprovalRequested/ApprovalDecided` 与 `ThreadConfigUpdated(approval_policy)`，为 approvals 做事件化与回放打底。
- `pm-protocol`：新增 `AssistantMessage` 事件，用于把模型输出落盘并支撑 resume 拼合对话上下文。
- workspace `tokio` 特性启用 `io-std`（支持 app-server 的 stdio 读写）。

### Fixed
- `pm-eventlog`：`read_events_since` 忽略并发写入时可能出现的尾部半行（避免 reader 在 writer 追加期间误报 parse error）。
- `pm-app-server file/*`：失败路径也会写入 `ToolCompleted`（避免工具卡在 “started but never finished”）。
- `pm-core::redaction`：修正 token 脱敏正则（Bearer/Google key），避免漏打码。
- `pm-core::sandbox`/`pm-app-server`：`sandbox_policy=danger_full_access` 现在会使用 unrestricted 路径解析（允许绝对路径与系统 symlink，如 macOS `/tmp`），不再误报 “escapes root”。

### Security
- `pm-core::threads`：落盘事件前自动脱敏（Turn input/argv/approval params/tool results 等），避免 secrets 进入 event log；`pm-app-server process/tail`/`process/follow` 返回内容也会脱敏展示。

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
- Rust workspace：修复 `cargo clippy -- -D warnings` 下的告警（`pm-jsonrpc` 忽略无 `id` 的 stdout 行、`pm-protocol` 的 id newtype 实现 `Default`、`pm-eventlog` lockfile 显式 `truncate(false)`、以及 `pm-app-server` 若干 clippy cleanups）。
- `thread/list_meta`：派生 `attention_state` 时现在会考虑 pending approvals 与 running processes（`pm inbox --watch --bell` 能正确提示 `need_approval`）。
- `code-pm run --max-concurrency`：现在会校验为 `>= 1`（拒绝 `0`，避免静默回退到 `1`）。
- `pm-app-server approvals`：当同类操作被 `remember=true` 记住为 `deny` 时，`file/write|patch|edit|delete`、`fs/mkdir`、`process/start` 现在会返回结构化 `denied` 结果并写入 `ToolStatus=Denied`（不再走内部 error 路径）。
- `pm-core threads resume`：现在会修复 “ToolStarted 没有对应 ToolCompleted” 的中间态，自动补写 `ToolStatus=Cancelled`（避免崩溃/中断后留下悬空 tool）。
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
