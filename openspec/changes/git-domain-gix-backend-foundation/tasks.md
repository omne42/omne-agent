# 任务：git-domain-gix-backend-foundation

## 相关文档与用途

- `openspec/changes/git-domain-gix-backend-foundation/proposal.md`：本阶段目标、边界与验收标准。
- `openspec/changes/git-domain-gix-backend-foundation/specs/git-domain/spec.md`：本阶段规格增量。
- `openspec/specs/git-domain/spec.md`：当前生效基线。
- `openspec/specs/git-domain/implementation-roadmap.md`：全链路目标与阶段顺序。
- `crates/git-runtime/src/lib.rs`：核心实现入口。
- `crates/app-server/src/agent/tools/dispatch/subagents_runtime_artifacts.rs`：编排边界检查重点。

## 1. 文档与规格

- [ ] 新增 spec delta，明确 `gix` 后端方向与 `fetch/pull` 支持边界。
- [ ] 更新全链路路线图，纳入“无系统 git 硬依赖”的目标与迁移阶段。
- [ ] 更新变更顺序，加入 gix backend 阶段。

## 2. Runtime 后端抽象

- [ ] 在 `omne-git-runtime` 增加后端抽象（`gix|cli`），并提供选择策略。
- [ ] 保持统一 API，不允许 app-server 旁路调用 git 进程完成领域逻辑。
- [ ] 对未迁移能力保留 runtime 内受控 fallback。

## 3. 功能迁移（本阶段最小可交付）

- [ ] 落地至少一条 `gix` 主链路实现（优先 `fetch/pull` 或仓库基础读操作）。
- [ ] 保证失败返回可诊断错误（包含 repo 路径、远端名/引用等关键上下文）。

## 4. 测试与验证

- [ ] `cargo test -p omne-git-runtime`
- [ ] `cargo test -p omne-app-server fan_out_result_writer_auto_apply`
- [ ] 边界扫描：`rg -n "Command::new\(\"git\"\)" crates/app-server/src`（不得新增命中）
- [ ] 能力扫描：`rg -n "fetch|pull|push|gix" crates/git-runtime/src/lib.rs`

## 5. 交接信息（离开前必须补全）

- [ ] 当前后端默认值与可配置项（环境变量/配置文件）
- [ ] 已迁移能力清单与未迁移能力清单
- [ ] 最近一次通过的测试命令与时间
- [ ] 下一步 1-3 个直接可执行任务
