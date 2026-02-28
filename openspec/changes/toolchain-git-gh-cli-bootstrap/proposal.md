# 提案：toolchain-git-gh-cli-bootstrap

## 相关文档

- `openspec/README.md`：OpenSpec 提案流程与文档约束。
- `openspec/specs/git-domain/implementation-roadmap.md`：Git 领域边界（runtime 归属）。
- `docs/workflow.md`：当前对 `git` 前置依赖的说明。
- `packages/omne/README.md`：Node launcher 与 vendor bundle 机制。

## 做什么

- 在安装阶段增加本机工具链探测：检测 `git` 与 `gh` 是否可用。
- 当本机缺失工具且安装包包含对应 feature 时，自动启用内置 CLI（`git-cli`、`gh-cli`）。
- 为发布产物增加 feature 元信息与可校验行为，确保“是否内置了 Git/GH”可追踪。

## 为什么做

- 目前全链路默认依赖用户机器已安装 `git`，会导致新用户首跑失败。
- Git 相关能力已是主链路（worktree/snapshot/patch/apply/lifecycle），缺失 `git` 会直接阻断核心流程。
- 需要把“环境依赖”从人工前置条件收敛为“安装期自动补齐 + 明确 feature 开关”。

## 怎么做

- 在 `packages/omne` 引入安装期脚本：
  - 探测本机 `git`/`gh` 可执行；
  - 若缺失且 vendor bundle 含 feature 对应二进制，则将其安装到受管工具目录；
  - 记录安装结果与失败原因（仅日志，不影响无关功能）。
- 在 launcher 注入 PATH 时加入受管工具目录，使 `omne`/`omne-app-server` 子进程可见该工具链。
- 在 vendor 构建脚本与 manifest 中增加 feature 字段：
  - `git-cli`、`gh-cli`；
  - 对应二进制是否存在要能被校验脚本验证。

## 非目标

- 不在本阶段实现跨平台包管理器集成（如 apt/brew/choco 自动安装系统包）。
- 不在本阶段改动 Git 领域 runtime 业务语义（仅工具链可用性改进）。
- 不把 Git 实现迁移到 `safe-fs-tools`。

## 验收标准

- 功能行为：
  - 当系统无 `git` 且安装包包含 `git-cli` feature 时，安装后可直接执行 Git 相关主链路。
  - 当系统无 `gh` 且安装包包含 `gh-cli` feature 时，`gh` 命令在运行时可被找到。
- 可观测性：
  - 安装日志可明确说明“本机已存在 / 已安装内置 / 缺失但无 feature”三种状态。
  - vendor manifest 能标识当前 bundle 包含的 feature。
- 边界约束：
  - `app-server` 不新增 Git 过程实现；Git 领域逻辑仍归属 `omne-git-runtime`。
