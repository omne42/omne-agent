# 规格增量：git-domain（gix backend foundation）

## 新增要求

### 要求：Git runtime 必须支持 gix 后端

`omne-git-runtime` 必须提供后端抽象，并支持以 `gix` 作为 Git 领域实现后端。

#### 场景：后端选择为 gix

- 给定 runtime 后端配置为 `gix`
- 当执行已迁移的 Git 领域能力
- 则调用 `gix` 路径完成处理
- 且不要求系统存在 `git` 可执行文件

### 要求：fetch 与 pull 能力纳入当前支持范围

Git 领域必须把 `fetch` 与 `pull` 作为可用能力纳入规划与实现路径。

#### 场景：fetch 可执行

- 给定仓库存在可访问远端
- 当执行 runtime 的 fetch 能力
- 则获取远端更新并返回可诊断结果

#### 场景：pull 可执行

- 给定仓库可 fast-forward 到已获取状态
- 当执行 runtime 的 pull 能力
- 则完成“获取并更新工作区到目标状态”的主链路

### 要求：push 不作为本阶段承诺能力

`push` 不纳入本阶段的对外交付承诺，不得被误标为“已支持”。

#### 场景：能力边界可见

- 给定系统输出能力声明或文档
- 当检索 `push` 状态
- 则明确其不在本阶段承诺范围

## 变更要求

### 要求：Git 实现边界继续收敛到 runtime

`app-server` 不得新增 Git 过程实现；Git 领域逻辑继续由 `omne-git-runtime` 承担。
