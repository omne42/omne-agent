# 规格增量：toolchain（binary-first bootstrap）

## 要求：bootstrap 必须由可执行程序提供

系统必须提供 `omne toolchain bootstrap` 命令，作为工具链补齐的主入口。

### 场景：命令可独立运行

- 给定仅有 `omne` 可执行程序（无 npm）
- 当执行 `omne toolchain bootstrap`
- 则系统能够完成工具链探测与补齐流程
- 且输出可读结果

## 要求：bootstrap 状态必须结构化

`--json` 输出必须包含每个工具（`git`/`gh`）的状态，且状态可枚举。

### 场景：工具已存在

- 给定 PATH 可找到 `git`
- 当执行 `omne toolchain bootstrap --json`
- 则 `git` 状态为 `present`

### 场景：工具缺失且可从 bundled 安装

- 给定 PATH 找不到 `git`
- 且 bundled feature 含 `git-cli`
- 当执行 `omne toolchain bootstrap --json`
- 则 `git` 状态为 `installed_bundled`

### 场景：工具缺失且未提供 feature

- 给定 PATH 找不到 `git`
- 且 bundled feature 不含 `git-cli`
- 当执行 `omne toolchain bootstrap --json`
- 则 `git` 状态为 `missing_without_feature`

## 要求：npm postinstall 仅为转发层

`packages/omne/scripts/postinstall-toolchain.mjs` 必须以调用 `omne toolchain bootstrap` 为主，
不得再承载独立的核心安装状态机。

### 场景：postinstall 执行

- 给定 npm 安装 `@omne/omne`
- 当触发 postinstall
- 则脚本调用 `omne toolchain bootstrap`
- 且转发输出结果
