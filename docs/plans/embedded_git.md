# Plan: Embedded Git (no system `git` dependency)

## Goal

让 `omne-agent` 在用户机器**未安装**系统 `git` 时，依然可以完成最小 Git 工作流（至少覆盖我们当前依赖 `git` 的能力，例如生成 diff/patch artifacts），并保持二进制体积可控。

## Non-goals

- 不做 GitHub PR/merge 自动化（仍属于上层交付适配层）。
- 不追求完整复刻所有 `git` 子命令；先做覆盖面最小且可验证的一组能力。
- 不改变 “Git 不是核心域，默认 `patch-only`” 的定位（见 `docs/development_process.md`）。

## Constraints

- 不能破坏现有 `thread_diff` / `thread_patch` 的可审计语义（产物仍落到 artifacts，默认只注入元信息）。
- 必须尊重现有 `sandbox_network_access`、`execpolicy`、`approval` 等边界（禁网时不允许 `clone/fetch` 这类网络操作）。
- 需要提供可复现的 size baseline 与裁剪策略（feature gating、LTO、strip、panic=abort）。

## Decision

当前代码库并没有内嵌 Git 实现：`thread_diff/thread_patch` 通过 `process/start` 调用系统 `git`（例如 `crates/app-server/src/main/thread_observe/disk_git_diff.rs`）。如果要做到 “用户不装 git 也能用”，需要引入内嵌实现：

### Options

1. **`gix`（gitoxide）**：纯 Rust Git 实现，可做 feature 裁剪，跨平台更一致。
2. **`git2`（libgit2 绑定）**：成熟但引入 C 依赖（以及 TLS 依赖），交叉编译与体积/动态链接策略更复杂。
3. **打包 `git` 可执行文件**：实现快但体积大、平台适配/更新成本高。

### Recommendation

优先 `gix`，并把 Git 能力作为“交付适配层/工具实现细节”，不渗透到核心域模型。

## Definition of Done (DoD)

- 在 PATH 不含 `git` 的环境中：
  - `thread_diff` 仍能生成 diff artifact。
  - `thread_patch` 仍能生成 patch artifact（含 binary 时的策略单独说明）。
- `sandbox_network_access=deny` 时，任何网络型 Git 操作被拒绝并落盘（行为与现有 `process/start` 拒绝一致）。
- `cargo fmt --all && cargo check --workspace --all-targets && cargo test --workspace && cargo clippy --workspace --all-targets --all-features -- -D warnings` 全绿。
- 增加 size report（至少记录：当前基线、启用内嵌 Git 后、裁剪后），并在文档中写明如何复现。

## Task DAG

### t1 - 定义最小 Git 能力面（spec）

- Files:
  - `docs/plans/embedded_git.md`
  - （如需要）`docs/special_directives.md`、`docs/artifacts.md`
- Acceptance:
  - 明确哪些能力必须内嵌：diff/patch（必选）、clone/fetch（可选）、status（可选）。
  - 明确 binary patch 策略：先支持文本 diff；binary diff/patch 作为下一阶段或明确降级行为。
- Verify:
  - 文档审阅 + `rg "thread_diff" docs -n`

### t2 - 抽象 Git Runner（实现）

- Files:
  - 新增 `crates/git/`（或放在 `crates/app-server` 内部模块，取决于耦合度）
  - `crates/app-server/src/main/thread_observe/disk_git_diff.rs`
- Acceptance:
  - 提供 `GitRunner` trait：`diff()`, `patch()`（以及可选 `clone/fetch/status`）。
  - 默认实现优先用内嵌 Git；如果明确允许也可提供 “shell git” 作为 fallback（或完全移除 fallback，按策略决定）。
- Verify:
  - 单元测试：在测试里模拟 “系统无 git” 时仍可生成 diff/patch。

### t3 - 网络/权限边界接入

- Files:
  - `crates/app-server/src/main/process_control/start.rs`（复用 `command_uses_network` 的判定语义）
  - `crates/core/src/modes.rs`（如需要新增 tool/capability 映射）
- Acceptance:
  - 网络型 Git 操作在 `sandbox_network_access=deny` 时 fail-closed。
  - 关键路径落盘可审计（事件含足够原因）。
- Verify:
  - 新增测试覆盖 deny 分支。

### t4 - Size baseline 与裁剪

- Files:
  - `docs/plans/embedded_git.md`
  - `Cargo.toml` feature flags（如需要）
- Acceptance:
  - 提供 `size` 复现命令（含平台/target/strip 说明）。
  - 给出明确的 feature 裁剪建议。
- Verify:
  - 本地跑 size 命令并记录输出到文档。

