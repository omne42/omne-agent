# Commit 规则（强制）：Conventional Commits + Changelog 绑定 + Rust gates

本项目的所有提交必须满足三类约束：

1. **Commit message 必须是标准格式（Conventional Commits）**
2. **每次提交必须同时包含 `CHANGELOG.md` 与实际变更**（禁止 changelog-only / non-changelog commit）
3. **提交前必须通过 Rust gates**（format/type check + 单文件行数上限；不通过会拒绝提交）

> 说明：本规则通过仓库内置的 `githooks/` 强制执行（`pre-commit` + `commit-msg`）。`git commit --no-verify` 可以绕过客户端 hooks，但在我们的自动化流水线（AI Coder/CI/本地 git 服务端 hook）里同样会做强校验，因此不应依赖绕过。

---

## 1) Commit message 标准格式

采用 Conventional Commits：

```
<type>(<scope>)!: <subject>
```

- `<type>`：必填，且必须在允许列表内
- `<scope>`：可选，建议使用模块名/目录名（仅允许 `a-z0-9._-`）
- `!`：可选，表示 breaking change
- `<subject>`：必填，简明描述变更（建议英文/中文均可，但不要为空）

### 允许的 type

- `feat`：新功能
- `fix`：修复 bug
- `docs`：文档
- `refactor`：重构（不改变外部行为）
- `perf`：性能优化
- `test`：测试
- `chore`：杂项（依赖、脚本、清理等）
- `build`：构建系统/打包
- `ci`：CI/工作流
- `revert`：回滚

### 示例

- `feat(omne): concurrent task workspaces`
- `fix(git): prevent push race with repo lock`
- `docs(research): add codex responses notes`
- `refactor(core)!: split orchestrator state`

### 特殊提交（自动放行）

以下类型的提交信息默认放行（避免阻断 git 自带流程）：

- `Merge ...`
- `Revert "..."`（git 自动生成）
- `fixup! ...` / `squash! ...`（rebase 工作流）

---

## 2) 提交 gate：CHANGELOG + Rust gates

### CHANGELOG 规则（强制）

- 每次提交必须包含 `CHANGELOG.md`，并在 `[Unreleased]` 下追加条目
- 禁止只提交 `CHANGELOG.md`（changelog-only commit）
- 默认禁止修改已发布版本的 changelog 段落（例如 `## [0.x.y]` 及以下内容）
  - 发布时如需改动已发布段落，可显式设置：`OMNE_ALLOW_CHANGELOG_RELEASE_EDIT=1 git commit ...`

### Rust 单文件行数上限（强制）

- 仓库内任意 `.rs` 文件行数不得超过 1000 行（以本次提交的 git index 为准）
- 超过会拒绝提交，并提示拆分模块/降低复杂度

在 `git commit` 前必须通过：

- 格式：`cargo fmt --all -- --check`
- 类型/编译：`cargo check --workspace --all-targets`

若失败：

- hooks 会打印错误原因并退出（commit 被拒绝）
- 你需要先修复再重新提交

---

## 3) 启用 hooks（一次性设置）

本仓库将 hooks 存在 `githooks/` 目录中（可被 git 跟踪）。需要在本地设置一次：

```bash
./scripts/setup-githooks.sh
```

等价于：

```bash
git config core.hooksPath githooks
chmod +x githooks/*
```
