# Workspace Hooks（setup/run/archive）（v0.2.0 口径）

> 目标：把 workspace 生命周期脚本化为可版本化配置，并把执行结果落到 `process/*` 的可观测/可回放通道里（stdout/stderr artifacts）。
>
> v0.2.0 实现的是“最小可用”：按 argv 执行命令，不做 stdin/PTY 交互，执行仍受 `mode/sandbox/execpolicy/approvals` 约束。

---

## 0) 配置文件位置（写死）

在 thread cwd（workspace root）下的项目配置目录：

- `./.omne_data/spec/workspace.yaml`
- `./.omne_data/spec/workspace.yml`

初始化提示：

- `omne init` 默认会创建 `./.omne_data/spec/workspace.yaml` 的最小空模板（`hooks: {}` + 示例注释）。
- 若不希望自动生成，可使用 `omne init --no-workspace-template`。
- 若希望一次跳过所有 spec 模板（commands/workspace/hooks/modes），可使用 `omne init --no-spec-templates`（等价 `--minimal`）。

> 注意：这是项目配置（可提交）。运行时数据根目录也是 `./.omne_data/`；不要把 hooks 配置写进 `threads/`、`tmp/` 等运行时目录。

---

## 1) 配置格式（YAML）

最小 schema：

```yaml
hooks:
  setup: ["cargo", "--version"]
  run: ["cargo", "test", "--workspace"]
  archive: ["git", "status", "--porcelain=v1"]
```

说明：

- `hooks.<name>` 的值是 **argv 数组**（不会再经过 shell 拼接）。
- 建议避免 `bash -lc` / `sh -c` / `python -c` 这类“解释执行字符串”的入口（见 `docs/execpolicy.md` 的默认建议）。需要多步骤时，把逻辑写进仓库脚本文件并直接执行（更可审计、也更容易写规则）。
- 支持的 hook 名称：`setup` / `run` / `archive`。

---

## 2) 执行方式（CLI）

```bash
omne thread hook-run <thread_id> setup
omne thread hook-run <thread_id> run
omne thread hook-run <thread_id> archive
```

返回值（概要）：

- config 不存在：`{ ok: true, skipped: true, reason: "...", searched: [...] }`
- hook 未配置：`{ ok: true, skipped: true, reason: "...", config_path: "..." }`
- 成功启动：`{ ok: true, process_id, stdout_path, stderr_path, ... }`
- 需要审批：返回 `{ needs_approval: true, approval_id, thread_id, hook }`
  - CLI 会报错并提示：先 `omne approval decide ... --approve`，然后带 `--approval-id <id>` 重跑。
- 被拒绝：返回 `{ denied: true, thread_id, hook, error_code, detail }`（`error_code` 与 `process/start` 拒绝分类一致，如 `sandbox_policy_denied` / `allowed_tools_denied` / `mode_denied` / `approval_denied`）。

自动触发（v0.2.x 已落地最小版本）：

- `thread/start`：创建 thread 后会自动尝试运行 `setup`，返回体包含 `auto_hook`。
- `thread/archive`：归档 thread 时会自动尝试运行 `archive`，返回体包含 `auto_hook`。
- `thread/hook_run run`：`run` 仍保持显式手动触发。

`auto_hook` 结构（概要）：

- 成功启动：`{ hook, ok: true, process_id, stdout_path, stderr_path }`
- 跳过：`{ hook, ok: true, skipped: true, reason, ... }`
- 失败：`{ hook, ok: false, error }`

---

## 3) 与安全策略的关系（别绕开）

workspace hook 最终会走 `process/start`：

- 仍受 `allowed_tools`、hard boundary / config validation，以及进入策略合并后的 `mode gate → execpolicy → approval handling` 约束。
- 仍是非交互进程：hook 命令必须是非交互式（否则就会“卡住”，只能 `process/kill`）。

---

## 4) 快速自检（可复制）

```bash
# 在当前 repo 启动一个 thread（cwd=repo root），从输出里复制 thread_id
omne thread start --cwd . --json

# 运行 setup hook（需要你已经创建 `.omne_data/spec/workspace.yaml`）
omne thread hook-run <thread_id> setup --json
```
