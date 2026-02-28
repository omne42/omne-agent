# 规格增量：toolchain（git/gh CLI 安装期引导）

## 要求：安装期必须探测 Git/GH 可用性

`packages/omne` 在安装生命周期中必须探测 `git` 与 `gh` 是否可执行，并输出可判定状态。

### 场景：本机已安装 git

- 给定 `git --version` 可执行
- 当执行安装期探测
- 则状态为 `present`
- 且不得覆盖本机现有 `git`

### 场景：本机缺失 git 且 bundle 含 git-cli feature

- 给定 `git --version` 不可执行
- 且 bundle manifest 标记包含 `git-cli`
- 当执行安装期探测
- 则必须把内置 `git` 安装到受管工具目录
- 且安装结果状态为 `installed_bundled`

### 场景：本机缺失 git 且 bundle 不含 git-cli feature

- 给定 `git --version` 不可执行
- 且 bundle manifest 未包含 `git-cli`
- 当执行安装期探测
- 则状态为 `missing_without_feature`
- 且必须给出可读告警

## 要求：运行时 PATH 必须包含受管工具目录

当受管工具目录存在时，launcher 在启动 `omne`/`omne-app-server` 前必须将该目录注入 PATH。

### 场景：受管目录存在

- 给定受管工具目录下存在 `git` 或 `gh`
- 当 launcher 构造子进程环境
- 则 PATH 包含该目录
- 且保留现有 PATH 顺序与兼容行为

## 要求：feature 与 bundle 文件必须一致

vendor manifest 的 feature 声明必须与实际文件布局一致，并可由校验脚本检测。

### 场景：manifest 含 git-cli feature 但缺少二进制

- 给定 manifest 声明 `git-cli`
- 且 `vendor/<target>/path/git[.exe]` 缺失
- 当执行 bundle 校验
- 则校验必须失败并给出明确错误

### 场景：manifest 含 gh-cli feature 但缺少二进制

- 给定 manifest 声明 `gh-cli`
- 且 `vendor/<target>/path/gh[.exe]` 缺失
- 当执行 bundle 校验
- 则校验必须失败并给出明确错误
