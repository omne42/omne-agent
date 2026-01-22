# Checkpoints / Rollback（TODO：规格草案）

> 目标：当 agent 走偏（坏改动/误操作/循环）时，用户能把 workspace 回到一个“稳定点”，并继续推进，而不是只能重开线程。
>
> 状态：本文是 **TODO 规格草案**（v0.2.0 未实现）。

---

## 0) 核心原则（别把历史删了）

- 事件流仍然 **append-only**：rollback 不会删除历史事件；它只是产生新的事件，表达“我回到了某个点”。
- 回滚的对象是 **workspace 状态**（文件树），不是“模型记忆”。
- 最小可用先解决一个问题：**让用户能回到一个可复现的文件树**，并能在此基础上继续 turn。
- v0.2.0 现状没有 workspace 隔离：如果多个 threads 指向同一个 `cwd`，restore 会影响所有这些 threads（并发写入本来就不安全；见 `docs/subagents.md` 的边界说明）。

---

## 1) 范围与非目标

范围（最小可用）：

- 支持创建一个 checkpoint（稳定点）。
- 支持从 checkpoint restore（回滚）。
- 能在事件/产物里定位：checkpoint 是什么时候创建的、对应哪个 turn、恢复是否成功。

非目标（先别碰）：

- 不做跨 repo / 跨 workspace 的全局时间机器。
- 不承诺“永远可 100% 恢复”：restore 失败必须可见且可行动（建议生成报告 artifact）。
- 不把 checkpoint 做成默认自动功能（先证明真实需求）。
- 不恢复进程状态：restore 前必须确保没有 running process（用户可先 `process/kill`）。
- 不做 patch-based/增量快照（v1 只做完整快照）。

### 1.1 DoD（未来实现的可验证清单）

- `checkpoint/create` 成功后：
  - 追加事件 `CheckpointCreated`，并能从中定位 `checkpoint_id + snapshot_ref`。
  - 写入 `manifest.json` 与快照 payload（v1 建议为目录快照）。
- `checkpoint/restore` 成功后：
  - workspace 文件树回到 checkpoint 对应状态（边界内一致）。
  - 追加事件 `CheckpointRestored{status=ok}`。
- `checkpoint/restore` 失败时：
  - 追加事件 `CheckpointRestored{status=failed, reason=...}`（append-only）
  - 建议生成 `artifact_type="rollback_report"`（脱敏）并能从事件/产物定位到该报告。

---

## 2) 最小数据模型（建议，协议层）

> 优先把“可审计、可定位”写死，存储格式先保持实现自由度。

建议引入：

- `CheckpointId`：稳定 ID
- `CheckpointCreated { checkpoint_id, thread_id, turn_id?, label?, snapshot_ref }`
- `CheckpointRestored { checkpoint_id, thread_id, turn_id?, status, reason?, report_artifact_id? }`

其中：

- `snapshot_ref` 是一个 **opaque** 引用（字符串/结构体均可），指向可恢复的 workspace 快照（v1 推荐固定为 `workspace/` 目录快照）。
- `label` 只用于展示（例如 `before refactor`）。

---

## 3) 存储布局（建议写死）

> 目标：不把快照塞进 repo；快照属于运行时 artifacts，能被 `thread/delete` 一次性清理。

建议存储在 thread artifacts 下（见 `docs/runtime_layout.md`）：

```
<pm_root>/threads/<thread_id>/artifacts/checkpoints/<checkpoint_id>/
  manifest.json
  workspace/            # 目录快照（推荐 P0）
  snapshot.tar.zst      # 可选：压缩快照（P1+）
```

`manifest.json` 最小字段建议：

- `version: 1`
- `checkpoint_id`
- `created_at`
- `label?`
- `source`：`thread_id/turn_id?/cwd`
- `snapshot_ref`（与 `CheckpointCreated.snapshot_ref` 对齐）
- `stats`：`file_count/total_bytes`（用于 disk report 与清理）
- `ignored_globs`（用于审计：哪些路径不会进入快照）
- `size_limits`（用于审计：v1 建议的硬上限）
- `excluded`（用于审计：symlink/oversize 的跳过计数）

---

## 4) 快照边界（写死，安全默认）

快照只覆盖 thread `cwd` 下的工作区文件树，并遵守以下硬约束：

- 必须排除运行时目录：
  - `.codepm_data/tmp/**`、`.codepm_data/threads/**`
  - `.codepm_data/data/**`、`.codepm_data/repos/**`
  - `.codepm_data/locks/**`、`.codepm_data/logs/**`
- 必须排除 `.git/**`（避免体积与凭据/配置问题；不把 git 当正确性前提）。
- 必须避免“把大目录打进快照”：
  - `target/**`、`node_modules/**`、`example/**`（与仓库约定一致：`example/` 不作为依赖）
- secrets 默认禁入（保守；避免把明文塞进 repo/运行时目录）：
  - `.env`、`.env.*`、`*.pem`、`*.key`、`.ssh/**`、`.aws/**`、`.kube/**`
- 不跟随 symlink 逃逸：对 snapshot/restore 的路径遍历必须拒绝 `..` 与 symlink escape（口径与 `docs/runtime_layout.md`/`docs/redaction.md` 保持一致）。

v1 建议的 size limits（防止无意把大文件打进快照）：

- `max_file_bytes=32MiB`：超限文件跳过并计数（写入 manifest）。
- `max_total_bytes=1GiB`：创建快照时超限则失败（避免把磁盘打爆）。

备注：这不是“安全边界”，但这是最低限度的自保。

---

## 5) 操作语义（TODO：未来实现时必须满足）

### 5.1 `checkpoint/create`

- 作用：把当前 workspace（按第 4 节的边界）保存为一个可恢复快照。
- 落盘：
  - 写入快照目录与 `manifest.json`
  - 追加事件 `CheckpointCreated`

### 5.2 `checkpoint/restore`

restore 是破坏性操作，最小约束建议写死：

- 前置条件：
  - 必须没有 active turn
  - 必须没有 running process（否则直接拒绝；由用户先 kill）
  - thread `sandbox_policy` 不能是 `read_only`
- 审批（强制）：
  - restore 必须请求人工审批（建议使用 `approval.requirement="prompt_strict"`；见 `docs/approvals.md`），避免 `ApprovalPolicy=auto_approve` 静默覆盖文件树。
  - `prompt_strict` 不应被 remembered decision 自动复用（见 `docs/approvals.md`）。
  - `ApprovalRequested.params` 建议包含一个“restore plan”摘要：预计 `create/modify/delete` 的文件数量（不要把完整清单塞进事件）。
- 成功：workspace 文件树与 checkpoint 一致，并追加事件 `CheckpointRestored{status="ok"}`。
- 失败：
  - 追加事件 `CheckpointRestored{status="failed", reason}`（append-only）
  - 建议生成 `artifact_type="rollback_report"`（markdown，脱敏；见 `docs/redaction.md`），说明失败原因与下一步建议。

### 5.3 `rollback_report`（失败报告模板，建议写死）

```md
# Rollback Report

- checkpoint_id: ...
- thread_id: ...
- restored_at: ...
- status: failed
- reason: ...
- snapshot_ref: ...

## Restore plan (best-effort)

- create: ...
- modify: ...
- delete: ...

## Boundary

- ignored_globs: [...]
- size_limits: { max_file_bytes: ..., max_total_bytes: ... }
- excluded: { symlink_count: ..., oversize_count: ... }

## Next steps

- pm process list --thread <thread_id>
- pm process kill <process_id>
- pm checkpoint restore <thread_id> <checkpoint_id>
```

---

## 6) 快照的实现策略（先 P0，别上来就复杂）

可选实现（按复杂度从低到高）：

1. **目录快照（推荐 P0）**：复制 workspace 目录到 `artifacts/checkpoints/<id>/workspace/`。
2. **压缩包快照（省空间）**：写成 `tar.zst`（或等价），restore 时解包。
3. **patch-based（复杂且脆）**：保存 diff 并反向应用（失败概率高，不建议作为 P0）。

无论哪种实现，都必须：

- 受 sandbox/mode 控制（不能把 `.codepm_data/{tmp,threads,data,repos,locks,logs}/**` 当可回滚对象）。
- 有磁盘占用可观测（应复用 `docs/runtime_layout.md` 的 disk warning/report 思路）。

---

## 7) CLI/API（未来实现占位）

```bash
pm checkpoint create <thread_id> --label "before refactor"
pm checkpoint list <thread_id>
pm checkpoint restore <thread_id> <checkpoint_id>
```

---

## 8) 验收（未来实现时）

- 创建 checkpoint 后，产生可回放事件 `CheckpointCreated`，并能从中定位 `snapshot_ref`。
- restore 成功后，workspace 文件树回到 checkpoint 对应状态，且产生 `CheckpointRestored{status=ok}`。
- restore 失败时：
  - `CheckpointRestored{status=failed, reason=...}` 必须落盘
  - 建议自动生成 `artifact_type="rollback_report"`（脱敏）说明失败原因与下一步建议
