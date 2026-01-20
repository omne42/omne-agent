# Superset（example/agent-gui/superset）能力与设计分析

> Snapshot: `example/agent-gui/superset` @ `8d1737342f77`
>
> 结论先行：Superset 把“并行跑 10+ CLI agent”这件事讲清楚了：**worktree 只是隔离代码，真正难的是隔离外部资源与环境**。它用 `.superset/config.json` 声明 `setup/teardown` 生命周期脚本，并在脚本里用 workspace 名做 key，自动创建/销毁 Neon branch、起/停 ElectricSQL 容器、写 workspace 专属 `.env`（含端口映射）。对 `CodePM vNext` 来说，这就是 RTS 风格需要的“单位补给线”：没有脚本化生命周期，你的并发只会制造混乱。

---

## 1) 产品定位：终端优先的“多任务调度台”

`example/agent-gui/superset/README.md` 明确定位：

- “A Terminal Built for Coding Agents”
- 同机并行运行多个 CLI agents（Claude Code/Codex 等）
- 每个并行 task 一个 git worktree
- UI 负责切换、通知、diff view（带 editor）

它强调“Superset 是你现有工具的超集”：worktree 可以继续用 IDE/文件系统/外部终端打开。这个定位很现实：**调度层不需要替代 IDE**，只需要把并发与收口做好。

---

## 2) Workspace 生命周期：`.superset/config.json` + scripts

配置文件极简，只做两件事：

```json
{
  "setup": ["./.superset/setup.sh"],
  "teardown": ["./.superset/teardown.sh"]
}
```

实现证据：`example/agent-gui/superset/.superset/config.json`。

对 `CodePM`：这就是我们需要的“生命周期接口”最小形态。别把环境准备塞进 prompt 或 UI 手册里，**写成脚本，变成契约**。

---

## 3) `setup.sh`：把外部资源也 worktree 化

### 3.1 关键输入（环境变量与命名）

`setup.sh` 依赖两个核心概念：

- `SUPERSET_ROOT_PATH`：根 repo 路径（用于读取根 `.env` 并复制到 workspace）。
- `SUPERSET_WORKSPACE_NAME`（可选）：默认用 `$(basename "$PWD")` 当 workspace 名（用于 Neon branch / Docker container 命名）。

实现证据：`example/agent-gui/superset/.superset/setup.sh`。

### 3.2 依赖检查与“可读的失败”

它会检查并提示安装：

- `bun`、`neonctl`、`jq`、`docker`

并在最后输出一份 “Setup Summary”（列出 skipped/failed steps），这比“脚本默默失败”靠谱得多。

### 3.3 Neon：每个 workspace 一个 DB branch

核心流程：

- `neonctl branches list` 查是否已存在同名 branch
- 不存在则 `neonctl branches create --name "$WORKSPACE_NAME"`
- 取 connection strings（pooled + direct）
- 写入 `.env`：`NEON_BRANCH_ID`、`DATABASE_URL`、`DATABASE_URL_UNPOOLED`

实现证据：`example/agent-gui/superset/.superset/setup.sh`。

这点是关键洞察：**并行 agent 的冲突经常不是代码冲突，而是共享数据库/端口/缓存导致的“环境冲突”**。

### 3.4 ElectricSQL：每个 workspace 一个容器 + 动态端口

脚本会：

- 用 workspace 名生成 container name（并做 sanitize + 长度限制）
- `docker run -p 3000` 让宿主机端口自动分配（避免冲突）
- `docker port` 反查实际端口
- health check 等待 ready
- 写入 `.env`：`ELECTRIC_CONTAINER`、`ELECTRIC_PORT`、`ELECTRIC_URL`、`ELECTRIC_SECRET`

实现证据：`example/agent-gui/superset/.superset/setup.sh`。

对 `CodePM`：如果我们要 RTS 风格跑多 workspace，“端口/外部资源隔离”必须是第一等需求，不是之后再补的优化。

---

## 4) `teardown.sh`：收尾同样要自动化

`teardown.sh` 做了两件必须做的事：

1. 停掉并删除 workspace 对应的 Electric 容器
2. 删除 Neon branch（用 `.env` 里的 `NEON_BRANCH_ID`）

实现证据：`example/agent-gui/superset/.superset/teardown.sh`。

对 `CodePM`：RTS 风格意味着你会开很多“临时单位”。没有 teardown，你就是在给自己制造长期垃圾与隐性成本（外部资源、容器、端口占用、云账单）。

---

## 5) Cookbook：把并行开发变成 pipeline（而不是祈祷）

`example/agent-gui/superset/docs/cookbook/README.md` 的 workflow 本质是：

1. 用高推理模型做 plan（作者偏好 Codex high）
2. 交给擅长写码的 agent 实现（如 Claude Code）
3. 用 review agent 或推理模型审（如 CodeRabbit/Codex）
4. 把反馈回传给 coding agent
5. 循环直到满意

它也强调两条硬建议：

- worktree 要用，但**setup 要自动化**
- 端口要用 env-based mapping，避免冲突

对 `CodePM`：这套 pipeline 就是“SLG/回合制”的基础节奏；RTS 的差异只是你同时跑更多回合，但**每个回合仍必须可观测、可暂停、可收口**。

---

## 6) 对 CodePM vNext 的启示（只取精华）

1. **Workspace 生命周期必须脚本化**：至少要有 `setup/run/teardown(archive)` 三段。
2. **外部资源隔离优先于 git**：DB/端口/容器/缓存是并发的真实冲突源。
3. **脚本要能给出“可读的失败摘要”**：失败原因要落盘并可回放（别靠人盯终端）。
4. **把并行开发建模成 pipeline**：plan → implement → review → fix → merge（并行只是吞吐，不是流程）。

