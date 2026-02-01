# Reference Repo / Snapshot（只读参考）（v0.2.0）

> 目标：给 agent 一个“只读参考仓库”，用于 `file/read|glob|grep` 补充上下文，但**不参与 resume、不参与索引、不参与 patch 交付**。

当前状态：v0.2.0 已实现最小闭环（显式 `root="reference"`）。

---

## 0) 关键约束（写死）

- **只读**：reference repo 只允许 `read/search`，禁止任何 `edit/command/process`。
- **不可见/可忽略**：用户可以完全不知道 reference repo 的存在；它不应该污染 thread 的事件历史与对话上下文（除非显式引用）。
- **不参与 resume**：`resume` 只能依赖 thread 的落盘事件 + artifacts；reference repo 缺失时不影响正确性。
- **不参与交付**：reference repo 里的内容不产生 patch/diff，不作为交付结果的一部分。
- **默认关闭**：未显式配置时，不应读取/扫描 reference repo（避免“隐式依赖”）。
- **不制造隐性依赖**：任何来自 reference repo 的内容，一旦被用于模型上下文/结论推导，必须被物化为 artifacts（脱敏后的摘录/摘要），保证 resume 不依赖 reference repo 的存在。

---

## 1) 最小接口（已实现）

reference repo 只提供两类能力：

- `file/read`：读取指定路径文件（只读）
- `file/glob` / `file/grep`：搜索/匹配（只读）

实现上可以把 reference repo 当成一个额外的只读 root（独立于 thread cwd/workspace root）。

v0.2.0 口径（写死）：

- reference repo 的读取必须是 **显式** 的：在 `file/read|glob|grep` 参数里使用 `root: "workspace"|"reference"`（默认 `workspace`）。
- unknown root / 未配置 reference repo 时：fail-closed（直接报错）。

---

## 2) 存储位置（已实现）

按 project 共享（不是按 thread）：

```
<agent_root>/
  reference/
    repo/          # 清理后的工作树（无 .git）
    manifest.json  # 清理结果（跳过了哪些文件、总大小等）
```

注意：这只是建议布局；关键是“只读 + 不参与 resume/交付”。

---

## 3) 清理规则（已实现）

拉取/复制 reference repo 后必须做清理：

- 移除 `.git/`（导入时不复制）
- 删除单文件 `> 10MB`（默认 10MB；导入时跳过）
- 默认排除敏感文件（建议写死与 checkpoint 同口径）：
  - `.env`、`.env.*`、`*.pem`、`*.key`、`.ssh/**`、`.aws/**`、`.kube/**`
- 生成 `manifest.json`（至少包含：跳过了哪些路径/原因、最终统计、时间戳）

清理结果应该可被用户查看（例如作为 user artifact 写入一个 markdown 报告），但不强制进入事件历史。

---

## 4) 风险提示（别自欺欺人）

- reference repo 不是安全边界：它只是一棵只读目录树；真正的安全来自 sandbox/mode/execpolicy。
- 不要把 reference repo 做成“隐藏依赖”：一旦你让 agent 的正确性依赖它，resume 就会变成赌博。

---

## 5) DoD

- 未配置 reference repo 时：
  - `root="reference"`（或 `ref/*`）的调用必须直接报错，不得隐式回退到 workspace。
- 配置并加载 reference repo 后：
  - `file/read|glob|grep` 的 reference 访问始终只读，且受 sandbox 的路径逃逸检查（拒绝 `..` 与 symlink escape）。
  - reference repo 的缺失/损坏不会影响 thread 的 replay/resume（最多让 reference 查询失败）。
  - 如果 reference 内容被注入上下文：必须生成对应的“摘录 artifact”（脱敏），并在后续 turn 的构建中只引用该 artifact（而不是再次读取 reference repo）。

---

## 6) 用法

导入一个本地目录为 reference repo：

```bash
omne-agent reference import /path/to/repo --force
```

查看当前 reference repo 状态：

```bash
omne-agent reference status
```
