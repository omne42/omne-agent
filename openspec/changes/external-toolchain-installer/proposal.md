# 提案：external-toolchain-installer

## 相关文档

- `docs/start.md`：当前全链路目标与约束（含 worktree/copy 路径）。
- `docs/rts_workflow.md`：运行时交互与可观测要求。
- `openspec/changes/toolchain-runtime-extraction/proposal.md`：现有 `toolchain-runtime` 领域边界。
- `crates/toolchain-runtime/src/lib.rs`：当前内置安装状态机实现。

## 最终目标

构建一个“专属的 Git 工具链获取能力”，支撑 `OmneAgent` 的工作树（worktree）全链路在用户机器未预装 `git`/`gh` 时仍可用，且符合公共资源来源、网络可达性与安全边界要求。

## 做什么

- 在 `p/` 下创建独立仓库（后续命名为 `omne-toolchain-installer`）作为辅助安装工具。
- `omne-agent` 不再直接承载复杂下载/镜像路由逻辑，改为调用该工具完成 `git`/`gh` 的准备。
- 引入可选的公共网关（Cloudflare Worker）能力，仅允许固定白名单路由，不提供任意代理。

## 为什么做

- 当前安装逻辑和业务 runtime 混在 `omne-agent` 内，扩展与运维成本高。
- “无预装 Git 也可跑完整链路”需要独立演进安装策略（平台、网络、镜像、重试、验签）。
- 反滥用能力需要单独的接口边界与策略，不应散落在业务仓库里。

## 怎么做

1. 文档先行：在 `omne-agent` OpenSpec 明确分层边界、调用契约与验收标准。  
2. 新仓库先定义协议：CLI JSON 输出、下载候选顺序、镜像切换与失败语义。  
3. `omne-agent` 接入：`toolchain bootstrap` 改为优先调用外部安装器，保留兼容 JSON 字段。  
4. 安全落地：Worker 只做固定路由与重定向，不支持任意 URL 转发；配合限流策略。  

## 边界约束

- `omne-agent` 的 `crates/app-server` 不得新增 git/gh 安装实现。
- `omne-agent` 的 `crates/toolchain-runtime` 不得继续扩张为“完整下载器平台”，仅保留编排与兼容层。
- 安装来源必须是公共可验证资源（官方发布页/公共镜像），不依赖私有文件服务器。

## 非目标

- 本变更不在 `safe-fs-tools` 中实现安装能力。
- 本变更不引入本地 Git 服务进程。
- 本变更不改变现有 worktree-first、copy-fallback 的工作区策略。

## 验收标准

- 存在独立安装仓库，且可独立运行安装流程并输出结构化结果。
- `omne toolchain bootstrap --json` 在调用新仓库后，字段契约保持兼容。
- `crates/app-server` 中不存在 git/gh 安装实现。
- 网关实现不具备开放代理能力（仅白名单路径可访问）。
