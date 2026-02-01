# Presets（可迁移配置）（v0.2.0 现状 + TODO）

> 目标：把 thread 的关键运行配置（approval/sandbox/mode/model/base_url）做成**可导出/可分享/可导入**的 preset 文件。
>
> 硬规则：preset **不包含任何 secrets**；密钥只来自运行环境（env/OS keychain），且不进事件/日志（见 `docs/redaction.md`）。

---

## 0) v0.2.0 现状：已有原语（可手工达成）

v0.2.0 已提供最小的 `omne-agent preset` import/export（无 secrets；通过 `thread/config-explain` + `thread/configure` 落盘 `ThreadConfigUpdated`）。

### 0.1 导出（export）

```bash
omne-agent preset export <thread_id> --out .omne_agent_data/spec/presets/coder-default.yaml
```

可选元信息：

```bash
omne-agent preset export <thread_id> \
  --out .omne_agent_data/spec/presets/coder-default.yaml \
  --name coder-default \
  --description "safe defaults for this repo"
```

### 0.2 导入（import）

```bash
omne-agent preset import <thread_id> --file .omne_agent_data/spec/presets/coder-default.yaml
```

约束：

- `import` 默认只允许从 `<agent_root>/spec/` 下加载（可提交/可 review）；`agent_root` 默认是 `<cwd>/.omne_agent_data`，可用 `--root` 覆盖。
- preset 文件严格 allowlist（`deny_unknown_fields`），未知字段直接报错。
- preset 文件不包含任何 secrets；密钥只来自运行环境（见 `docs/redaction.md`）。

### 0.3 仍可手工（调试/兜底）

```bash
omne-agent thread config-explain <thread_id> --json
omne-agent thread configure <thread_id> --help
```

---

## 1) TODO：Preset 文件位置与发现顺序（建议写死）

> 目标：不要发明“配置搜索 DSL”。固定路径 + 固定优先级就够了。

建议的 preset 文件位置：

- Canonical：`./.omne_agent_data/spec/presets/<name>.yaml`
- Default：`./.omne_agent_data/spec/preset.yaml`（可选；作为“项目默认 preset”）

v1 建议（写死）：

- preset **不自动生效**：不会因为文件存在就隐式改变 thread 配置。
- preset 只能通过显式动作生效：
  - 人类显式 `omne-agent preset import ...`（或等价 API）把 preset 物化为一次 `ThreadConfigUpdated`。
  - 人类显式 `omne-agent preset export ...` 从当前 thread 导出一个可 review 的文件。
- 信任边界（写死）：
  - v1 只允许从 `./.omne_agent_data/spec/` 下加载 preset（可提交/可 review）。
  - 不从 env/网络/任意绝对路径隐式加载。
  - 不从 `.omne_agent_data/{tmp,threads,data,repos,locks,logs}/` 这类运行时目录加载。

建议的发现/覆盖顺序（从低到高，越后越强）：

生效层级（与现状保持一致）：

1. default（硬编码）
2. env（如 `OMNE_AGENT_OPENAI_MODEL`/`OMNE_AGENT_OPENAI_BASE_URL`，见 `docs/model_routing.md`）
3. thread（`ThreadConfigUpdated` 事件）

preset/CLI flags 的关系：

- preset 导入与 CLI 显式 flags **都应**通过写入 `ThreadConfigUpdated` 来生效（因此在 explain 里同属 `thread` 层）。
- 当“同一次启动/配置”同时提供 preset 与 CLI flags：建议 merge 规则为 **CLI flags 覆盖 preset**（仅覆盖显式提供的字段），再把 merge 后的结果写入 `ThreadConfigUpdated`。

备注：

- v0.2.0 的 `thread/config/explain` 目前只有 `default → env → thread` 三层；引入 preset 后建议把 preset 作为独立层展示（可解释性），但实现仍是 TODO。

---

## 2) TODO：最小数据模型（v1）

> 原则：严格 allowlist；未知字段直接报错（避免“静默忽略导致误配置”）。

### 2.1 文件结构（YAML）

```yaml
version: 1
name: coder-default
description: "safe defaults for this repo"
thread_config:
  approval_policy: auto_approve
  sandbox_policy: workspace_write
  sandbox_network_access: deny
  sandbox_writable_roots: ["."]
  mode: coder
  model: gpt-4.1
  openai_base_url: https://api.openai.com/v1
```

字段与协议类型对齐（snake_case）：

- `approval_policy`: `auto_approve|on_request|manual|unless_trusted|auto_deny`
- `sandbox_policy`: `read_only|workspace_write|danger_full_access`
- `sandbox_network_access`: `deny|allow`

可选字段（TODO，先别承诺实现）：

- `execpolicy_rules`: `["./.omne_agent_data/spec/execpolicy/*.rules", ...]`
  - 备注：execpolicy 目前是 **app-server 全局启动参数**（`omne-agent --execpolicy-rules <path>`），不是 thread 配置；preset 里最多作为“启动建议/提示”，导入到已运行的 app-server 不应静默生效。

硬规则（再次强调）：

- preset 文件里 **不允许**出现任何 secrets（包括“引用/占位符”）。如需密钥，只能来自运行环境（env/OS keychain）。

### 2.2 目录与可提交性

- preset 属于“项目可提交配置”，建议放在 `./.omne_agent_data/spec/`（见 `docs/runtime_layout.md`）。
- `.omne_agent_data/` 下只有 `spec/` 承载可迁移 preset；其它子目录都是运行时数据，不承载 preset。

---

## 3) TODO：导入语义（apply preset）

最小导入语义建议：

1. 解析 preset（YAML）。
2. 校验：
   - `version` 受支持。
   - 所有字段都在 allowlist 中（未知字段报错）。
   - `mode` 若存在，必须能在 `ModeCatalog` 找到（否则报错）。
   - 文件中若出现任何“看起来像 secrets 的字段名/值形态”，应直接报错（宁可误报也别误放行；见 `docs/redaction.md` 的口径）。
   - `sandbox_writable_roots` 必须通过与 `thread/configure` 一致的路径校验（拒绝 `..` 与 symlink escape；路径解析以 thread cwd 为基准）。
3. 路径处理：
   - `sandbox_writable_roots` 以 thread cwd 为基准解析；
   - 若未来支持导出为相对路径，导入端需把 `.` 等相对路径规范化为绝对路径（行为与 `thread/configure` 一致）。
4. 应用：
   - 调用 `thread/configure` 写入 `ThreadConfigUpdated` 事件。

审计建议（TODO）：

- 导入时追加一条 provenance 事件（例如 `ThreadPresetApplied { name, path, sha256 }`），避免“只看结果看不出来源”。
- 日志/事件只记录 `name/path/hash` 与脱敏视图；禁止落盘原始 YAML payload（见 `docs/redaction.md` 的口径）。

---

## 4) TODO：导出语义（export preset）

最小导出语义建议：

1. 输入：`thread_id`。
2. 来源：调用 `thread/config/explain`，取 `effective`（这是可解释的最终生效值）。
3. 生成：按 2.1 的结构写出 YAML（字段稳定排序，便于 diff/review）。
4. 便携性：
   - `sandbox_writable_roots` 若位于 thread 根目录下，导出为相对路径（例如 `.`）；
   - 若位于根目录外，导出为绝对路径并显式标注“不可移植”（避免静默坑别人）。

安全约束（硬规则）：

- 导出严格 allowlist；**不导出任何密钥/令牌**（例如 `OPENAI_API_KEY`/`OMNE_AGENT_OPENAI_API_KEY`）。
- 若未来引入 provider 配置与 secret refs：导出只允许“引用/占位符”（例如 `{{ENV:OPENAI_API_KEY}}`），绝不写明文。

---

## 5) DoD（v0.2.0 最小实现）

- `omne-agent preset export <id> --out .omne_agent_data/spec/presets/x.yaml` 生成的文件里不包含任何 token 形态（例如 `rg -n \"sk-\" .omne_agent_data/spec/presets/x.yaml` 命中为 0）。
- `omne-agent preset import <id> --file .omne_agent_data/spec/presets/x.yaml` 后，`omne-agent thread config-explain <id> --json` 的 `effective` 与 preset 对齐。
- （如果实现了独立层）`thread/config/explain.layers` 能看到 `preset` 来源与元信息（name/hash）。
