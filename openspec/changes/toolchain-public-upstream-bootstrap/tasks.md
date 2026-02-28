# 任务：toolchain-public-upstream-bootstrap

## 相关文档与用途

- `openspec/changes/toolchain-public-upstream-bootstrap/proposal.md`：目标、边界与验收标准。
- `openspec/changes/toolchain-public-upstream-bootstrap/specs/toolchain/spec.md`：行为约束与状态语义。
- `crates/agent-cli/src/main/toolchain.rs`：bootstrap 核心实现。
- `packages/omne/scripts/postinstall-toolchain.mjs`：npm 薄转发入口。

## 1. 文档与规范

- [x] 新增 proposal（做什么/为什么做/怎么做/验收标准）。
- [x] 新增 spec delta（公共上游、镜像候选、校验与状态约束）。
- [x] 与现有 binary-first 规范保持一致，不回退 npm thin wrapper 边界。

## 2. Rust 安装器能力（核心）

- [x] 在 `omne toolchain bootstrap` 中新增 public upstream 阶段。
- [x] 保持顺序：PATH -> managed -> bundled -> public upstream。
- [x] 增加 release metadata 拉取与目标资产匹配逻辑（`git`/`gh`）。
- [x] 增加镜像候选前缀机制（可配置，默认仅官方上游）。
- [x] 增加下载后校验（checksum）与解压安装流程。
- [x] 结构化输出新增来源类型与失败详情（便于诊断网络/镜像问题）。

## 3. 资源边界与安全约束

- [x] 默认实现不得引用私有服务器域名。
- [x] 文档与代码中明确“镜像为用户配置项，不是私有默认源”。
- [x] 安装目录继续使用 managed toolchain 目录，不污染仓库目录。

## 4. 测试与验证

- [x] Rust 单元测试：
  - [x] 目标资产匹配（不同 target triple）。
  - [x] 镜像候选排序与回退。
  - [x] checksum 校验成功/失败路径。
- [x] Rust 端到端测试：
  - [x] 本地假 release API + 假资产服务。
  - [x] `bootstrap` 可将工具安装到临时 managed 目录。
- [x] Node 侧回归：
  - [x] `npm --prefix packages/omne run check`
  - [x] `npm --prefix packages/omne test`
- [x] CLI 手动验证：
  - [x] `cargo run -q -p omne -- toolchain bootstrap --json`

## 5. 完成定义（DoD）

- [x] 在无私有资源前提下，bootstrap 具备公共上游安装能力。
- [x] npm 保持薄转发，Rust 保持唯一核心状态机。
- [x] 单元测试与端到端测试均通过。
- [x] 变更文档、实现与测试结果可让下一位接手者直接复跑。
