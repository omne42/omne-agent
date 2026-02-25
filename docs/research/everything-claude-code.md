# Everything Claude Code（example/everything-claude-code）能力与启发

> Snapshot: `example/everything-claude-code` @ `7daa830`
>
> 结论先行：这是一个“高密度 Agent 资产库”（agents/commands/skills/hooks/rules），不是底层执行引擎。  
> 对 `OmneAgent` 最有价值的是：**默认模板体系 + hook 治理 + 资产校验流水线 + 会话学习闭环**。

---

## 1. 这个仓库到底是什么

从仓库结构和 README 看，它核心是可复用的工作流资产包：

- `agents/`：13 个角色代理
- `commands/`：32 个命令模板
- `skills/`：44 个技能目录
- `hooks/`：完整生命周期 hook 配置
- `rules/`：分语言规范（common/typescript/python/golang）

可验证信息：
- “13 agents / 43 skills / 31 commands” 对外说明：`example/everything-claude-code/README.md`
- 插件清单与入口：`example/everything-claude-code/.claude-plugin/plugin.json`
- OpenCode 版本映射：`example/everything-claude-code/.opencode/opencode.json`

---

## 2. 对 OmneAgent 有效的研究信息（重点）

### 2.1 命令模板可直接借鉴（最快落地）

这套命令已经形成“计划 -> 实施 -> 校验 -> 复盘”闭环：

- `plan`：先计划、必须用户确认再动代码  
  见 `commands/plan.md`（明确 `WAIT for user CONFIRM`）
- `tdd`：强制 RED/GREEN/REFACTOR，覆盖率门槛 80%+
- `orchestrate`：多代理串联 + handoff 文档格式
- `verify` / `checkpoint` / `eval`：质量门禁与阶段性检查

对 `OmneAgent` 的启发：
- `omne init` 时直接生成一组“可跑的默认命令模板”，降低首日门槛。
- 把命令模板和角色绑定（如 `plan->planner`, `verify->builder/reviewer`）。

### 2.2 Hook 治理层很实用（不是花架子）

`hooks/hooks.json` 覆盖了：
- `PreToolUse`：可阻断高风险或不规范动作
- `PostToolUse`：自动格式化、类型检查、提醒
- `SessionStart/End`：会话上下文加载和持久化
- `Stop/PreCompact`：收尾检查与压缩前处理

并且配有 schema + 校验脚本：
- schema：`schemas/hooks.schema.json`
- 校验：`scripts/ci/validate-hooks.js`（包含 inline JS 语法校验）

对 `OmneAgent` 的启发：
- 把 hook 从“经验配置”升级为“有 schema 的产品能力”。
- 对 hook 配置接入 CI 校验，避免线上才发现坏规则。

### 2.3 内容资产有 CI gate（长期维护关键）

它不是只放 Markdown，而是给资产做了自动检查：

- `validate-agents.js`
- `validate-commands.js`
- `validate-rules.js`
- `validate-skills.js`
- `validate-hooks.js`

统一由 `package.json` 的 `test` 脚本串起来。

对 `OmneAgent` 的启发：
- 我们的 `.omne_data/spec/commands`、roles、rules 也应有 lint/validate。
- “命令引用不存在 agent/skill”这类错误应在 CI 阶段就阻断。

### 2.4 会话沉淀与学习闭环可迁移

仓库实现了三层能力：

1. 会话摘要持久化（session-start/session-end hooks）
2. session alias 管理（便于恢复上下文）
3. `continuous-learning-v2`：observations -> instincts -> evolve(命令/技能/agent)

关键文件：
- `scripts/hooks/session-start.js`
- `scripts/hooks/session-end.js`
- `scripts/lib/session-manager.js`
- `skills/continuous-learning-v2/SKILL.md`

对 `OmneAgent` 的启发：
- 我们已有事件日志与 artifacts，可进一步产出“经验模式（instinct）”层。
- 先做“建议型学习”而非“自动改写规则”，防止模型漂移引入噪声。

### 2.5 跨生态打包方式值得学

同一套资产不仅支持 Claude，还提供 `.opencode` 对接层：
- `agent`、`command`、`instructions`、`plugin` 的集中配置
- 命令到子代理(`subtask`)的映射比较清晰

对 `OmneAgent` 的启发：
- 未来可把我们的 spec/workflow 变成“可导出包”，在不同宿主复用。
- 同一套 spec 资产，运行在不同 runtime 时只替换适配层。

---

## 3. 不能照搬的地方（避免踩坑）

- 路径强绑定 `~/.claude/*`，直接搬会与 `OmneAgent` 目录语义冲突。
- 部分约束是 prompt 级规范，不是硬执行策略；需要 runtime policy 兜底。
- 某些组织偏好（如拦截新建随机 `.md`）未必适合所有项目默认开启。

---

## 4. 对 OmneAgent 的建议落地顺序

### P0（短平快）

1. `omne init` 生成默认命令模板：`plan/tdd/orchestrate/verify/eval`
2. 增加 `hooks` 配置与 schema 校验命令
3. 对 spec 资产加 CI gate（至少检查存在性与引用完整性）

### P1（中期）

1. 增加会话摘要 + alias + 续作机制
2. 建立 `observations -> instincts` 存储结构
3. 提供 `instinct status/evolve` 的只读预览命令

### P2（长期）

1. 跨 runtime 资产打包（类似 `.opencode` 适配）
2. 把规范资产发布成版本化包，并支持项目级覆盖

---

## 5. 一句话总结

`everything-claude-code` 给我们的核心价值不是“替代引擎”，而是“把 Agent 团队经验产品化”的方法论：  
**模板化工作流 + 可验证治理 + 可沉淀学习 + 可移植资产层。**
