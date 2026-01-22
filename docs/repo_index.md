# Repo Index / Search（索引与搜索）（v0.2.0 现状 + TODO）

> 目标：让“我怎么定位到这些文件/这些匹配结果”变成可回放、可引用的产物（artifact），而不是一次性的 tool 输出。
>
> 现状：v0.2.0 已有 `file/glob`、`file/grep`、`file/read`（工具化、事件化），但**搜索结果本体不会自动写成 artifact**。

---

## 0) 范围与非目标

范围：

- 解释 v0.2.0 现状：`file/glob`/`file/grep` 的边界与落盘字段。
- 给出“把搜索结果写成 artifact”的最小口径（可手工组合，或未来用薄封装工具实现）。
- 预留 Repo index（文件清单/符号索引）的最小扩展点。

非目标（先别碰）：

- 不做 tree-sitter 符号索引、语义搜索、向量检索、ranking 算法。
- 不做常驻 daemon/后台增量索引与一致性保证。
- 不引入查询 DSL（filter/facet/highlight/boolean 等都先别写死）。

---

## 1) v0.2.0 现状：搜索原语（已实现）

### 1.1 `file/glob`（列文件）

- 扫描范围：thread 的 workspace root（`thread cwd`）。
- 默认忽略目录：`.git`、`.code_pm`、`.codepm`、`target`、`node_modules`、`example`（实现对照：`crates/app-server/src/main/file_read_glob_grep.rs`）。
- 并额外跳过 `.codepm_data/{tmp,threads,data,repos,locks,logs}/`（避免扫描运行时目录）。
- 参数边界：
  - `max_results` 默认 `2000`，上限 `20000`；超限 `truncated=true`。
- 事件落盘（摘要）：`ToolCompleted.result` 只记录 `{matches,truncated}`，不记录全部 paths（避免事件爆炸）。

### 1.2 `file/grep`（全文检索）

- 扫描范围：同上（workspace root + 默认忽略目录）。
- 资源边界（重要）：
  - `max_matches` 默认 `200`，上限 `2000`
  - `max_files` 默认 `20000`，上限 `200000`
  - `max_bytes_per_file` 默认 `1MiB`，上限 `16MiB`
  - 跳过二进制文件（检测 NUL byte）；匹配行会截断到 4000 字符（加 `…`）
- 事件落盘（摘要）：`ToolCompleted.result` 记录
  - `matches/truncated/files_scanned/files_skipped_too_large/files_skipped_binary`
  - **不记录**具体 `matches` 列表（列表只在 RPC 响应里返回）

### 1.3 结论：为什么需要 artifact

因为事件里只有摘要，`resume/replay` 时你会丢掉“当时具体搜到了哪些行”。要做到可审计/可引用，就必须把结果写成 artifact（见 `docs/artifacts.md`）。

---

## 2) 最小口径：Search Result Artifact（推荐约定）

> 这部分不要求新增协议：agent 可以用 `file/grep` 拿到结果，然后用 `artifact/write` 写一份可引用产物。

### 2.1 artifact 类型与 provenance（建议）

- `artifact_type="repo_search"`（建议写死；避免近似名字分裂）
- `summary`：`rg: <query> (<include_glob>)`
- `provenance`：
  - `thread_id` 必填
  - `turn_id` 建议填
  - `tool_id` 建议填为那次 `file/grep` 的 `tool_id`（`file/grep` RPC 响应会返回）

### 2.2 内容结构（写死最小模板）

Markdown 内容建议固定结构（方便人看/脚本解析）：

- Query：`query/is_regex/include_glob`
- Stats：`matches/truncated/files_scanned/...`
- Results：表格（path:line + line excerpt）
- Next steps：可复制命令（例如“打开文件/继续 grep/生成 diff”）

> 注意：结果过大时必须截断并显式写明 `truncated=true`，不要生成无限大 artifact。

---

## 3) TODO：薄封装工具（repo/index + repo/search）

> 目标：把“搜索→产物”收敛成单一 tool call，便于测试、审计与复用（避免 prompt 里拼装）。

### 3.1 `repo/search`（TODO）

行为：

- 内部复用 `file/grep` 的扫描逻辑与限制（保持同一 ignore list 与边界）。
- 直接写入一个 search artifact（见上文模板），并返回 `artifact_id`。
- `ToolCompleted.result` 只返回摘要 + `artifact_id`（避免把结果塞进事件）。

### 3.2 `repo/index`（TODO）

最小行为：

- 生成一个“文件清单/统计”artifact（`artifact_type="repo_index"`）。
- 默认只输出统计 + top-N 文件路径（避免一次性输出全仓库）。
- 未来如需增量索引，可在 artifact 内容里记录“扫描时间/忽略规则/截断信息”，不要先承诺缓存一致性。

---

## 4) 验收（未来实现时）

### 4.1 现状可做（不新增协议）

通过 `file/grep` + `artifact/write` 组合：

- `pm artifact list/read/delete` 能对 `repo_search` 产物进行管理（见 `docs/artifacts.md`）。
- 产物 metadata 的 provenance 能定位到 thread/turn/tool（见 `pm_protocol::ArtifactProvenance`）。

### 4.2 薄封装落地后（新增工具/CLI）

（占位 CLI）：

```bash
pm repo search <thread_id> --query "TODO" --include-glob "crates/**" --max-matches 50 --json
pm repo index <thread_id> --include-glob "**/*" --max-files 20000 --json
```

验收点：

- `repo/search` 必须返回 `artifact_id`，并且 `pm artifact read` 能读到固定模板的 Markdown。
- `repo/index` 生成的 artifact 必须标记是否截断（避免“看起来像全量，其实不是”）。
