# Tool Surface Benchmark v2 (Realistic)

这个版本专门解决“提示词污染”和“模型跳过工具直接交卷”的问题，并支持真实执行（real sandbox）：

- `User Prompt` 是真实用户意图，不再包含伪代码控制语句（例如 `When tool returns ok=true...`）。
- 默认注入完整工具面（从 `spec.rs` 自动读取），不做“单工具强引导”。
- 成功判定不看模型自报 `success:true`，而是看外部证据：
  - 目标工具是否真实调用成功
  - 最终回答是否包含工具返回的证据 nonce（模型无法提前猜到）
- `recovery` 注入错误由 runtime 层执行，不写进 prompt。
- 默认 `--runtime-mode real-sandbox`，工具返回真实执行结果（文件内容、进程输出、检索命中等），不再返回统一 `task_completed` 占位。
- `--runtime-mode mock` 仍保留，用于历史结果对照。
- “无工具结案拦截”默认关闭（`--no-tool-conclusion-guard off`），仅用于对照实验。

## 文件

- 运行脚本: `scripts/tool_suite/run_tool_surface_benchmark_v2.py`
- 样例 case: `scripts/tool_suite/cases.default.v2.json`
- 信息差 case: `scripts/tool_suite/cases.information_gap.v1.json`
- provider 配置: `scripts/tool_suite/providers.example.json`

## 运行

### 1) 只展开 case 和工具面（不请求模型）

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --list-cases-only
```

### 2) 自动生成“全工具一条用例”并只展开

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --case-source auto \
  --list-cases-only
```

### 3) 跑一个 provider

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --benchmark-version v3 \
  --runtime-mode real-sandbox \
  --no-tool-conclusion-guard off \
  --providers codex_responses \
  --modes direct,recovery
```

### 4) 指定 case 文件

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --benchmark-version v3 \
  --runtime-mode real-sandbox \
  --no-tool-conclusion-guard off \
  --providers codex_responses \
  --cases-file scripts/tool_suite/cases.information_gap.v1.json
```

### 5) 自动 case + 全工具注入跑测

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --benchmark-version v3 \
  --runtime-mode real-sandbox \
  --no-tool-conclusion-guard off \
  --providers codex_responses \
  --case-source auto \
  --modes direct,recovery
```

### 6) 限制工具注入面（可选）

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --benchmark-version v3 \
  --runtime-mode real-sandbox \
  --no-tool-conclusion-guard off \
  --providers codex_responses \
  --tools file_read,file_glob,file_grep,file_write \
  --case-source auto
```

### 7) 配置真实执行沙箱边界（推荐）

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --providers codex_responses \
  --case-source auto \
  --runtime-mode real-sandbox \
  --runtime-workspace-root tmp/tool_suite_runtime \
  --real-process-allowlist echo,python,python3,ls,pwd,date,whoami,cat,grep,head,tail,uname \
  --real-process-timeout-sec 20 \
  --real-web-timeout-sec 12
```

## 输出

每次运行写入 `docs/reports/tool-surface-realistic-benchmark-<timestamp>/`：

- `report.md`: 汇总报告（pass/skip/fake-success 等）
- `raw_results.json`: 全量结构化结果
- `details/<provider>/<mode>/<case_id>.json`: 单案例完整链路
- `raw_results.json.meta.args.runtime_mode`: 记录本次是 `mock` 还是 `real-sandbox`

推荐在报告目录额外记录：

- 测试经验与哲学：`docs/research/tool-surface-realistic-benchmark-v3-philosophy.md`

## 指标说明

- `case_pass`: 框架判定目标工具执行成功（direct）或“注入失败后修复成功”（recovery）
- `skip_tool`: 本轮没有任何工具调用
- `guard_trigger_rate`: 触发“无工具结案拦截”的比例
- `target_miss_rate`: 调用了工具但没有调用到目标工具的比例（路由偏移）
- `claimed_success_without_execution`: 没执行成功但文本里出现“success/完成/done”等成功声称
- `fake_success_rate`: 上述“口头成功”占比
- `evidence_echo_rate`: 最终文本主动回显证据 nonce 的比例（质量指标，不作为通过硬门槛）

Omne Agent runtime 里的同类拦截可通过环境变量关闭：

```bash
export OMNE_AGENT_NO_TOOL_CONCLUSION_GUARD=0
```

Benchmark 脚本本身默认关闭 no-tool 拦截（更贴近真实场景）；如需做对照实验可显式打开：

```bash
python3 scripts/tool_suite/run_tool_surface_benchmark_v2.py \
  --no-tool-conclusion-guard on \
  --providers codex_responses
```
