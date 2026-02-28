# 提案：toolchain-runtime-extraction

## 相关文档

- `openspec/changes/toolchain-public-upstream-bootstrap/proposal.md`：公共上游安装能力已落地。
- `openspec/changes/toolchain-public-upstream-bootstrap/specs/toolchain/spec.md`：现有行为约束。
- `crates/agent-cli/src/main/toolchain.rs`：当前实现仍混合了 CLI 与安装状态机。
- `crates/git-runtime/src/lib.rs`：仓库内“runtime 领域下沉”现有模式。

## 做什么

- 新增独立领域 crate：`crates/toolchain-runtime`（包名 `omne-toolchain-runtime`）。
- 将 `toolchain bootstrap` 的核心状态机下沉到 runtime crate：
  - PATH / managed / bundled / public upstream 的判定与安装；
  - 公共上游 release 获取、下载、校验、解压安装；
  - 结构化结果与状态枚举。
- `agent-cli` 仅保留参数解析、调用 runtime、输出渲染与 `--strict` 退出码策略。

## 为什么做

- 当前实现把领域逻辑放在 `agent-cli`，边界不清晰，不利于复用和后续接入（例如 app-server、守护进程或安装器调用）。
- 与现有 `git-runtime` 风格对齐后，职责更稳定：CLI 不再承载核心安装细节。
- 便于后续持续扩展（例如更多工具或策略）而不扩大 CLI 文件复杂度。

## 怎么做

- 创建 `crates/toolchain-runtime`：
  - 对外暴露 `bootstrap` 入口与请求/响应结构；
  - 内部封装下载、校验、解压与路径策略。
- `crates/agent-cli/src/main/toolchain.rs` 改为轻量桥接层：
  - 将 CLI 参数映射为 runtime 请求；
  - 维持原 JSON 输出结构与兼容字段；
  - 维持 `--strict` 失败判定语义。
- 测试迁移：
  - 核心单测与 mock e2e 移入 `toolchain-runtime`；
  - `agent-cli` 保留命令路由与 strict 语义测试。

## 非目标

- 不修改 npm `postinstall` 的“薄转发”边界。
- 不修改当前 public upstream 策略的业务语义与默认来源。
- 不把 toolchain 逻辑迁移到 `safe-fs-tools`。

## 验收标准

- `domain-boundary`：`toolchain` 核心安装状态机不再位于 `agent-cli`。
- `compat`：`omne toolchain bootstrap --json` 的关键字段与语义保持兼容。
- `tests`：`toolchain-runtime` 单测与 e2e 测试通过；`agent-cli` 路由/命令测试通过。
- `boundary-check`：`rg -n "fetch_latest_github_release|download_with_candidates|install_from_public|install_gh_from_public|install_git_from_public" crates/agent-cli/src/main/toolchain.rs` 应无输出。
