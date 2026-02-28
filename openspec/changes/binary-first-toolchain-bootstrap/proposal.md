# 提案：binary-first-toolchain-bootstrap

## 相关文档

- `openspec/changes/toolchain-git-gh-cli-bootstrap/proposal.md`：已落地的 npm 安装期补齐方案。
- `packages/omne/README.md`：Node launcher 与 vendor 分发机制。
- `docs/implementation_plan.md`：可执行程序为主的工程路线。
- `docs/TODO.md`：Node 侧定位（分发层而非核心逻辑层）。

## 做什么

- 将 `git`/`gh` 补齐逻辑从 npm `postinstall` 下沉到 `omne` 可执行程序。
- 提供 `omne toolchain bootstrap` 命令，支持直接在二进制发行包中完成工具链补齐。
- 让 npm `postinstall` 仅做轻量转发：调用 `omne toolchain bootstrap`，不再承载核心安装流程。

## 为什么做

- 最终交付目标是“可执行程序优先”，npm 只是分发壳，不应成为核心能力依赖点。
- 当前补齐逻辑主要在 Node 脚本中，导致脱离 npm 的二进制发行链路能力不完整。
- 将逻辑下沉到 Rust CLI 后，可统一行为、统一可观测性、统一测试入口。

## 怎么做

- 在 `crates/agent-cli` 增加 `toolchain` 子命令域：
  - `omne toolchain bootstrap` 执行探测与补齐；
  - 支持 `--json` 输出结构化结果，便于脚本与安装器消费。
- 运行时策略：
  - 优先检测系统 PATH 的 `git`/`gh`；
  - 缺失时从 bundled toolchain 目录读取并安装到受管目录；
  - 受管目录默认 `~/.omne/toolchain/<target>/bin`，支持环境变量覆盖。
- npm `postinstall` 简化为调用 `omne toolchain bootstrap`。

## 非目标

- 不在本阶段实现跨平台系统包管理器安装（apt/brew/choco）。
- 不在本阶段改动 Git runtime 业务逻辑。
- 不在本阶段引入新的远程服务依赖。

## 验收标准

- `binary-first`：
  - 在无 npm 环境下，直接执行二进制命令可完成 toolchain bootstrap。
- `npm-thin-wrapper`：
  - npm `postinstall` 可转发并展示 bootstrap 结果，不再内嵌核心复制逻辑。
- `可观测性`：
  - `--json` 输出包含每个工具的状态（present / installed / missing / failed）。
- `兼容性`：
  - 现有 vendor/path 分发流程与测试通过，不回归既有发布脚本行为。
