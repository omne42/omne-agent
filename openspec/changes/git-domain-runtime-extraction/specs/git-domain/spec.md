# 规格增量：git-domain

## 新增要求

### 要求：Runtime Patch 抓取 API

`omne-git-runtime` 必须提供可复用 API，支持按字节与超时限制抓取工作区 patch 文本。

#### 场景：patch 包含未跟踪文件

- 给定工作区存在未跟踪文件
- 当执行抓取 API
- 则会以 best-effort 方式执行 `git add --intent-to-add`
- 且生成的 patch 可以包含这些文件

### 要求：Runtime Patch 应用 API

`omne-git-runtime` 必须提供可复用 API，支持通过 stdin 传入 patch 内容并执行 `git apply`。

#### 场景：失败时包含命令上下文

- 给定 `git apply --check` 执行失败
- 当 runtime API 返回错误
- 则错误信息应包含命令与 cwd 上下文，便于排查

## 变更要求

### 要求：App-Server 作为编排层

App-server 在子代理隔离工作区 patch 抓取/应用路径上，必须把 Git 子进程执行委托给 runtime API。
