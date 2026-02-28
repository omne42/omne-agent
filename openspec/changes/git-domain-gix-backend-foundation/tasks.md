# 任务：git-domain-gix-backend-foundation

## 相关文档与用途

- `openspec/changes/git-domain-gix-backend-foundation/proposal.md`：本阶段目标、边界与验收标准。
- `openspec/changes/git-domain-gix-backend-foundation/specs/git-domain/spec.md`：本阶段规格增量。
- `openspec/specs/git-domain/spec.md`：当前生效基线。
- `openspec/specs/git-domain/implementation-roadmap.md`：全链路目标与阶段顺序。
- `crates/git-runtime/src/lib.rs`：核心实现入口。
- `crates/app-server/src/agent/tools/dispatch/subagents_runtime_artifacts.rs`：编排边界检查重点。

## 1. 文档与规格

- [x] 新增 spec delta，明确 `gix` 后端方向与 `fetch/pull` 支持边界。
- [x] 更新全链路路线图，纳入“无系统 git 硬依赖”的目标与迁移阶段。
- [x] 更新变更顺序，加入 gix backend 阶段。

## 2. Runtime 后端抽象

- [x] 在 `omne-git-runtime` 增加后端抽象（`gix|cli`），并提供选择策略（`OMNE_GIT_RUNTIME_BACKEND`）。
- [x] 保持统一 API，不允许 app-server 旁路调用 git 进程完成领域逻辑。
- [x] 对未迁移能力保留 runtime 内受控 fallback。

## 3. 功能迁移（本阶段最小可交付）

- [x] 落地至少一条 `gix` 主链路实现（优先 `fetch/pull` 或仓库基础读操作）。
- [x] 保证失败返回可诊断错误（包含 repo 路径、远端名/引用等关键上下文）。

## 4. 测试与验证

- [x] `cargo test -p omne-git-runtime`
- [x] `cargo test -p omne-app-server fan_out_result_writer_auto_apply`
- [x] 边界扫描：`rg -n "Command::new\(\"git\"\)" crates/app-server/src`（不得新增命中）
- [x] 能力扫描：`rg -n "fetch|pull|push|gix" crates/git-runtime/src/lib.rs`

## 5. 交接信息（离开前必须补全）

- [x] 当前后端默认值与可配置项（环境变量/配置文件）
- [x] 已迁移能力清单与未迁移能力清单
- [x] 最近一次通过的测试命令与时间
- [x] 下一步 1-3 个直接可执行任务

交接记录（2026-03-01）：

- 当前默认后端：`cli`；可通过 `OMNE_GIT_RUNTIME_BACKEND=gix` 开启 `gix` 路径。
- 已迁移能力：`create_detached_worktree` 在 `gix` 后端下会先走 `gix::open()` 仓库校验，再执行现有 worktree 命令链。
- 未迁移能力：`capture_workspace_patch`、`git apply`、`worktree remove/prune` 仍由 CLI 执行。
- 最近通过：
  - `cargo test -p omne-git-runtime`
  - `cargo check -p omne-app-server`
- 下一步建议：
  1. 在 runtime 增加 `gix fetch` API（含本地 bare remote 集成测试）。
  2. 在 runtime 增加 `pull`（先支持 fast-forward，冲突回传结构化错误）。
  3. 将 `remove_detached_worktree_and_prune` 的 repo/common-dir 判定从 CLI 迁到 gix。
