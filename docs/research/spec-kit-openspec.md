# Spec Kit vs OpenSpec（example/spec-kit + example/OpenSpec）对比与启发

> Snapshot:
> - `example/spec-kit` @ `6f523ed`
> - `example/OpenSpec` @ `d485281`
>
> 结论先行：两者都在做“Spec 驱动开发”，但主战场不同。`Spec Kit` 更像“从 0 到 1 的落地流水线生成器”，`OpenSpec` 更像“1 到 N 变更治理系统”。

---

## 1. 说人话：它们分别像什么

- `Spec Kit`：像“给团队装一条标准研发流水线”。
  - 核心是 `/speckit.constitution -> /speckit.specify -> /speckit.plan -> /speckit.tasks -> /speckit.implement`
  - 强项是快速把“想法”压成可执行任务，适合新功能快速起盘。
- `OpenSpec`：像“给已有系统装一个变更审计层”。
  - 核心是 `openspec/specs/`（当前真相）和 `openspec/changes/`（变更提案）分离
  - 强项是让需求变更可审阅、可归档、可追踪，适合持续演进的存量项目。

---

## 2. 共同点（为什么都值得学）

- 都把“先对齐规格，再写代码”作为默认流程。
- 都尽量用 Markdown/目录结构把流程外显，便于 review 和版本管理。
- 都强调与现有 AI 工具集成（命令/提示词/工作流文件），而不是要求团队换整套开发工具。

---

## 3. 核心差别（重点）

| 维度 | Spec Kit | OpenSpec |
| --- | --- | --- |
| 典型场景 | 新功能快速落地（0->1） | 存量系统持续变更（1->N） |
| 主体结构 | 一条“规格到实现”的线性流水线 | “当前规格”和“提案变更”双轨并存 |
| 治理能力 | 偏“执行导向” | 偏“变更治理/审计导向” |
| 产物重心 | 计划、任务、实现步骤 | proposal/tasks/spec delta/archive |
| 团队价值 | 快速推进，降低起步摩擦 | 控 scope、防漂移、可追溯 |

简化理解：
- 想把事情“做出来”，`Spec Kit` 更快。
- 想把事情“长期做稳”，`OpenSpec` 更强。

---

## 4. 对 OmneAgent 的直接启发（结合当前实现）

先看现状（我们已有）：
- 已有工作流命令运行时（`omne command list/show/run` + frontmatter + vars + fan-out/fan-in）。
- 已有事件日志、artifact、审批与观察链路。

当前短板：
- 缺少明确的“规格治理层”（尤其是 source-of-truth 与 change proposal 分离）。
- `omne init` 只建目录，不脚手架默认 command/spec 模板，新手起步成本高。

### 4.1 应该借鉴 Spec Kit 的部分

- 在 `omne init` 时直接生成可用模板：
  - `.omne_data/spec/commands/` 下放默认命令（如 `specify/plan/tasks/implement`）
  - 每个模板带 frontmatter、输入变量、验收输出格式
- 把“从需求到任务分解”的链路固定成最小闭环，减少每个团队重复造轮子。

### 4.2 应该借鉴 OpenSpec 的部分

- 在 `.omne_data/spec/` 引入双轨模型：
  - `specs/`：当前有效规格（真相）
  - `changes/`：提案、任务、spec delta、归档状态
- 增加生命周期命令：
  - `omne spec proposal`
  - `omne spec apply`
  - `omne spec archive`
- 给 spec/change 增加 schema 校验和状态查询，避免“文档像有、流程失控”。

### 4.3 推荐路线（务实版）

1. P0：`omne init` 生成默认命令模板（先解决可用性）。
2. P0：定义 `.omne_data/spec/{specs,changes}` 目录约定（先统一模型）。
3. P1：补 `proposal/apply/archive` 命令和 schema 校验（再补治理闭环）。
4. P1：在 UI/CLI 增加 change 状态视图（active/approved/archived）。

---

## 5. 一句话策略

`OmneAgent` 不需要二选一：执行层学 `Spec Kit`，治理层学 `OpenSpec`，组合成“既能快跑、又不失控”的 spec 系统。
