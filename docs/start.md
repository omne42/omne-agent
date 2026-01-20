# 项目启动（CodePM / Rust）

## 需求草案（早期记录）

我希望实现这样的功能（仅 Rust）：

1. 创建本地的git服务
2. 注入仓库
3. 可以异步同时处理多个ai task，会在/tmp/{repo*name}*{session_id}中进行开发并保存到本地git。需要format和check和commit并提交pr
4. 使用一个ai服务来妥善的合并多个pr。我们需要有自己的agent。
   我们的重点是创建临时目录并让ai可以并发处理task。

传入的应该是prompt（规范和需求和目标），pr_name。并在完成时hook回主要处。

我们应该有一个架构师。他必须能负责完成所有任务拆分（早期骨架-必须先单进程实现的骨架，多线程的后期功能的添加。 保持高内聚低耦合的实现）。我们
需要学习每一个仓库中的任何优秀的设计。 我们还需要预留构思者（用于引导想法并总结要做出什么样的东西），构思审核者（用于质疑必要性，实现难度，是
否有竞品，竟品有没有值得学习的地方，有哪些功能应该使用社区库避免重复开发，基建如何选择，代码选择什么类型），架构师（规划任务），coder（多人进
行开发），前端美化师（专门修改css），review师（专门负责审核代码变更），builder（负责部署，运维）。 我们的软件的目的是完成code的全生命流程的构
建。完整的自动化实现全流程。 另外我们未来还会提供一个react的ui包，作为可选的前端组件选择，其质量很高，且ai适配，组件丰富。但这是一个未来
feature，仅占位即可。

所有的一次完整的修改必须经过format check(type/lint等) changelog记录 commit

---

## 当前进度（Phase 1：单进程骨架）

已落地最小 Rust workspace 与 CLI（`code-pm`），用于验证端到端链路：

- 本地 bare repo 注入（`repo inject`）
- `/tmp/{repo}_{session_id}/tasks/{task_id}/repo` 隔离工作区
- task：clone →（可选）`git apply` →（Rust repo）`cargo fmt`/`cargo check` → commit → push 分支
- merge：顺序合并多个分支回 `base`（默认 `main`）
- artifacts：`/tmp/.../session.json` + `result.json` + `tasks/<id>/task.json`，并保留每一步的 `checks.steps` 日志（`tasks/<id>/artifacts/*.log`、`merge/artifacts/*.log`、失败时的 `error.log`/`merge-error.log`），以及 `.code_pm/data/sessions/<id>/`

## 开发/试跑

初始化本地数据目录：

```bash
cargo run -p code-pm -- init
```

注入一个仓库（本地路径或远端 URL）：

```bash
cargo run -p code-pm -- repo inject <repo_path_or_url> --name <repo_name>
```

启动本地 Git Smart HTTP（Phase 2，可选）：

```bash
cargo run -p code-pm -- serve --addr 127.0.0.1:9417
```

此时可通过 HTTP 访问 bare repo（例如 clone/push）：

```bash
git clone http://127.0.0.1:9417/git/<repo_name>.git
git push http://127.0.0.1:9417/git/<repo_name>.git <branch>
```

生成 patch（示例：在源 repo 内）：

```bash
git diff > /tmp/change.patch
```

跑一次 session：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "your spec + requirements + goals" \
  --apply-patch /tmp/change.patch
```

可选：实时输出任务/合并进度（stderr）：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --apply-patch /tmp/change.patch \
  --stream-events
```

可选：以 JSON Lines（NDJSON）输出事件流（stderr，每行包含 `type` 字段）：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --apply-patch /tmp/change.patch \
  --stream-events-json
```

多任务并发（CLI 显式提供 task 列表，绕过 Phase 1 的模板 Architect）：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --apply-patch /tmp/change.patch \
  --task a:apply-patch \
  --task b:apply-patch \
  --max-concurrency 2
```

从 prompt 自动拆分 tasks（Phase 1：规则化；识别 checklist/编号/无序列表）：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "...\n- [ ] task A\n- [ ] task B\n" \
  --auto-tasks \
  --max-concurrency 2
```

多任务文件（JSON）：

```json
{
  "tasks": [
    { "id": "a", "title": "apply patch A" },
    { "id": "b", "title": "apply patch B", "description": "optional" }
  ]
}
```

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --apply-patch /tmp/change.patch \
  --tasks-file /tmp/tasks.json \
  --max-concurrency 2
```

分支命名约定：

- `ai/<pr_name>/<session_id>/<task_id>`

临时目录根路径：

- 默认：`/tmp`
- 覆盖：`CODE_PM_TMP_ROOT=/your/tmp/root`

数据目录根路径（`PmPaths` / `.code_pm`）：

- 默认：`<repo_root>/.code_pm`（`repo_root` 为当前目录的 git root；若不在 git repo 中则为当前目录）
- 覆盖：`--pm-root /path/to/.code_pm` 或 `CODE_PM_ROOT=/path/to/.code_pm`（相对路径会按 `repo_root` 解析）

可选：完成后执行 hook 命令（将通过环境变量拿到 session 上下文）：

```bash
cargo run -p code-pm -- run \
  --repo <repo_name> \
  --pr-name <pr_name> \
  --prompt "..." \
  --hook-cmd /usr/bin/env \
  --hook-arg bash --hook-arg -lc --hook-arg 'echo "$CODE_PM_SESSION_ID $CODE_PM_RESULT_JSON"'
```
