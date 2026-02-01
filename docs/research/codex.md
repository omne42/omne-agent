# Codex（example/codex）能力与设计分析（以 Rust/Responses API 为中心）

> Snapshot: `example/codex` @ `b66018a`
>
> 结论先行：Codex 的 Rust 实现（`example/codex/codex-rs`）已经具备我们要做“并发 AI task + /tmp 临时目录隔离 + 工具执行 + 审批/沙箱 + 可被 UI 驱动”的大部分底座能力。`omne-agent` 最合理的路线是：**基于 codex-rs 魔改/复用**，先把我们缺失的“多任务编排 + Git PR 流水线 + PR 合并”补上；并且**第一阶段仅启用 Responses API（wire_api = responses）**。

---

## 1. 仓库结构与“可复用资产”

`example/codex` 是一个大仓，主要可复用的资产集中在：

- `example/codex/codex-rs/`：Rust workspace（核心逻辑、TUI、exec、app-server、sandbox、mcp、responses api proxy…）
- `example/codex/codex-cli/`：Node 打包与发布脚本（用于分发 Rust 二进制、安装 native deps）
- `example/codex/sdk/`：SDK（可作为未来外部集成参考）

对 `omne-agent`（Rust-only）而言，优先研究 `codex-rs`。

---

## 2. Codex（Rust）“所有能力”盘点（按能力域）

> 下面以“系统能力”来盘点，而不是按 crate 罗列，避免遗漏。

### 2.1 多界面形态：TUI / exec / app-server

来自 `example/codex/codex-rs/README.md`：

- `codex`（multitool CLI）：聚合多个子命令/子 UI。
- `codex-tui` / `codex-tui2`：Ratatui 全屏交互式 UI。
- `codex exec`：非交互/自动化模式（适合 CI 或脚本化运行）。
- `codex app-server`：**面向 IDE/富客户端的后端**（JSON-RPC over stdio，事件流式推送）。

这些形态对我们非常重要：

- 并发 worker 可以用 `exec` 模式（最轻）。
- 如果需要高可观测与 UI/daemon 化，使用 `app-server` 协议最稳。

### 2.2 会话模型：Thread / Turn / Item（可追溯）

`example/codex/codex-rs/app-server/README.md` 定义了核心 primitives：

- Thread：一个对话
- Turn：一轮交互（用户输入 → agent 产出）
- Item：turn 内的细粒度事件/产物（message、reasoning、shell、file edit、diff、approval request…）

关键价值：

- **所有 side effects 都被 item 化与持久化**，便于审计、重放、UI 渲染。
- `thread/resume` / `thread/fork` 等操作使“并发分支任务”天然可表达。

补充（实现侧）：

- `example/codex/codex-rs/core/src/thread_manager.rs` 的 `ThreadManager` 负责创建并维护 threads（内存 map + broadcast channel），并持有 `ModelsManager`/`SkillsManager` 等全局资源。这类“统一生命周期管理器”非常接近我们想要的 `AgentManager/Orchestrator`。

### 2.3 工具执行与权限：Sandbox + Approvals + ExecPolicy（安全底座）

Codex 的安全与执行底座很厚，相关 crate/文档：

- sandbox policy（跨平台）：`read-only` / `workspace-write` / `danger-full-access`
- Linux：`codex-linux-sandbox`（Landlock 等）
- macOS：Seatbelt（`/usr/bin/sandbox-exec`）+ 对 `.git` 只读保护（见 `codex-core/README.md`）
- `codex-execpolicy`：规则引擎（prefix_rule 语言），可输出匹配结果 JSON（`example/codex/codex-rs/execpolicy/README.md`）
- `codex-exec-server`：**patched Bash + execve wrapper**（把 Bash 内部所有 execve 调用“上报”到 MCP server 决策，支持 Run/Escalate/Deny）——这是 Codex CLI “可审批执行”的关键工程实现（`example/codex/codex-rs/exec-server/README.md`）
- `codex-process-hardening`：pre-main hardening（禁 core dump、禁 ptrace、清理危险 env vars）（`example/codex/codex-rs/process-hardening/README.md`）

对 `omne-agent` 的意义：

- 我们要做自动化的 git/format/check/build，风险面比单纯改文件更大。
- Codex 的安全底座可以直接复用（尤其 approvals + execpolicy），避免我们自研“命令白名单/审批系统”。

### 2.4 MCP（工具生态）

`codex-rs` 同时支持：

- MCP client：启动时连接 MCP servers（让 Codex 具备额外工具能力）
- MCP server（实验）：`codex mcp-server` 让“其它 agent”把 Codex 当工具用（`example/codex/codex-rs/README.md`）
- 独立的 `mcp-types`：对 MCP schema 做类型化建模（类似 lsp-types）（`example/codex/codex-rs/mcp-types/README.md`）

对 `omne-agent` 的意义：

- 我们的 builder/reviewer/merger 后续可能需要外部工具（CI、issue tracker、code search、browser automation…）。
- 用 MCP 做生态扩展是可行路线，但 MVP 不必依赖。

### 2.5 模型与 Provider：wire_api 选择（Responses 是主线）

`example/codex/codex-rs/core/src/model_provider_info.rs`（节选）说明：

- `wire_api` 区分：
  - `responses`（OpenAI `/v1/responses`）
  - `responses_websocket`（实验）
  - `chat`（`/v1/chat/completions`，默认但已明确 deprecated）
- `CHAT_WIRE_API_DEPRECATION_SUMMARY` 表明未来会移除 chat wire API。

对 `omne-agent` 的明确约束：

- **第一阶段只支持 Responses**：我们应在配置层与代码路径上“默认且优先 Responses”，并尽量不要引入 chat 兼容逻辑到新模块里。

### 2.6 配置系统与可运维性（强烈相关：hook/策略/多环境）

`example/codex/codex-rs/core/src/config/mod.rs` 的 `Config` 展示了 Codex 的“配置即产品能力”：

- config layering：`config_layer_stack` 记录配置来源与合并过程（便于调试“为什么生效的是这个值”）。
- 模型相关：`model/review_model/model_context_window/model_auto_compact_token_limit`。
- 安全相关：`approval_policy`、`sandbox_policy`、`shell_environment_policy`。
- 指令来源：`user_instructions`（AGENTS.md）、`base_instructions`、`developer_instructions`、`compact_prompt`。
- **notify hook（与我们需求直接对齐）**：
  - `notify: Option<Vec<String>>`：每个 turn 完成后执行外部程序，并附加一个 JSON 参数（文件内注释给了完整示例）。

> 这说明 Codex 已经把 “完成时 hook 回调” 做成了第一等能力。`omne-agent` 不应重复造轮子，应该直接复用（或在其上扩展）这一套 hook 机制，补齐我们“session/task/pr 合并完成后回调”的语义即可。

---

## 3. Responses API：Codex 的实现细节（本期重点）

### 3.1 统一的 Prompt 抽象（Chat/Responses 共用）

`example/codex/codex-rs/codex-api/src/common.rs` 定义：

- `Prompt { instructions, input: Vec<ResponseItem>, tools: Vec<Value>, parallel_tool_calls, output_schema }`
- 这意味着上层业务（codex-core）不需要关心 endpoint 形状差异，只要提供：
  - “完整 system instructions”
  - “结构化对话历史/工具消息”（ResponseItem）
  - “工具定义 JSON”

### 3.2 Responses 请求体构造（含 reasoning/text controls）

同文件定义 `ResponsesApiRequest` 与 `TextControls`：

- `reasoning`：effort + summary（对应 GPT-5 reasoning 能力）
- `text`：verbosity + `text.format`（JSON schema 输出约束）
- `include`、`prompt_cache_key`、`parallel_tool_calls` 等都被显式建模

这对 `omne-agent` 的价值：

- 我们未来的 `Architect`/`Merger` 很可能需要结构化输出（DAG、合并计划、风险列表）。
- Codex 已内置 JSON schema 输出控制接口，可直接复用，而不必自己在 prompt 上“正则约束输出”。

### 3.3 SSE 事件解析：把 Responses stream 变成强类型事件流

`example/codex/codex-rs/codex-api/src/sse/responses.rs` 负责把 SSE 转换成 `ResponseEvent`：

- `response.output_text.delta` → `ResponseEvent::OutputTextDelta`
- `response.output_item.done` → `ResponseEvent::OutputItemDone(ResponseItem)`
- `response.reasoning_text.delta` / `response.reasoning_summary_text.delta`
- `response.completed` / `response.done` → `ResponseEvent::Completed { token_usage }`
- `response.failed` → 解析 error 并映射成 `ApiError`（包含 retryable、quota exceeded、context window exceeded 等）
- 在流启动阶段还会解析：
  - rate limit headers → `ResponseEvent::RateLimits`
  - `X-Models-Etag` → `ResponseEvent::ModelsEtag`
  - `x-codex-turn-state`（存入 OnceLock，用于 turn 状态追踪）

这说明 Codex 对 Responses 的“可观测性/可靠性”投入很大：

- 错误类型化，便于上层重试策略
- token usage/rate limits 作为第一等事件

### 3.4 “只允许 /v1/responses”的安全代理（responses-api-proxy）

`example/codex/codex-rs/responses-api-proxy/README.md` 描述了一个严格代理：

- 只转发 `POST /v1/responses` 到 `https://api.openai.com/v1/responses`
- API key 从 stdin 读取，避免出现在环境变量/命令行历史里
- 对 Bearer token 做 `mlock(2)` 与 `zeroize` 等硬化，减少泄露风险
- 支持 `--http-shutdown` 让非特权用户能关停代理

对 `omne-agent`：

- 我们的 worker 如果运行在“低权限用户”上下文（或者未来有远端执行/多租户），这套代理可以作为“安全传递 OpenAI key”的参考实现。

补充：该代理也支持 `--upstream-url` 指向 Azure responses endpoint（README 有示例），并要求 Bearer header 形式；这对企业场景下的 Azure OpenAI 很实用。

---

## 4. app-server：把 Codex 变成“可被编排的后端”

`example/codex/codex-rs/app-server/README.md` 极其关键（建议全文精读），核心要点：

- JSON-RPC 2.0（省略 jsonrpc header），JSONL over stdio。
- 初始化握手：`initialize` → `initialized`，未初始化前其它请求会被拒绝。
- 线程与 turn API：
  - `thread/start` / `thread/resume` / `thread/fork` / `thread/list` / `thread/archive`
  - `turn/start` / `turn/interrupt`
- turn/start 支持覆盖：
  - `cwd`
  - `approvalPolicy`
  - `sandboxPolicy`（含 writableRoots、networkAccess）
  - `model/effort/summary/outputSchema`
- 支持 skill invocation（把 skill 文件作为 input item 发送）
- 事件流：`item/*` notifications（agent message delta、tool started/completed、diff items 等）

> 对 `omne-agent`：如果我们要做“并发 worker + 状态订阅 + hook 回调”，app-server 协议几乎是现成的“编排协议”。我们可以选择：
>
> - 直接复用 app-server 作为 worker 的控制面（orchestrator 作为 client）
> - 或在 codex-core 上层嵌入，但保留同样的事件模型（Thread/Turn/Item）

补充（schema 生成很关键）：

- app-server 支持 `generate-ts` 与 `generate-json-schema`（README 提到），这意味着我们未来做 Web/桌面 UI 时，可以用“协议驱动”生成 types，减少前后端漂移风险。

---

## 5. Codex 中值得直接复刻的“工程巧思”

### 5.1 协议 types 与业务逻辑分离

`codex-protocol` 的 README 明确：协议 crate 应保持 minimal dependencies，不放“实质业务逻辑”。这使得：

- app-server、tui、exec 等多个前端共享同一套 types；
- 协议稳定可生成 TS/JSON schema（app-server 支持 generate-ts/generate-json-schema）。

### 5.2 安全是“系统性工程”，不是一个开关

从 exec-server/execpolicy/process-hardening 可以看到：

- 不是简单地 “prompt 用户确认”，而是把**命令执行的每一次 execve**都纳入 policy 决策。
- 通过 patched bash 把 shell 内部子进程也纳入约束。

这类实现非常适合我们的“自动化全生命周期流水线”，因为后期 builder/deployer 会涉及大量命令执行。

补充：`codex-process-hardening` 在 pre-main 阶段清理危险环境变量并禁用 core dump/ptrace，是“默认安全”的重要一环；我们若 fork 运行在 daemon 场景，也应该保留这一层。

### 5.3 Responses-first 的演进路线

代码与文档已经把 chat wire API 标记为 deprecated，并提供 `responses_websocket` 实验线路。说明：

- Codex 的未来重心已经明确在 Responses。
- 我们选择 Responses-only 会更贴近上游方向，减少未来合并成本。

---

## 6. `omne-agent` 应如何“基于 codex 魔改”（推荐路线）

> 这里给出两条路线：A 更像 fork + 新 crate；B 更像外部 orchestrator 驱动 `codex exec/app-server`。

### 路线 A：fork codex-rs，加一个 `omne-agent` crate（推荐长期）

- 优点：
  - 直接复用 `codex-core`、`codex-api`、`codex-protocol` 的内部接口
  - 事件/状态可以更细粒度接入（不用解析 JSON-RPC）
  - 更容易做“多线程并发 Thread/Turn 管理”
- 我们新增的核心：
  - repo injection（bare repo 托管）
  - /tmp session/task workspace 管理
  - 并发调度（JoinSet + Semaphore）
  - git fmt/check/commit/push/pr pipeline
  - merger agent（多 PR 合并与冲突修复）
  - 完成 hook（webhook/command）

### 路线 B：外部 orchestrator，驱动 `codex exec` 或 `codex app-server`（更快验证）

- 优点：
  - 不需要深入改 codex 内部，可先把流程跑通
  - 适合快速做 MVP
- 缺点：
  - 要么解析 app-server JSON-RPC，要么解析 exec 输出，集成成本更高
  - 对 tool/approval/事件的控制不如内嵌

> 结合你的目标（允许复制 codex 功能 + Rust-only），建议从 A 起步；如果你希望更快看到结果，可先用 B 跑通 Phase 1，然后迁移到 A。

---

## 6.5 `omne-agent` 需要优先补齐的“Codex 未覆盖领域”

即便 Codex 底座很强，但它不是“PR 工厂”。我们要新增的差异化能力主要是：

- **Repo 注入与本地 bare repo 托管**（Git 服务视角）
- **/tmp workspace 编排**：每个 task 独立 cwd + sandbox roots（并发安全）
- **fmt/check/test/commit/push/pr** 的自动化流水线（且要可观测、可重试）
- **多 PR 合并策略**：顺序、冲突预判、自动修复、二次校验
- **任务级 hook**：比 Codex 的 turn-level notify 更高层（session 完成/PR 创建/合并完成）

---

## 7. 对 `omne-agent` 的第一阶段约束（Responses-only 的落地建议）

1. 配置层：默认 `wire_api = "responses"`，并在我们的 orchestrator 配置中不暴露 chat 选项。
2. Provider 层：只保留/测试 Responses 路径（包括 SSE 事件解析）。
3. 结构化输出：充分使用 `text.format`（JSON schema）来约束：
   - `Architect` 输出任务 DAG
   - `Reviewer` 输出 review 结论结构
   - `Merger` 输出合并计划与顺序
4. 安全与密钥：优先使用 `OPENAI_API_KEY`，后续可引入 `codex-responses-api-proxy` 模式用于多用户/隔离场景。

---

## 8. 其它值得借鉴的工程细节（补充：codex-cli + codex-rs 各子 crate）

> 这一节不重复前文的 “app-server / Responses / Thread 模型”。重点是 Codex 在“分发、配置、硬化、安全降噪、传输形态”上的工程化细节。

### 8.1 Node 只做 launcher（分发边界：薄而正确）

参考：`example/codex/codex-cli/bin/codex.js`

- Node 入口只做：
  - 平台/架构 → target triple 映射
  - 定位 `vendor/<triple>/<component>/<exe>`
  - `spawn()` 子进程并转发信号（保证 Ctrl-C/SIGTERM 语义正确）
  - 可选：把 `vendor/<triple>/path` prepend 到 `PATH`（让 `rg` 之类的依赖可用）
- Node **不**做：任何 agent/core/权限/工具逻辑（这些留给 Rust binary）。

对 `omne-agent` 的直接启发：

- 我们的 v0.3.0 Node 方案应该复刻这个边界（见 `docs/TODO.md`）。
- Node 的职责越薄，Rust 才是唯一可信执行体（安全模型只存在一份）。

### 8.2 npm 包的 vendor 组装脚本（release 工程，不是“顺便写写”）

参考：`example/codex/codex-cli/scripts/build_npm_package.py`

- 显式列出每个 npm 包需要哪些 native components，并把它们复制到固定目录结构（例如把 `rg` 放进 `path/`）。
- release 工作流把 `vendor/` 树作为产物，后续再被 npm 包/发行版复用。

对 `omne-agent`：

- 如果选择 “npm vendoring 多平台二进制”，必须把 vendor layout 与 release pipeline 当作第一等工程，而不是散落脚本。
- 反过来，如果不想背负这套复杂度，就选 “npm thin client + 外部安装二进制”。

### 8.3 “arg0 trick”：单个 Rust binary 伪装多个工具（同时守住安全边界）

参考：`example/codex/codex-rs/arg0/src/lib.rs`

- 通过 `argv0`（可执行文件名）或特殊 `argv1` 触发分发：
  - `codex-linux-sandbox`（Linux 专用）
  - `apply_patch`（也兼容拼错的 `applypatch`）
- 在启动 tokio runtime/多线程**之前**：
  - 加载 `~/.codex/.env`
  - 过滤掉保留前缀变量（Codex 禁止 `.env` 写 `CODEX_*`）
  - 创建 `~/.codex/tmp/path/*` 临时目录，把 helper symlink/bat 放进去并 prepend 到 `PATH`

对 `omne-agent`：

- `.env` 必须“可用但不可提权”：建议对 `OMNE_AGENT_*` 做同类过滤（避免 `.env` 伪造 root/path/sandbox 等关键配置）。
- “工具别名（apply_patch/…）”可以作为分发优化，但一定要在多线程前完成，避免环境竞态。

### 8.4 配置分层 + 可解释性 + 冲突检测（把配置当产品能力）

参考：`example/codex/codex-rs/core/src/config_loader/README.md`

- Codex 的 config loader 不是简单读 `config.toml`，而是输出：
  - effective merged config（合并后的最终值）
  - per-key origins（每个 key 谁赢了，来自哪一层）
  - per-layer fingerprint（稳定 hash，用于乐观并发写/冲突检测）

对 `omne-agent`：

- 我们已经在 v0.2.0 做了部分 “config explain”，但如果要做 GUI/daemon 的在线配置编辑，这种 layer fingerprint 才是“不会互相踩配置”的正确答案。

### 8.5 “命令安全降噪”：已知安全命令 + 保守解析 `bash -lc`

参考：

- `example/codex/codex-rs/core/src/command_safety/is_safe_command.rs`
- `example/codex/codex-rs/core/src/command_safety/is_dangerous_command.rs`
- `example/codex/codex-rs/core/src/bash.rs`

Codex 做了两件值得抄但要克制的事：

- 对一小撮“确定安全、不会写盘/不会执行任意命令”的工具做 allow-list（例如 `ls/cat/rg/git status/...`，并对 `rg --pre` 这类选项做反向禁用）。
- 对 `bash -lc "..."` 并不是“一刀切允许”，而是用 tree-sitter-bash **只接受**：
  - word-only commands
  - 只包含 `&& || ; |` 这类“不会引入副作用”的 operator
  - 并拒绝重定向、命令替换、控制流、括号等复杂构造

对 `omne-agent`：

- 我们文档已强调 `bash -lc` 默认应 forbidden（见 `docs/execpolicy.md`）。
- 但如果未来要在 `ApprovalPolicy=unless_trusted` 下减少噪音，可以考虑引入这种“可证明安全的极小子集解析”，把“安全读取类命令”自动通过，其余一律走审批/拒绝。

### 8.6 sandbox 的“持久化提权防护”：workspace-write 也要保护 `.git` / 配置目录

参考：`example/codex/codex-rs/core/src/seatbelt.rs`

- Codex 的 seatbelt policy 在 workspace-write 下仍然把 `.git` 与 `.codex` 设为只读。
- 测试里直说原因：`.git/hooks/*` 可写会导致用户后续手工 `git commit` 执行恶意代码；修改配置目录可能把下一次运行升级成 full-access。

对 `omne-agent`：

- 我们的等价目录是 `.git` 与 `.omne_agent_data/`；“允许写 workspace”不能等价于“允许写一切”，否则会把 agent 变成持久化后门的写入器。

### 8.7 pre-main hardening：别让宿主环境轻易劫持你

参考：`example/codex/codex-rs/process-hardening/src/lib.rs`

- pre-main 直接做：
  - 禁 core dump
  - 禁 ptrace attach
  - 清理 `LD_*` / `DYLD_*` 这类动态链接劫持入口
- 并且注意非 UTF-8 env key 的边界条件（测试覆盖）。

对 `omne-agent`：

- daemon/长驻进程尤其需要这一层；否则“你以为你在沙箱里”，实际上你已经被 `LD_PRELOAD` 之类的东西劫持了。

### 8.8 传输形态：stdio ↔ UDS 适配（让协议既能嵌入也能常驻）

参考：`example/codex/codex-rs/stdio-to-uds/README.md`

- 把原本只支持 stdio 的 server 挂到 Unix domain socket 上：
  - 便于常驻进程复用
  - 可以用文件权限控制访问（比 TCP 暴露更安全）

对 `omne-agent`：

- 我们已有 daemon/socket 方向（见 `docs/daemon.md`）；Codex 的适配器提供了“保留 stdio 协议但支持常驻”的通用解法。

### 8.9 Auth 与凭据：device code + keyring store（未来如果做“Sign in with ChatGPT”）

参考：

- `example/codex/codex-rs/login/src/device_code_auth.rs`
- `example/codex/codex-rs/keyring-store/src/lib.rs`

- 设备码登录 UX 里明确提示钓鱼风险（“Never share this code”），这不是花活，是必要防线。
- keyring store 被抽象成一个很小的 trait（默认实现 + mock 测试），避免把 OS keyring 细节污染到 core。

对 `omne-agent`：

- 如果未来做 OAuth/device-code 登录，建议同样抽象出 “credential store” 与 “login flow”，并确保日志/事件落盘不会泄露 token。

### 8.10 可观测性：独立的 OTEL 集成 crate（别把 tracing/metrics 绑死在主逻辑）

参考：`example/codex/codex-rs/otel/README.md`

- 把 tracing + metrics 的出口封装成独立 crate，并提供 in-memory exporter 供测试断言。

对 `omne-agent`：

- 我们现在以事件 log 为主，但 daemon/多 workspace 并发后，“指标化”会变得很值钱（排队时延、turn 时延、tool 失败率、重试率等）。
