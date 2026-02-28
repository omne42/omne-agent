# 任务：toolchain-runtime-extraction

## 相关文档与用途

- `openspec/changes/toolchain-runtime-extraction/proposal.md`：迁移目标与边界。
- `openspec/changes/toolchain-runtime-extraction/specs/toolchain/spec.md`：领域归属与兼容性约束。
- `crates/agent-cli/src/main/toolchain.rs`：待下沉的现有实现。
- `crates/git-runtime/src/lib.rs`：runtime 风格参考。

## 1. 规格与文档

- [x] 新增提案文档（做什么/为什么做/怎么做/验收标准）。
- [ ] 新增 spec delta，明确 `toolchain-runtime` 领域归属。
- [ ] 在任务文档中补齐可执行边界自检命令。

## 2. 领域 crate 建设

- [ ] 创建 `crates/toolchain-runtime`（包名 `omne-toolchain-runtime`）。
- [ ] 下沉核心模型：状态枚举、结果结构、请求参数、环境配置。
- [ ] 下沉核心流程：PATH/managed/bundled/public upstream 状态机。
- [ ] 下沉公共上游安装细节：release 拉取、镜像候选、下载校验、解压安装。

## 3. CLI 接入收敛

- [ ] `agent-cli` 仅保留参数映射、输出渲染、strict 失败策略。
- [ ] 保持 `omne toolchain bootstrap` 现有 JSON 输出兼容。
- [ ] 删除 `agent-cli` 中不应保留的安装器实现细节。

## 4. 测试迁移与验证

- [ ] `toolchain-runtime`：
  - [ ] 单元测试（候选排序、摘要校验、资产选择）。
  - [ ] 本地 mock e2e（release API + 资产下载 + 安装落盘）。
- [ ] `agent-cli`：
  - [ ] preconnect 命令路由测试通过。
  - [ ] `--strict` 语义保持不变。
- [ ] 全链路回归：
  - [ ] `cargo check -p omne`
  - [ ] `cargo test -p omne-toolchain-runtime`
  - [ ] `cargo test -p omne`

## 5. 边界自检（DoD）

- [ ] 领域边界扫描：
  - [ ] `rg -n "fetch_latest_github_release|download_with_candidates|install_from_public|install_gh_from_public|install_git_from_public" crates/agent-cli/src/main/toolchain.rs`（应无输出）
- [ ] 运行时入口扫描：
  - [ ] `rg -n "pub async fn bootstrap_toolchain|ToolchainBootstrapResult|ToolchainBootstrapStatus" crates/toolchain-runtime/src/lib.rs`
- [ ] npm 薄层未回退：
  - [ ] `rg -n "toolchain bootstrap" packages/omne/scripts/postinstall-toolchain.mjs`
