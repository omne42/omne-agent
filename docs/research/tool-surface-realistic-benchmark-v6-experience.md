# Tool Surface Benchmark v6：测试经验与哲学

## 核心原则

1. 不强制模型调用工具。
   - 评测应模拟真实用户场景，不用伪代码 Prompt 绑架模型输出格式。
   - 如果任务可凭常识回答，不应把“没调工具”直接视为系统缺陷。

2. 用信息差驱动工具调用。
   - Case 设计要让模型无法从参数记忆直接回答，例如：读真实文件、查真实进程状态、拿真实网络返回。
   - 让模型“必须调用”，而不是“被命令调用”。

3. 工具运行必须给证据，不给占位。
   - 禁止统一 `task_completed` 这类空结果。
   - 返回真实 payload（content / stdout / matches / ids / hashes / status 等）。

4. 接口要顺应模型直觉。
   - 允许 facade 与 atomic 等价路由（alias），减少“能力已完成但工具名不同”导致的误判。
   - 对常见参数形态做容错兼容（尤其 MCP 参数结构）。

## v6 迭代路径（real-sandbox）

- v6: 首版 real-sandbox，真实执行路径打通，但 auto prompt 仍偏泛化。
- v6-r2/r3/r4: 强化信息差 prompt + 扩展 alias + 修复路径与样本数据。
- v6-r5/r6: 修复剩余边缘失败（web fallback、image fixture、kill/view 场景）。
- v6-r7: 在不启用 no-tool guard 的前提下，达到 43/43 通过。

## 工程落地建议

1. 生产默认 `real-sandbox`，`mock` 仅用于回归对照。
2. Benchmark 报告必须同时保存 `raw_results.json` 和同名 `raw_results.toon`。
3. 失败分类优先区分：
   - 模型跳过动作
   - 等价动作误判
   - 运行时环境失败（网络/权限/路径）
4. 先修评测误判和工具可用性，再讨论模型能力优劣，避免错怪模型。
