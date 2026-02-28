# 任务：toolchain-git-gh-cli-bootstrap

## 相关文档与用途

- `openspec/changes/toolchain-git-gh-cli-bootstrap/proposal.md`：本阶段做什么/为什么/怎么做/验收标准。
- `openspec/changes/toolchain-git-gh-cli-bootstrap/specs/toolchain/spec.md`：工具链 feature 与安装行为规范。
- `packages/omne/README.md`：Node launcher 与 vendor bundle 使用说明。
- `packages/omne/lib/launcher.js`：运行时 PATH 注入入口。
- `packages/omne/scripts/assemble-vendor.mjs`：vendor 组装入口。

## 1. 文档与规范

- [x] 完成 proposal（做什么/为什么做/怎么做/验收）。
- [x] 完成 spec delta（feature 字段、安装期行为、失败语义）。
- [x] 补 README 使用方式与 feature 说明。

## 2. 安装期能力

- [x] 新增安装期探测脚本：
  - [x] 检测 `git`、`gh` 是否在 PATH 可用。
  - [x] 缺失时尝试安装内置 CLI（前提：bundle 包含对应 feature）。
  - [x] 输出结构化/可读日志（已存在、已安装、缺失无 feature、安装失败）。
- [x] 将安装脚本接入 `packages/omne/package.json` 的安装生命周期。

## 3. 运行时可见性

- [x] launcher PATH 注入支持受管工具目录（安装期落盘目录）。
- [x] 保持现有 vendor/path prepend 兼容，不破坏已发布 bundle 行为。

## 4. vendor feature 与校验

- [x] `assemble-vendor` 支持 `git-cli` / `gh-cli` feature 输入与二进制落盘。
- [x] manifest 增加 feature 元信息（可被 `verify-vendor-bundle` 校验）。
- [x] `verify-vendor-bundle` 增加 feature→文件一致性校验。

## 5. 测试与验证

- [x] `npm --prefix packages/omne test`
- [x] 新增/更新测试覆盖：
  - [x] launcher PATH 行为（受管工具目录注入）。
  - [x] 安装期探测与 fallback 行为。
  - [x] manifest feature 校验行为。
- [x] 手动验证（Linux）：
  - [x] 模拟无 `git`/`gh` PATH 场景；
  - [x] 验证安装后受管目录出现内置 `git`，并由 launcher 注入 PATH。

## 6. 完成定义（DoD）

- [x] 无系统 `git` 的环境可完成 Git 主链路（前提：`git-cli` feature 已打包）。
- [x] feature 与二进制布局一致，校验命令能拦截错配 bundle。
- [x] 文档、脚本、测试三者一致，无“文档说有、产物没有”的偏差。
