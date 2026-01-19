# OpenAI CLI Agent（Rust）[即codex] 能力与设计分析（Responses API 重点）

> Snapshot: `b66018a`（对应 `example/` 中的 OpenAI Rust CLI agent 快照）
>
> 结论先行：该上游 Rust 实现已经具备我们要做“并发 AI task + /tmp 临时目录隔离 + 工具执行 + 审批/沙箱 + 可被 UI 驱动”的大部分底座能力。我们最合理的路线是：**在遵守“允许 copy、禁止引用依赖”的前提下复制其关键实现到本项目**，优先补齐“多任务编排 + Git PR 流水线 + PR 合并”，并且**第一阶段仅启用 Responses 接口**。

> 复用规则见：`docs/upstream_reuse_policy.md`（最终产物不得依赖 `example/` 中任何仓库）。

---

## 1. 上游快照包含的“可复用资产”

该快照是一个大仓，主要可复用资产集中在：

- Rust workspace：核心业务逻辑、TUI、非交互执行模式、app-server（JSON-RPC/事件流）、sandbox/审批/执行策略、MCP、Responses 相关工具等。
- 打包/发布脚本：用于分发 Rust 二进制与 native 依赖。
- SDK：可作为未来对外 API/客户端设计参考（MVP 不依赖）。

---

## 2. 能力盘点（按系统能力域）

### 2.1 多运行形态：交互式 UI / 非交互 / app-server

上游同时提供三类入口：

- 交互式 TUI：适合本地人机协作与审批交互。
- 非交互执行模式（headless）：适合自动化与批处理（我们并发 worker 可优先走这一形态）。
- app-server：面向 IDE/富客户端的后端（JSON-RPC over stdio，turn/item 事件流式推送，适合做编排控制面与 UI）。

### 2.2 会话模型：Thread / Turn / Item（可追溯）

上游把一次交互拆成：

- Thread：一次对话会话
- Turn：一轮用户输入→模型输出
- Item：turn 内的细粒度产物（message、reasoning、shell、file edit、diff、approval request…）

关键价值：

- 所有 side effects 都能被 item 化与持久化，天然适配 UI 渲染与审计回放。
- thread 支持 resume/fork，天然适配“并发分支任务”的表达。

补充：上游有统一的 thread 生命周期管理器（维护 threads 的内存集合、事件广播、模型/技能等全局资源），这与我们后续要做的 orchestrator/agent manager 十分接近。

### 2.3 安全与执行：Sandbox + Approvals + ExecPolicy（安全底座）

上游在“执行命令/改文件”这条链路上做了系统性工程：

- sandbox policy（跨平台）：`read-only` / `workspace-write` / `danger-full-access`
- Linux sandbox（Landlock 等）
- macOS sandbox（Seatbelt/sandbox-exec）
- exec policy：前缀规则（prefix rule）引擎，能对命令 token 序列匹配并给出 allow/prompt/forbidden 结果
- exec server：通过 patched shell + execve wrapper，把 shell 内每一次 `execve` 都纳入策略决策（Run / Escalate / Deny），避免“shell 拼接绕过”
- process hardening：pre-main 阶段禁 core dump、禁 ptrace attach、清理危险环境变量等

对我们而言，这是“自动化全生命周期流水线”的必要底座：fmt/check/build/deploy 都会触发命令执行，必须有可审计的 policy 与审批边界。

### 2.4 MCP（工具生态）

上游支持 MCP client/server：

- MCP client：启动时连接外部工具服务（扩展 agent 工具能力）。
- MCP server（实验）：让其它 MCP client 把上游 agent 当作一个工具调用。

对我们来说，MVP 不依赖 MCP，但应把它当成“未来能力扩展槽位”预留接口。

### 2.5 Provider 与 wire API：Responses 为主线

上游明确区分 wire-level API：

- Responses（主线）
- Responses over WebSocket（实验）
- Chat Completions（已明确不推荐，未来会被移除）

对我们第一阶段的约束：

- **只做 Responses**：配置与代码路径默认走 Responses；避免引入 chat 兼容逻辑，减少未来维护成本。

### 2.6 配置系统与 hook 能力（可运维性）

上游配置对象包含：

- config layering（记录配置来源与合并过程，便于排障）
- 模型选择（通用/专用 review model 等）
- sandbox/approval/环境策略
- 指令来源（例如项目级指令、开发者指令、compact prompt 等）
- notify hook：支持在 turn 完成时执行外部程序并附带 JSON payload（这与我们“完成时 hook 回主流程”天然同构）

---

## 3. Responses 接口实现细节（本期重点）

### 3.1 Prompt 抽象：统一承载 instructions / input / tools

上游把“对模型的调用”抽象为一个统一 Prompt：

- `instructions`：完全解析后的系统指令
- `input`：结构化对话历史（含 tool messages）
- `tools`：结构化工具定义
- `parallel_tool_calls`：是否允许并行工具调用
- `output_schema`：可选 JSON schema（用于约束最终输出）

这使得上层业务不需要关心 endpoint 的 JSON 细节，只要构造 Prompt 即可。

### 3.2 Responses 请求体控制：reasoning + text.format

上游对 Responses 的关键控制面做了类型化建模：

- `reasoning`：effort + summary（适配推理模型的可控输出）
- `text`：verbosity + `text.format`（JSON schema 约束输出）
- `include`、`prompt_cache_key`、`parallel_tool_calls` 等

对我们后续角色体系（Architect/Reviewer/Merger）尤其重要：可以用 JSON schema 让“任务 DAG/合并计划/风险清单”变成结构化输出，而不是靠 prompt 约束文本格式。

### 3.3 SSE 解析：把 streaming events 变成强类型事件流

上游把 Responses 的 SSE stream 解析为一组强类型事件：

- output_text delta
- output_item done/added（结构化 item）
- reasoning delta / reasoning summary delta
- completed/done（含 token usage）
- failed（含错误映射与重试/配额/上下文窗口等分类）
- rate limit snapshot、models etag 等可观测性信息

这对并发 orchestrator 很关键：我们可以用事件流驱动 UI、hook、以及“失败重试/回滚/降级策略”。

### 3.4 严格 Responses 代理（密钥隔离）

上游提供一个“只允许转发 `POST /v1/responses`”的严格代理：

- API key 从 stdin 读取（避免出现在环境变量/命令行历史）
- 对敏感内存做硬化（如 mlock/zeroize 等，减少泄露面）
- 可选提供 http shutdown 入口，便于不同权限用户协作

对我们而言，这是未来做“多用户/隔离执行/远端 worker”时的重要参考实现。

---

## 4. app-server：把上游 agent 变成“可被编排的后端”

上游 app-server 的关键点：

- JSON-RPC（省略 jsonrpc 字段），JSONL over stdio
- 初始化握手（initialize/initialized）
- thread/turn API（start/resume/fork/list/archive 等）
- turn 支持覆盖 cwd、sandbox/approval policy、model、effort、summary、output schema
- turn/item 事件流（工具调用、diff、审批请求等）
- 支持 schema 生成（TS/JSON schema），便于 UI/SDK 对齐

对我们而言，app-server 协议可以作为：

- worker 控制面（orchestrator 作为 client）
- UI/daemon 的稳定后端协议

---

## 5. 值得直接抄作业的工程原则

1. **协议 types 与业务逻辑分离**：协议层保持最小依赖，业务逻辑通过扩展 trait/adapter 实现。
2. **安全是系统工程**：不是“提示用户确认”这么简单，而是把命令执行路径纳入策略决策，避免 shell 绕过。
3. **Responses-first**：避免引入未来会被移除的旧接口路径，减少维护成本。

---

## 6. 我们应如何“复制式实现”并保持零依赖

有两条落地路线（都必须满足 `docs/upstream_reuse_policy.md`）：

### 路线 A：复制关键 Rust 模块到本项目 workspace（推荐长期）

- 优点：
  - 事件/状态可更细粒度接入（无需再解析 JSON-RPC 文本）
  - 更容易做“多线程并发 thread/turn 管理”
  - 更容易把 sandbox/approvals/execpolicy 作为库复用
- 我们新增的差异化核心：
  - repo 注入与本地 bare repo 托管
  - `/tmp/{repo}_{session}/tasks/{task}` 并发 workspace 编排
  - fmt/check/test/commit/push/本地 PR 元数据
  - 多 PR 合并与冲突修复（Merger Agent）
  - session/task/pr 级 hook（在 turn-level notify 之上）

### 路线 B：外部 orchestrator 驱动上游二进制（仅用于极早期验证）

不推荐作为长期形态（容易形成运行时依赖），但可用于非常早期的流程验证：

- 快速验证：并发 task → fmt/check → commit → 产出 PR 元数据 → 合并策略
- 一旦流程跑通，应尽快迁移到路线 A，以确保最终产物“零依赖 `example/`”

---

## 7. 第一阶段（Responses-only）的落地建议

1. 配置层默认走 Responses wire API，不暴露旧接口选项。
2. 只保留/测试 Responses 的 streaming 事件路径（含错误分类与 token usage）。
3. 角色输出尽量结构化：用 JSON schema 约束：
   - Architect：任务 DAG（含依赖/并发边界）
   - Reviewer：review 结论结构化
   - Merger：合并计划与顺序
4. 密钥策略先简单（环境变量），后续再引入严格代理/隔离执行。
