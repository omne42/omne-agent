# Artifacts（产物）与 Preview（v0.2.0 口径）

> 目标：把“用户需要看的东西”（计划/报告/diff/日志摘要）做成一等产物：可索引、可定位、可回放、可清理。
>
> 约束：repo/workspace 内的代码变更属于“repo 更新”，不叫 artifact。artifact 指**给用户看的文档** + **不进 repo 的临时产物**（详见 `docs/v0.2.0_parity.md`）。

---

## 0) 运行时目录与定位

产物的实际落盘路径见：

- `docs/runtime_layout.md`

---

## 1) user artifacts（`artifact/*`）

### 1.1 文件形态（写死）

v0.2.0 user artifact 采用：

- `*.md`（内容）
- `*.metadata.json`（元数据）

同一个 `artifact_id` 的路径（见 `docs/runtime_layout.md`）：

```
<thread_dir>/artifacts/user/<artifact_id>.md
<thread_dir>/artifacts/user/<artifact_id>.metadata.json
```

### 1.2 元数据字段（已实现）

数据模型：`pm_protocol::ArtifactMetadata`（详见 `crates/agent-protocol/src/lib.rs`）。

关键字段（口径）：

- `artifact_id`：稳定 ID
- `artifact_type`：字符串（类型标签，用于 UI/脚本过滤）
- `summary`：一行摘要（用于列表）
- `preview`：可选渲染提示（见 §3）
- `created_at/updated_at`：RFC3339
- `version`：单调递增（从 1 开始）
- `content_path`：`*.md` 绝对路径（字符串）
- `size_bytes`：内容大小
- `provenance`：`thread/turn/tool/process` 溯源（可选）

### 1.3 写入与版本语义（已实现）

- `artifact/write`：
  - 若未指定 `artifact_id`：生成新 ID，`created=true`，`version=1`。
  - 若指定 `artifact_id` 且已存在：覆盖 `*.md` 内容，`version += 1`，保留 `created_at`，更新 `updated_at`。
- bounded history 默认关闭：不会保留旧版本内容（只有 version 号递增）；可用 `CODE_PM_ARTIFACT_HISTORY_MAX_VERSIONS` 启用（见 §4）。

补充：

- 写入内容会做脱敏（见 `docs/redaction.md`）。
- 写入行为仍受 `mode/approval` 约束（见 `docs/modes.md`、`docs/approvals.md`）。

### 1.4 CLI（可复制）

```bash
pm artifact list <thread_id>
pm artifact read <thread_id> <artifact_id>
pm artifact delete <thread_id> <artifact_id>
```

生成 diff artifact（`git diff`，写入 user artifact `artifact_type="diff"`）：

```bash
pm thread diff <thread_id> --max-bytes 4194304 --wait-seconds 30
```

---

## 2) process artifacts（stdout/stderr）

`process/start` 会把 stdout/stderr 实时追加写入 artifacts，并支持 rotate：

- rotate 默认阈值 `8MiB`，可用 `CODE_PM_PROCESS_LOG_MAX_BYTES_PER_PART` 覆盖
- attach（只读）：`process/tail`、`process/follow`

详见：

- `docs/runtime_layout.md`

注意：

- process logs 属于“原始输出”，不保证脱敏；不要把完整 stdout/stderr 直接注入模型上下文（推荐生成脱敏摘要 artifact）。

---

## 3) Preview types（已实现：协议层）

现状：

- `artifact_type` 仍是字符串标签（用途/产品语义）。
- metadata 新增可选字段 `preview`（渲染提示），用于 UI/工具选择合适的预览方式。

协议类型：

- `pm_protocol::ArtifactPreviewKind`：
  - `markdown` / `diff_unified` / `patch_unified` / `code` / `html` / `log`
- `pm_protocol::ArtifactPreview`：
  - `preview: { kind, language?: string, title?: string }`
- `pm_protocol::ArtifactMetadata.preview: Option<ArtifactPreview>`（旧的 `*.metadata.json` 可能缺失该字段）

app-server 默认推断规则（写入/覆盖 artifact 时填充 `preview`）：

- `artifact_type="diff"` → `preview.kind="diff_unified"`
- `artifact_type="patch"` → `preview.kind="patch_unified"`
- `artifact_type="html"` → `preview.kind="html"`
- `artifact_type="code"` → `preview.kind="code"`
- `artifact_type="log"|"log_excerpt"` → `preview.kind="log"`
- 其他 → `preview.kind="markdown"`

兼容规则（建议写死）：

- metadata 缺失 `preview` 时：默认按 `markdown` 处理（或按 `artifact_type` 推断）。
- 未知 `preview.kind` 必须降级为纯文本显示（不要报错，也不要静默丢失内容）。
- markdown 渲染必须是“安全子集”（禁用/转义原始 HTML），避免内容注入。

`artifact_type` 惯例值（非强制；仍允许自定义）：

- `markdown`（默认）
- `plan`
- `disk_report`
- `repo_search`
- `diff` / `patch`（内容格式建议 unified）
- `test_report`
- `log_excerpt`
- `stuck_report`
- `rollback_report`
- `hook_context`
- `mcp_result`
- `fan_out_result`
- `fan_in_summary`
- `artifact_prune_report`

### 3.1 `artifact_type` vs `preview`（已实现：语义拆分）

这两个维度要分离（别混成一坨字符串）：

- `artifact_type`：**用途/产品语义**（例如 `stuck_report`、`disk_report`、`plan`、`review`…）
- `preview`：**渲染提示**（例如 `diff_unified`、`code(rust)`、`html`…）

这样 UI 可以：

- 用 `artifact_type` 做过滤与列表分组（“这是什么”）
- 用 `preview.kind` 选择渲染器（“怎么预览”）

最小 metadata 扩展示例：

```json
{
  "artifact_id": "01J…",
  "artifact_type": "diff",
  "summary": "git diff (workspace)",
  "version": 3,
  "preview": { "kind": "diff_unified", "title": "git diff --" }
}
```

---

## 4) bounded history（保留旧版本；已实现）

目标：

- 保留最近 N 个版本内容（避免“覆盖后找不回”），但不能让磁盘无限增长。

配置项：

- `CODE_PM_ARTIFACT_HISTORY_MAX_VERSIONS`：
  - `0` = 关闭（默认）
  - `N>0` = 保留最近 N 个旧版本

### 4.1 路径布局（写死）

建议把历史版本收进一个单独目录（避免把 `artifacts/user/` 撒满各种 `.v0007.md`）：

```
<thread_dir>/artifacts/user/<artifact_id>.md
<thread_dir>/artifacts/user/<artifact_id>.metadata.json
<thread_dir>/artifacts/user/history/<artifact_id>/v0001.md
<thread_dir>/artifacts/user/history/<artifact_id>/v0002.md
...
```

### 4.2 行为与保留策略（已实现）

当 `artifact/write` 覆盖一个已存在的 `artifact_id` 且 bounded history 开启时：

- 在覆盖前，把旧内容复制到 `history/<artifact_id>/v{old_version:04}.md`。
- 写入成功后，仅保留最近 `N` 个历史版本（不包含当前最新版本）；超出则删除最老的。
- `artifact/delete` 会级联删除 `history/<artifact_id>/`（避免“以为删了但旧版本还在”）。

补充：

- 当发生清理时，`artifact/write` 的 tool result 会包含 `history.pruned_versions`（随事件落盘，可审计）。

### 4.3 清理的可审计性（别悄悄删）

当发生历史版本清理时，未来可以生成一份 user artifact（TODO）：

- `artifact_type="artifact_prune_report"`
- `summary`：例如 `pruned artifact history: <artifact_id> (kept N)`
- 内容：列出删除了哪些版本号与 bytes（尽量避免写入绝对路径），并走脱敏（见 `docs/redaction.md`）。

安全边界（建议写死）：

- bounded history 仅针对 user artifacts（`artifact/*`），不影响 process logs。
- 删除 artifact 时必须级联删除 history（避免“以为删了但旧版本还在”）。

### 4.4 CLI/API（未来实现占位）

```bash
pm artifact versions <thread_id> <artifact_id>
pm artifact read <thread_id> <artifact_id> --version 2
```

---

## 5) 验收（已实现）

- 开启 `CODE_PM_ARTIFACT_HISTORY_MAX_VERSIONS=2`：
  - 连续写入同一 `artifact_id` 3 次后，`history/<artifact_id>/` 下最多只保留 2 个旧版本文件。
- 执行 `pm artifact delete <thread_id> <artifact_id>` 后，`history/<artifact_id>/` 会被级联删除。
