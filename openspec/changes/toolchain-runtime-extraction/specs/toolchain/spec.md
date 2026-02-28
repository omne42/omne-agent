# 规格增量：toolchain（runtime extraction）

## 新增要求

### 要求：Toolchain 安装状态机必须归属 runtime 领域

`toolchain bootstrap` 的核心安装状态机必须由独立 runtime crate 实现。

#### 场景：执行 bootstrap

- 给定用户执行 `omne toolchain bootstrap --json`
- 当系统进入工具链补齐流程
- 则核心状态机由 `toolchain-runtime` 执行
- 且 CLI 层仅负责编排与输出

### 要求：CLI 层不得承载安装实现细节

`agent-cli` 的 `toolchain` 命令文件不得保留 public upstream 安装器实现。

#### 场景：边界扫描

- 给定源码仓库
- 当执行边界扫描命令
- 则 `crates/agent-cli/src/main/toolchain.rs` 中不包含下载/解压安装核心函数

### 要求：输出契约保持兼容

迁移后 `omne toolchain bootstrap --json` 的关键字段必须保持兼容。

#### 场景：JSON 输出兼容

- 给定迁移后的实现
- 当执行 `omne toolchain bootstrap --json`
- 则输出仍包含 `schema_version`、`target_triple`、`managed_dir`、`bundled_dir`、`items[*].tool/status/detail/source/destination`

## 变更要求

### 要求：npm postinstall 仍是薄转发

`packages/omne/scripts/postinstall-toolchain.mjs` 继续只调用 `omne toolchain bootstrap`。

#### 场景：npm 安装

- 给定执行 npm 安装
- 当触发 postinstall
- 则脚本只做 CLI 转发
- 且不实现独立安装状态机
