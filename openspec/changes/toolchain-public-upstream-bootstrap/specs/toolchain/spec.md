# 规格增量：toolchain（public upstream bootstrap）

## 要求：bootstrap 必须支持公共上游安装

当系统 PATH、managed 目录、bundled 目录都不可用时，
`omne toolchain bootstrap` 必须尝试从公共上游完成安装。

### 场景：进入 public upstream 阶段

- 给定 PATH 找不到目标工具
- 且 managed 目录无已安装副本
- 且 bundled 阶段不可用或失败
- 当执行 `omne toolchain bootstrap --json`
- 则系统进入 `public upstream` 安装阶段
- 且输出中包含来源类型与诊断信息

## 要求：默认来源不得依赖私有服务器

默认配置下，下载地址只能来自公共上游资源。

### 场景：默认配置安装

- 给定未设置任何镜像相关环境变量
- 当执行 `omne toolchain bootstrap --json`
- 则安装来源仅使用官方公共上游
- 且不会引用私有服务器域名

## 要求：镜像候选必须可配置并可回退

系统必须允许通过配置追加公共镜像候选，并按顺序回退。

### 场景：主上游不可达

- 给定主上游连接失败
- 且已配置镜像候选前缀列表
- 当执行 `omne toolchain bootstrap --json`
- 则系统按配置顺序尝试镜像候选
- 且在成功后输出最终命中的来源地址

## 要求：下载产物必须可校验

从公共上游下载的产物必须执行 checksum 校验，校验失败不得安装。

### 场景：checksum 不匹配

- 给定下载内容与 checksum 清单不一致
- 当执行 `omne toolchain bootstrap --json`
- 则该工具状态为失败
- 且输出包含 checksum mismatch 诊断

## 要求：状态输出必须可枚举

`--json` 输出中的每个工具状态必须是明确枚举值，并区分来源阶段。

### 场景：public upstream 安装成功

- 给定 PATH 无工具，bundled 阶段不可用
- 且 public upstream 安装成功
- 当执行 `omne toolchain bootstrap --json`
- 则目标工具状态为 `installed_public`
- 且 `source` 字段包含命中的公共来源地址

## 要求：npm 仍然是薄转发层

`packages/omne/scripts/postinstall-toolchain.mjs` 继续只转发 CLI 命令，
不得承载独立安装状态机。

### 场景：npm postinstall

- 给定 npm 安装 `@omne/omne`
- 当触发 postinstall
- 则脚本调用 `omne toolchain bootstrap`
- 且不新增 npm 侧下载/解压核心逻辑
