# 提案：toolchain-public-upstream-bootstrap

## 相关文档

- `openspec/changes/binary-first-toolchain-bootstrap/proposal.md`：已落地的 binary-first 基线。
- `openspec/changes/binary-first-toolchain-bootstrap/specs/toolchain/spec.md`：当前 bootstrap 行为规范。
- `crates/agent-cli/src/main/toolchain.rs`：现有 toolchain bootstrap 实现入口。
- `packages/omne/scripts/postinstall-toolchain.mjs`：npm 薄转发脚本（非核心安装器）。

## 做什么

- 将 `omne toolchain bootstrap` 从“仅依赖 bundled”扩展为“公共上游资源优先”的安装器。
- 明确约束：`git/gh` 的补齐资源不得依赖私有服务器，默认仅使用公共上游。
- 在保持 binary-first 的前提下，引入镜像候选能力（用于公共网络可达性差异场景），但镜像必须是可配置项而非私有内置源。

## 为什么做

- 当前 bundled 方案依赖发行包预置 `git/gh`，会放大安装包体积与多平台打包成本。
- 目标体验是“单可执行优先”，而不是“npm 或私有资源托管优先”。
- 面向中国与国际网络环境时，需要可观测的探测与回退机制，但边界必须是“公共资源”。

## 怎么做

- Rust CLI（`omne`）新增公共源安装路径：
  - 顺序保持：系统 PATH -> managed 已安装 -> bundled -> public upstream。
  - public upstream 阶段由 `omne` 直接完成：release 元数据获取、资产匹配、镜像候选探测、下载、校验、解压、安装。
- 资源策略：
  - 默认官方上游地址（如 GitHub Releases API/asset）。
  - 允许通过环境变量追加公共镜像前缀（按顺序探测），但不引入任何私有默认源。
- 职责边界：
  - npm `postinstall` 继续只转发 `omne toolchain bootstrap`。
  - 核心安装状态机仅在 Rust 侧维护。

## 非目标

- 不新增或依赖任何私有下载服务/CDN。
- 不把 npm 脚本扩展为新的安装核心逻辑。
- 不在本阶段改动 git-domain 业务运行时逻辑。

## 验收标准

- `public-only`：默认配置下，bootstrap 使用的下载地址仅来自公共上游。
- `mirror-fallback`：当主上游不可达时，可按配置镜像候选自动回退，并输出结构化诊断。
- `binary-first`：无 npm 环境时，`omne toolchain bootstrap` 可独立完成同等行为。
- `thin-npm`：npm `postinstall` 仍仅为转发层。
- `testable`：包含单元测试与本地端到端测试（本地假 release 服务 + 假资产）覆盖关键路径。
