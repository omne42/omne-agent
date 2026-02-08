# Codex 上游差异调研：`079fd2adb` → `a90ff831e`（266 commits）

调研日期：2026-02-02  
Snapshot 来源：`/Users/zyp/code/personal/code2/example/codex`  
对比范围：`079fd2adb96bf1b66f3d339e6ee0c0b71f35c322..a90ff831e7d7a049c5638cda6fa72f2abc0b62e6`（`266` commits）

## 0) TL;DR（先讲结论）

这 266 次提交不是“零碎修补”，而是围绕 **Plan mode / 协议 v2 / 权限链路（approvals+execpolicy+network）/ skills&MCP 生态 / TUI 输入体验 / SQLite 状态索引** 做了一轮系统性强化。

对 `omne-agent` 的价值不在“逐行复刻”，而在：把你们已经写死的口径（事件落盘、fail-closed、审批可审计）继续往下推到 **policy/requirements 的统一来源**、**跨平台网络收口**、**可扩展技能生态（含依赖声明）**、以及 **可扩展 UI（plan 流式分段、request_user_input 交互、粘贴/IME 兼容）**。

## 1) 量化概览（帮助定位“哪里变得最多”）

在该 diff 范围内：

- `542 files changed, 48396 insertions(+), 6314 deletions(-)`
- 变更文件数按二级目录统计（越大表示触达越广，不代表复杂度）：
  - `codex-rs/core`（191）
  - `codex-rs/tui`（115）
  - `codex-rs/app-server`（37）
  - `codex-rs/network-proxy`（16，新增 crate）
  - `codex-rs/state`（15，新增 crate）
  - 其余：`codex-rs/protocol`、`codex-rs/execpolicy`、`sdk/typescript`、CI/workflows 等

> 复现命令（在 Codex repo 内）：  
> `git diff --shortstat <old>..<new>`  
> `git diff --name-only <old>..<new> | awk -F/ 'NF>=2 {print $1\"/\"$2} NF==1 {print $1}' | sort | uniq -c | sort -nr | head`

## 2) 主题拆解（按“能力”而不是按“文件”）

下面每一节都包含：

- Codex 做了什么（可落盘、可审计、可测试的那部分）
- 关键落点（文件/模块）
- 对 omne-agent 的启发（建议你们按“是否真的需要”做取舍）

### 2.1 Plan mode：流式 plan 分段 + plan items + TUI 渲染

Codex 明显把“plan”从纯提示词升级为“**可结构化处理的流**”：

- 引入 `<proposed_plan>...</proposed_plan>` 的流式 tag，服务端能在 delta 流中分段识别 plan 文本，并可在 UI 做差异化渲染/交互。
- Plan mode 里出现了 “plan items”（在 app-server 协议 v2 + TUI 中成一等对象），并且有一系列 prompt/规则迭代（大量 `plan prompt` / `plan mode` 提交）。

关键落点：

- `codex-rs/core/src/proposed_plan_parser.rs`（`<proposed_plan>` 流式解析）
- `codex-rs/core/src/tagged_block_parser.rs`（通用 tagged block parser）
- `codex-rs/app-server-protocol/src/protocol/v2.rs`（plan item/线程事件结构扩展）
- `codex-rs/tui/src/chatwidget.rs` + 多个 `snapshots/*plan*`（UI 侧行为锁定）

对 omne-agent 的启发：

- 你们已经有 `Thread/Turn/Item` 的强类型事件模型，Plan mode 最缺的是“**可增量消费**”的结构化分段；`<proposed_plan>` 这种做法简单、低耦合、可测试。
- 不要直接把 plan 当 markdown 解析：先把 “流式分段 + 事件落盘 + UI 渲染”打通，再谈更复杂的 plan schema。

### 2.2 Approvals “变聪明”：缓存/复用 + retry-without-sandbox + execpolicy amendment

这波最关键的不是改名（`/approvals -> /permissions`），而是把审批从“每次都问”变成“**按 key 缓存**”：

- `ToolOrchestrator` 集中编排：`approval → sandbox → attempt → (denied?) retry without sandbox`，并且通过缓存避免重复 re-approval。
- 引入 `ApprovalStore`（按序列化 key 缓存），支持 “ApprovedForSession” 的按-key 复用；`apply_patch` 这种一次影响多个 file 的 tool，会把多个 key 拆开缓存，避免“子集再次问”。
- `ExecApprovalRequirement` 支持携带 `proposed_execpolicy_amendment`：当用户在“重试无沙箱”时批准，可以顺便给出“未来同类命令免审批”的规则建议。

关键落点：

- `codex-rs/core/src/tools/orchestrator.rs`
- `codex-rs/core/src/tools/sandboxing.rs`（`ApprovalStore` / `with_cached_approval` / `ExecApprovalRequirement`）
- `codex-rs/core/src/config_loader/requirements_exec_policy.rs`（见 2.3）
- `codex-rs/protocol/src/prompts/permissions/approval_policy/on_request_rule.md`（prompt 侧文案也被系统化）

对 omne-agent 的启发：

- 你们已有 `remember=true`（thread/session 内记忆）+ `prompt_strict`（强制人工）口径；Codex 的这套“按 key 缓存 + 工具编排器”更偏运行时效率与 UX。
- 如果要学：建议先把 “approval key 的稳定性”当成 **数据结构问题**（别在 UI/提示词里 patch），并保证所有自动复用仍落盘为事件（你们已经写死这个原则了）。

### 2.3 ExecPolicy 与 requirements：把“规则来源”变成可配置且可解释

Codex 在这一段做得很“工程化”：

- 增加 **requirements.toml → execpolicy policy** 的 TOML 表示（但禁止 `allow`，只允许更严格的 `prompt/forbidden`），理由是：requirements 是“叠加约束层”，永远不能用它放宽权限。
- 支持从 requirements 中加载 execpolicy 规则（提交信息里就是 `Load exec policy rules from requirements`），把“企业/组织策略”当成一等输入源。
- 引入 `codex-rs/cloud-requirements`：从 backend 拉取 requirements 文件（目前 best-effort，未来可能收紧到 fail-closed）。

关键落点：

- `codex-rs/core/src/config_loader/requirements_exec_policy.rs`（TOML schema + 解析 + `AllowDecisionNotAllowed`）
- `codex-rs/core/src/config_loader/cloud_requirements.rs`
- `codex-rs/cloud-requirements/src/lib.rs`（抓取 requirements；5s timeout；Business/Enterprise gating）
- commit subjects：`Add exec policy TOML representation (#10026)`、`Fetch Requirements from cloud (#10167)`、`Load exec policy rules from requirements (#10190)`、`Turn on cloud requirements for business too (#10283)`

对 omne-agent 的启发：

- 你们已经有 `docs/execpolicy.md` 写死了 fail-closed 与多层来源（global/mode/thread TODO）；Codex 的 “requirements 只能更严格、不能放宽” 这个约束非常值得照搬。
- 这会直接服务你们的 `docs/plans/local_github.md`：forge/token/remote 操作如果要在企业环境跑，最终一定会需要一个“组织侧可下发的 restrictions 层”。

### 2.4 Network sandbox：引入本地策略代理（HTTP + SOCKS5 + admin），并与 execpolicy 打通

Codex 新增了 `codex-network-proxy`，目的非常明确：在 OS sandbox 不可靠/不统一的情况下，用 **进程外代理**把 egress 口收住：

- HTTP proxy（默认 `127.0.0.1:3128`）+ 可选 SOCKS5（默认 `127.0.0.1:8081`）+ admin API（默认 `127.0.0.1:8080`）
- allowlist-first：`allowed_domains` 为空时默认全阻断（fail-closed）
- denylist 永远优先；并对 local/private 做额外阻断（防 SSRF/内网探测）
- “limited mode” 只允许 `GET/HEAD/OPTIONS`，并阻断 HTTPS CONNECT / SOCKS5（避免绕过方法限制）
- 还有一个非常实用的 hook：policy decider 可拿到 `command` / `exec_policy_hint`，用于把“用户已批准 curl/git”等信号映射成网络放行（但 deny 仍然赢）

关键落点：

- `codex-rs/network-proxy/README.md`（完整设计/边界）
- `codex-rs/network-proxy/src/*`（实现）
- commit subjects：`feat: introducing a network sandbox proxy (#8442)`、`feat(network-proxy): add a SOCKS5 proxy with policy enforcement (#9803)`、以及一串 windows/sandbox 修复

对 omne-agent 的启发：

- 你们 `docs/modes.md`/`docs/approvals.md` 已经把“权限链顺序”写死；网络这一块如果想跨平台强制，就别迷信 OS 沙箱，代理是现实路线。
- 但要小心：代理会引入 build/依赖复杂度（BoringSSL/CMake），也会引入“配置错误导致全阻断”的 UX 问题；要配套 `config diagnostics`（见 2.6）。

### 2.5 Skills 生态：更多来源、元数据、依赖声明、追踪事件

从 commit subjects 能看出 Codex 在 skills 上做了系统化：

- 支持从 `.agents/skills` 加载（除了原本的 skills 目录）。
- 改进 system skills marker：递归包含 nested folders。
- system skills 同步机制（从 public repo sync）。
- `SKILL.json` 元数据加载（移除 `SKILL.toml` fallback）。
- skill invocation tracking（埋点/事件）。
- 处理 skill 对环境变量的依赖（`env_var_dependencies`），并且有 “Auto install MCP dependencies when running skills with dependency specs”。

关键落点：

- `codex-rs/core/src/skills/loader.rs`、`codex-rs/core/src/skills/injection.rs`
- 新增：`codex-rs/core/src/skills/env_var_dependencies.rs`
- commit subjects 列表见：`git log ... | rg -i '\\bskills?\\b'`

对 omne-agent 的启发：

- 你们已经实现了 skills 加载（甚至兼容 `.codex/skills`），下一步不是“再多支持一个目录”，而是把 **skill 元信息/依赖/安装/审计** 做成闭环：
  - “缺啥依赖”要能结构化提示（避免模型瞎猜）。
  - “安装依赖”必须走 `process/start` + execpolicy + approvals，并把结果写 artifact（可回放）。

### 2.6 Config diagnostics：把 TOML/YAML 的错误定位到行列范围

Codex 新增了一套 config 诊断辅助，把“解析/校验失败”变成 “具体文件:行:列 + 高亮范围”：

- 用 `serde_path_to_error` 把 schema 校验失败映射到字段路径
- 用 `toml_edit` 在文档里定位 span，尽量给出准确 range

关键落点：

- `codex-rs/core/src/config_loader/diagnostics.rs`

对 omne-agent 的启发：

- 你们会引入越来越多 `.omne_agent_data/spec/*.yaml|toml`，没这个诊断能力，最后就会变成“用户看到一个大 error 但不知道改哪里”。
- 这属于“工具型工程化”，不 glam，但能显著减少 support 成本。

### 2.7 SQLite state：把 rollout/threads 元数据抽到 SQLite（可选 feature）

Codex 引入 `codex-state`（SQLite），并在 `core` 里做了 feature gating 与 backfill：

- 目标是：不用每次扫 JSONL rollouts，就能做 thread list / metadata 查找 / logs 查询（并支持 parity check）。
- `init_if_enabled` 会在首次创建 DB 后后台 backfill。

关键落点：

- 新增 crate：`codex-rs/state/*`
- `codex-rs/core/src/state_db.rs`（core-facing handle + reconcile）
- `codex-rs/core/src/rollout/metadata.rs`（配合抽取元数据）

对 omne-agent 的启发：

- 你们现在是 JSONL eventlog + replay 派生 state；如果 thread 数量上来（或要做“按 updated_at 排序/过滤/全文检索”），纯扫文件会成为瓶颈。
- SQLite 不一定是必须，但“把索引从事件流中分离出来”这个方向很现实：至少要有一个可选的索引层（SQLite/SQLite + FTS5/甚至纯 JSONL 反向索引都行）。

### 2.8 TUI：输入体验与跨平台（Windows/WSL/IME）工程化

你之前总结里漏掉的一个大头：Codex 把 TUI 的 “输入框”当成产品在做，而不是 demo：

- 新增 `docs/tui-chat-composer.md`，完整记录 `ChatComposer` 状态机与 Windows paste-burst 兼容方案。
- 引入 paste burst detector：处理 bracketed paste 不可靠时的 key burst，避免误触快捷键/误提交。
- 细化 bottom pane：`chat_composer`、`request_user_input`、`slash_commands` 等模块化，并大量用 snapshot tests 锁定 UI 输出。
- 新增 `cwd_prompt`、`personality selection popup`、`plan implementation popup` 等交互面板（从 snapshots 可见）。

关键落点：

- `docs/tui-chat-composer.md`
- `codex-rs/tui/src/bottom_pane/chat_composer.rs` + `.../paste_burst.rs`
- `codex-rs/tui/src/bottom_pane/request_user_input/*`
- 大量 `codex-rs/tui/src/**/snapshots/*.snap`

对 omne-agent 的启发：

- 你们的 TUI 也是 ratatui；跨平台输入问题（尤其 Windows）迟早会踩坑。Codex 的做法很对：把输入行为写成 state machine + 用 snapshot/集成测试钉住。
- 这类改动“看起来只是 UI”，其实是降低用户误操作成本的核心。

### 2.9 Threads/协议 v2：thread/read、archive/unarchive、ephemeral、source filter

Codex 在 thread 能力上补了很多“产品级必需件”：

- `thread/read` API（读取单 thread）
- archived threads 支持 + `thread/unarchive`（恢复 archived rollouts，并刷新 sidebar ordering）
- ephemeral threads（临时线程语义）
- thread list filter（按 source kind 过滤，部分需要 post-filter）
- exec 模式自动订阅新 thread

关键落点：

- `codex-rs/app-server/tests/suite/v2/thread_read.rs`（新增）
- `codex-rs/app-server/tests/suite/v2/thread_unarchive.rs`（新增）
- `codex-rs/app-server/src/filters.rs`（source kind filter 计算）

对 omne-agent 的启发：

- 你们已经有 `thread/*` 很多接口；这里最值得学的是：把“边缘语义”做成测试（尤其是 archive/unarchive 对排序与恢复的影响），否则 UI 会出各种“我找不到 thread”类问题。

### 2.10 Connectors：目录列表 + MCP 可用性合并 + TUI/Slash 命令

Codex 的 Connectors 体系是把“生态入口”产品化：

- 从 ChatGPT 目录拉 “所有 connectors”（directory list，分页）
- 从 MCP tools 推断 “当前可用 connectors”
- merge 两者，并生成 install_url / display_label；过滤 disallowed ids/prefix（安全/策略原因）

关键落点：

- `codex-rs/chatgpt/src/connectors.rs`（directory list + merge）
- `codex-rs/core/src/connectors.rs`（从 MCP tools 推断 accessible + merge）
- commit subjects：`[connectors] part 1/2` + MCP scopes/support

对 omne-agent 的启发：

- 你们在做 `local_github`（Forgejo）与未来外部能力接入时，会面临同样的问题：**“可用/已授权/可安装”是三件不同的事**，必须拆开建模，否则 UX 会非常混乱。

### 2.11 Dynamic tools：按 thread 注入“动态工具规格”，并把调用交给 client 执行

Codex 这轮把 “dynamic tools” 从概念落成了一条完整链路（协议 → 校验 → 注入 → 事件化请求/响应）：

- 协议层（v2）允许 thread/start 携带 `dynamic_tools`（`name/description/input_schema`）。
- app-server 会校验：
  - tool name 不能为空、禁止前后空白、不能与 MCP tool 冲突、不能占用 `mcp`/`mcp__*` 这种保留命名空间
  - `input_schema` 必须是 core 支持/可净化的 schema（防止注入奇怪结构）
- core 会把 dynamic tools 合并进可调用 tools 列表（与 MCP tools 一起），并把 “dynamic tool call” 事件化：
  - `DynamicToolCallRequest { call_id, turn_id, tool, arguments }`
  - client 返回 `DynamicToolCallResponse { output, success }` 后，core 生成 `DynamicToolResponse` 作为 function call output 返回给模型
- dynamic tools 还会被写进 session/rollout（从代码看：当 thread/start 没传时，会从历史里 `get_dynamic_tools()` 复用）

关键落点（抽样）：

- `codex-rs/protocol/src/dynamic_tools.rs`（types）
- `codex-rs/app-server-protocol/src/protocol/v2.rs`（`dynamic_tools` 字段）
- `codex-rs/app-server/src/codex_message_processor.rs`（`validate_dynamic_tools`）
- `codex-rs/core/src/tools/handlers/dynamic.rs`（把调用转换成 `EventMsg::DynamicToolCallRequest` + oneshot 等待）
- `codex-rs/app-server/src/dynamic_tools.rs`（接收 client response 并提交 `Op::DynamicToolResponse`）

对 omne-agent 的启发：

- 你们已有“tool dispatch + approvals + 事件落盘”骨架；dynamic tools 是把“外部能力”做成一等公民的关键：它允许 **不发版**就把一组工具 schema 安全地注入到某个 thread。
- 但必须坚持两条硬边界：
  1) schema 必须可校验/可净化（否则等价 prompt 注入一个“任意 JSON”入口）
  2) dynamic tool 的实际执行必须走审批链（或者至少落盘可审计），否则会变成绕过路径

### 2.12 AuthMode / 外部鉴权：把“鉴权模式”当作协议字段与一等状态

Codex 把鉴权从“隐式环境变量/文件”更进一步做成“可在协议里显式表达”的状态：

- `AuthMode` 在 app-server-protocol 里是显式枚举：`apiKey | chatgpt | chatgptAuthTokens`（其中 `chatgptAuthTokens` 标注为 internal-only、外部宿主 app 负责 refresh，Codex 仅内存存储）。
- app-server 侧支持 external auth mode（commit subject `support external auth mode`），并且 core 侧对 `CodexAuth` 做了“invalid state cannot be represented”的结构化重构（这本质是在修数据结构，不是 UI/逻辑小修小补）。

关键落点（抽样）：

- `codex-rs/app-server-protocol/src/protocol/common.rs`（`enum AuthMode`）
- `codex-rs/core/src/auth.rs`、`codex-rs/core/src/auth/storage.rs`（存储与模式解析）
- commit subjects：`feat(app-server): support external auth mode (#10012)`、`feat: refactor CodexAuth so invalid state cannot be represented (#10208)`

对 omne-agent 的启发：

- 你们正在做 `openai-provider-thread-config`：这类改动最容易变成“配置散落各处 + 推断逻辑越来越黑盒”。把“鉴权模式”抬到协议/事件里是一种止血手段：可解释、可审计、也更容易写测试覆盖。

## 3) 给 omne-agent 的“下一步建议”（按性价比排序）

1. 先完成 `t0`：把这份 diff 继续补齐成“可采纳/不可采纳”结论（别靠脑补）。
2. 抽出 `execpolicy rules` 的“叠加约束层”（requirements 只能更严格），并补齐 `config-explain` 的解释输出。
3. 定一个“跨平台网络收口”的落地路线：优先评估 proxy 模式（像 codex-network-proxy），别指望 OS sandbox 一把梭。
4. 把 approvals 做成“按 key 缓存”的数据结构（别只靠 prompt），并把自动复用仍作为事件落盘审计。
5. 把 Plan mode 变成“可结构化增量消费”的流（参考 `<proposed_plan>` tag），再谈更复杂的 plan schema。
6. skills：补齐元数据/依赖/安装/审计闭环（否则生态永远只停留在“能加载一个文件”）。
7. 如果 thread 数量/查询需求上来，预留/验证一个索引层（SQLite 或其它），别让 JSONL 扫描成为未来架构瓶颈。

---

## 4) 附：本次调研用到的命令（便于复现）

在 Codex repo（`/Users/zyp/code/personal/code2/example/codex`）内：

```bash
old=079fd2adb96bf1b66f3d339e6ee0c0b71f35c322
new=a90ff831e7d7a049c5638cda6fa72f2abc0b62e6

git rev-list --count $old..$new
git diff --shortstat $old..$new
git diff --dirstat=files,0 $old..$new | sort -nr | head
git diff --name-only $old..$new | awk -F/ 'NF>=2 {print $1\"/\"$2} NF==1 {print $1}' | sort | uniq -c | sort -nr | head
git diff --name-status $old..$new | awk '$1==\"A\" {print $2}' | head
git log --format='%h %s' $old..$new | rg -i 'plan|mode|approvals|execpolicy|requirements|mcp|skill|tui|sandbox|network|thread' | head
```

## 5) 附：按主题抓取的 commit subjects（便于继续深挖）

> 说明：这里仅按 commit subject 的关键词抓取，不保证“无漏/无误分类”；但足够作为后续逐条 `git show <hash>` 深读的索引入口。

### approvals / execpolicy

```text
a8c9e386e feat(core) Smart approvals on (#10286)
0fac2744f Hide /approvals from the slash-command list (#10265)
5662eb8b7 Load exec policy rules from requirements (#10190)
23db79fae chore(feature) Experimental: Smart Approvals (#10211)
34f89b12d MCP tool call approval (simplified version) (#10200)
2d9ac8227 fix: /approvals -> /permissions (#10184)
71b8d937e Add exec policy TOML representation (#10026)
f815fa14e Fix execpolicy parsing for multiline quoted args (#9565)
a4cb97ba5 Chore: add cmd related info to exec approval request (#9659)
```

### requirements / cloud

```text
9a10121fd fix(nix): update flake for newer Rust toolchain requirements (#10302)
47faa1594 Turn on cloud requirements for business too (#10283)
149f3aa27 Add enforce_residency to requirements (#10263)
5662eb8b7 Load exec policy rules from requirements (#10190)
e85d019da Fetch Requirements from cloud (#10167)
ddc704d4c backend-client: add get_config_requirements_file (#10001)
```

### skills

```text
5fb46187b fix: System skills marker includes nested folders recursively (#10350)
e470461a9 Sync system skills from public repo for openai yaml changes (#10322)
dfba95309 Sync system skills from public repo (#10320)
aab3705c7 Make skills prompt explicit about relative-path lookup (#10282)
39a6a8409 feat: Support loading skills from .agents/skills (#10317)
b164ac6d1 feat: fire tracking events for skill invocation (#10120)
bdd8a7d58 Better handling skill depdenencies on ENV VAR. (#9017)
3bb8e69dd [skills] Auto install MCP dependencies when running skils with dependency specs. (#9982)
2f8a44bae Remove load from SKILL.toml fallback (#10007)
a641a6427 feat: load interface metadata from SKILL.json (#9953)
c6ded0afd still load skills (#9700)
```

### MCP / shell-tool-mcp

```text
34f89b12d MCP tool call approval (simplified version) (#10200)
48f203120 fix: unify `npm publish` call across shell-tool-mcp.yml and rust-release.yml (#10182)
3e798c5a7 Add OpenAI docs MCP tooltip (#10175)
b4b476300 fix(ci) missing package.json for shell-mcp-tool (#10135)
83d7c4450 update the ci pnpm workflow for shell-tool-mcp to use corepack for pnpm versioning (#10115)
7b34cad1b fix(ci) more shell-tool-mcp issues (#10111)
f7699e048 fix(ci) fix shell-tool-mcp version v2 (#10101)
35e03a071 Update shell-tool-mcp.yml (#10095)
2a624661e Update shell-tool-mcp.yml (#10092)
3bb8e69dd [skills] Auto install MCP dependencies when running skils with dependency specs. (#9982)
337643b00 Fix: Render MCP image outputs regardless of ordering (#9815)
bdc4742bf Add MCP server `scopes` config and use it as fallback for OAuth login (#9647)
a2c829a80 [connectors] Support connectors part 1 - App server & MCP (#9667)
```

### plan / mode（节选）

```text
3dd9a37e0 Improve plan mode interaction rules (#10329)
30ed29a7b enable plan mode (#10313)
2d6757430 plan mode prompt (#10308)
83317ed4b Make plan highlight use popup grey background (#10253)
b7351f7f5 plan prompt (#10255)
9b29a48a0 Plan mode prompt (#10238)
2d10aa685 Tui: hide Code mode footer label (#10063)
ec4a2d07e Plan mode: stream proposed plans, emit plan items, and render in TUI (#9786)
1ce722ed2 plan mode: add TL;DR checkpoint and client behavior note (#10195)
a0ccef9d5 Chore: plan mode do not include free form question and always include isOther (#10210)
11958221a tui: add feature-gated /plan slash command to switch to Plan mode (#10103)
47aa1f3b6 Reject request_user_input outside Plan/Pair (#9955)
cb2bbe5cb Adjust modes masks (#9868)
58450ba2a Use collaboration mode masks without mutating base settings (#9806)
b3127e2ee Have a coding mode and only show coding and plan (#9802)
69cfc73dc change collaboration mode to struct (#9793)
4210fb9e6 Modes label below textarea (#9645)
```

### threads

```text
d3514bbdd Bump thread updated_at on unarchive to refresh sidebar ordering (#10280)
89c5f3c4d feat: adding thread ID to logs + filter in the client (#10150)
dabafe204 feat: codex exec auto-subscribe to new threads (#9821)
247fb2de6 [app-server] feat: add filtering on thread list  (#9897)
62266b13f Add thread/unarchive to restore archived rollouts (#9843)
83775f4df feat: ephemeral threads (#9765)
515ac2cd1 feat: add thread spawn source for collab tools (#9769)
733cb6849 feat(app-server): support archived threads in thread/list (#9571)
80240b3b6 feat(app-server): thread/read API (#9569)
```

### sandbox / network / windows

```text
66de985e4 allow elevated sandbox to be enabled without base experimental flag (#10028)
3f3916e59 tui: stabilize shortcut overlay snapshots on WSL (#9359)
28051d18c enable live web search for DangerFullAccess sandbox policy (#10008)
30eb655ad really fix pwd for windows codex zip (#10011)
c40ad65bd remove sandbox globals. (#9797)
877b76bb9 feat(network-proxy): add a SOCKS5 proxy with policy enforcement (#9803)
313ee3003 fix: handle utf-8 in windows sandbox logs (#8647)
e2bd9311c fix(windows-sandbox): remove request files after read (#9316)
77222492f feat: introducing a network sandbox proxy (#8442)
d9232403a bundle sandbox helper binaries in main zip, for winget. (#9707)
0e4adcd76 use machine scope instead of user scope for dpapi. (#9713)
e117a3ff3 feat: support proxy for ws connection (#9719)
4d48d4e0c Revert "feat: support proxy for ws connection" (#9693)
```

### file-search

```text
13e85b154 fix: update file search directory when session CWD changes (#9279)
d59685f6d file-search: multi-root walk (#10240)
b8156706e file-search: improve file query perf (#9939)
```

### connectors

```text
b9cd089d1 [connectors] Support connectors part 2 - slash command and tui (#9728)
a2c829a80 [connectors] Support connectors part 1 - App server & MCP (#9667)
```

### sqlite / state

```text
377ab0c77 feat: refactor CodexAuth so invalid state cannot be represented (#10208)
3878c3dc7 feat: sqlite 1 (#10004)
```
