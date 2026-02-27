# Git 领域阶段交接模板

## 用途

- 用于每个阶段提交后写“可接力”的标准化交接信息。
- 目标是让下一位不依赖聊天上下文，直接继续推进最终目标。

## 最终目标（固定不变）

- 完整实现“类 Claude Code worktree”的专属 Git 领域全链路。
- `app-server` 仅承担编排与协议映射，不承载 Git 过程实现细节。
- Git 过程能力（snapshot / patch / auto-apply / lifecycle）统一在 runtime。

## 阶段交接卡（复制后填写）

### 1. 当前阶段状态

- 阶段：`<phase-id / 变更名>`
- 目标：`<本阶段做什么>`
- 当前状态：`未开始 | 进行中 | 已完成`
- 对应提交：
- `<commit-hash> <message>`

### 2. 本阶段已完成

- `<已完成事项 1>`
- `<已完成事项 2>`

### 3. 下一步（必须可直接执行）

1. `<下一步动作 1>`
   - 入口文件：`<path>`
   - 命令：`<command>`
2. `<下一步动作 2>`
   - 入口文件：`<path>`
   - 命令：`<command>`
3. `<下一步动作 3>`
   - 入口文件：`<path>`
   - 命令：`<command>`

### 4. 阻塞与风险

- 阻塞：`<无/有，具体说明>`
- 风险：`<风险点 + 影响范围 + 回退策略>`

### 5. 最近通过的验证命令

- `cargo fmt --all --check`
- `cargo check --workspace`
- `<本阶段新增测试命令 1>`
- `<本阶段新增测试命令 2>`
- `rg -n "Command::new\\(\"git\"\\)" crates/app-server/src/main crates/app-server/src/agent/tools/dispatch`

### 6. 架构边界核对（必须填写）

- `app-server` 新增/修改内容仅限：`<编排/映射/诊断>`
- Git 过程实现是否在 runtime：`是/否`
- 是否发现边界回流：`否/是（若是必须列出文件并阻断合并）`

### 7. 相关文档

- `openspec/changes/git-domain-sequence.md`
- `openspec/specs/git-domain/spec.md`
- `openspec/specs/git-domain/implementation-roadmap.md`
- `openspec/changes/git-domain-e2e-handoff-hardening/tasks.md`
