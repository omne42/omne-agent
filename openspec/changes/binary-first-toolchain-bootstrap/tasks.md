# 任务：binary-first-toolchain-bootstrap

## 相关文档与用途

- `openspec/changes/binary-first-toolchain-bootstrap/proposal.md`：阶段目标与验收标准。
- `openspec/changes/binary-first-toolchain-bootstrap/specs/toolchain/spec.md`：命令行为与边界规范。
- `crates/agent-cli/src/main/preamble.rs`：CLI 命令入口定义。
- `crates/agent-cli/src/main/app.rs`：preconnect 路由与命令分发。
- `packages/omne/scripts/postinstall-toolchain.mjs`：npm 安装期转发入口。

## 1. 文档与规范

- [x] 完成 proposal（做什么/为什么做/怎么做/验收）。
- [x] 完成 spec delta（命令接口、状态语义、路径约束）。
- [x] 更新 `packages/omne/README.md` 与相关运行文档，说明“二进制优先”。

## 2. CLI 能力下沉（Rust）

- [x] 新增 `omne toolchain bootstrap` 命令：
  - [x] 支持 `--json` 输出。
  - [x] 支持 strict/非 strict 失败策略。
  - [x] 支持受管目录覆盖参数（或环境变量）。
- [x] 统一探测与安装行为：
  - [x] PATH 探测 `git`/`gh`。
  - [x] 缺失时从 bundled 目录安装。
  - [x] 输出每个工具的最终状态。

## 3. npm 轻量化

- [x] 将 `packages/omne/scripts/postinstall-toolchain.mjs` 简化为二进制命令转发。
- [x] 保留失败可控策略（默认不阻断，strict 才阻断）。
- [x] 删除/下线 Node 侧重复的核心安装实现。

## 4. 测试与验证

- [x] Rust 侧：
  - [x] 新增 `toolchain bootstrap` 单元测试（路径解析、状态机）。
  - [x] `cargo test -p omne toolchain`（或等价过滤）通过。
- [x] Node 侧：
  - [x] `npm --prefix packages/omne run check`
  - [x] `npm --prefix packages/omne test`
  - [x] postinstall 转发路径测试覆盖。
- [x] 手动：
  - [x] 二进制直跑 `omne toolchain bootstrap --json`（无 npm）验证。

## 5. 完成定义（DoD）

- [x] 核心 bootstrap 能力在可执行程序内闭环，不依赖 npm 内部脚本实现。
- [x] npm 仅作为下载/调用壳，行为与二进制命令一致。
- [x] 文档、代码、测试一致，可由下一位接手者直接复跑验证。
