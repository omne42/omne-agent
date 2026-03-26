# omne-agent

`omne-agent` 是一个 Rust agent workspace，当前承载：

- `omne-app-server`：Codex 风格的控制面与 agent loop
- `omne`：人类可用 CLI / TUI
- 线程、事件、artifact、approval、tool/process 等协议与运行时数据模型
- 项目级 `./.omne_data/` 目录约定

它不是通用 foundation 仓库，也不是 LLM provider SDK 仓库。更底层的 primitives 应继续进入 `omne-runtime`，更通用的应用侧 kit 应继续进入 `omne_foundation`。

## 文档入口

这个仓库采用 agent-first 的文档系统。先看这些文件：

- `AGENTS.md`
- `docs/README.md`
- `docs/docs-system-map.md`
- `docs/start.md`
- `docs/v0.2.0_parity.md`
- `docs/reports/domain-boundary-audit-20260304.md`

## 最低验证

```bash
./scripts/check-docs-system.sh
cargo test --workspace
```
