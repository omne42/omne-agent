# omne-agent AGENTS Map

这个文件只做导航。稳定事实写在 `README.md` 和 `docs/`。

## 先看哪里

- 外部概览：`README.md`
- 文档入口：`docs/README.md`
- 文档系统地图：`docs/docs-system-map.md`
- 当前起点与范围：`docs/start.md`
- 实现对齐清单：`docs/v0.2.0_parity.md`
- 运行时数据与目录：`docs/omne_data.md`、`docs/runtime_layout.md`
- 模式 / 审批 / 权限：`docs/modes.md`、`docs/approvals.md`、`docs/permissions_matrix.md`
- 模型与工具接入：`docs/model_routing.md`、`docs/mcp.md`
- 边界审计：`docs/reports/domain-boundary-audit-20260304.md`

## 仓库地图

- `crates/app-server/`
  - 控制面、agent loop、tool dispatch、JSONL 事件落盘/回放。
- `crates/agent-cli/`
  - 人类可用 CLI / TUI。
- `crates/app-server-protocol/`、`crates/agent-protocol/`
  - 协议类型与导出产物。
- `crates/eventlog/`
  - append-only JSONL event log 与派生视图。
- `crates/core/`
  - 存储、路径、模式、脱敏和通用运行时能力。
- `docs/`
  - 版本化记录系统。

## 修改规则

- `AGENTS.md` 保持短小；长期事实写回 `docs/`。
- 运行时、协议或目录布局变化时，同步更新对应专题文档。
- 如果调研或报告修正了事实，要把结论回写到专题文档，不要只留在 `docs/reports/` 或 `docs/research/`。
- `README.md`、`AGENTS.md`、`docs/README.md`、`docs/docs-system-map.md` 必须互相指向。

## 协作安全

- 不要改写共享分支历史；避免 `git rebase` / `git push --force`。
- 只暂存当前任务相关文件，避免把别人的脏改动一起带走。
- 不要使用 `git reset --hard`、`git checkout -- .`、`git clean -fdx` 这类破坏性命令。
- 默认不要碰 `target/`、`tmp/`、`reviewctx.*` 等临时目录。
- `example/` 仅作参考，不把它当作 CI 或实现依赖。

## 验证

- `./scripts/check-workspace.sh [local|ci|docs-system]`
- `./scripts/check-docs-system.sh`
- `cargo fmt --all`
- `cargo check --workspace --all-targets`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
