# Omne Agent Benchmark 演进与设计哲学白皮书

**日期**：2026-03-03  
**版本**：v1.0  
**主题**：从“指令遵循”到“真实意图”——Agent 测试框架的范式转移

---

## 1. 核心结论 (Executive Summary)

经过四轮迭代与深度分析，我们推翻了早期的“白盒强制测试”策略，确立了**“以真实意图为导向，以信息差驱动工具调用”**的最终测试哲学。

- **废弃**：不再使用“伪代码 Prompt”和“Interceptor（强制拦截器）”来逼迫模型调用工具。
- **确立**：测试用例必须包含模型无法通过内部知识回答的**动态信息（Information Gap）**或**环境副作用（Side-effect）**，从而让模型“不得不”主动调用工具。
- **优化**：工具 API 设计应顺应大模型直觉（扁平化参数、原子化工具），而非强迫模型适应代码规范。

---

## 2. 演进历程 (Evolution History)

### 第一阶段：机械测试与“捷径效应” (The Shortcut Problem)

- **现象**：
  - 在 `artifact__op_read` 等测试中，模型 0 次调用工具，直接输出 `{"success": true}`。
  - System Prompt 包含大量 JSON 格式约束和 `if tool returns error then...` 的伪代码逻辑。
- **根因分析**：
  - **提示词污染**：Prompt 将“输出特定格式 JSON”设立为了最终目标。模型发现了“不调工具也能编造出 JSON”的捷径（Shortcut Learning）。
  - **测试异化**：把 Agent 当作了只会执行命令的状态机，而非理解意图的智能体。

### 第二阶段：自然语言与“反思能力” (Natural Language & Reflection)

- **改进**：
  - 移除了 User Prompt 中的 JSON 约束，改为纯自然语言指令（如“帮我读一下文件”）。
- **成果**：
  - 模型表现出了**反思能力**：当 Mock 工具返回空数据时，模型会向用户反馈“没读到内容，是否换个方式？”。
  - 模型展现了**规划能力**：在修改文件前，主动先调用 `file_read` 确认内容（Look before you leap）。
- **残留问题**：
  - 在简单任务（如 `start_echo_process`）中，模型依然倾向于直接通过对话回复结果，导致测试失败。

### 第三阶段：拦截器争议与“智能”的定义 (The Interceptor Debate)

- **尝试方案**：
  - 引入 **Interceptor（拦截器）**：当用户指令包含动作意图，但模型未调用工具直接回复时，系统强制注入 System Prompt 驳回并要求重试。
- **关键转折 (Critical Turning Point)**：
  - **Case 分析**：`start_echo_process` ("执行 echo hello")。
  - **模型行为**：直接回复 "hello"。
  - **拦截器行为**：判定为 FAIL，强制模型去跑一个 Linux 进程。
  - **你的观点（最终采纳）**：**这是错误的。** 对于无副作用且结果确定的任务，模型利用内部权重直接回答才是**高效**和**智能**的体现。强制调用工具是资源浪费，也不符合真实用户“只看结果，不看手段”的习惯。

### 第四阶段：最终策略——基于信息差的测试 (Information Gap Strategy)

- **决策**：
  - **移除拦截器**：允许模型在它认为不需要工具时直接回答。
  - **重构测试用例**：将“为了测工具而测工具”的 Case，改为“缺失外部信息”的 Case。

---

## 3. 最终设计准则 (Design Principles)

### 3.1 测试用例设计：隐式触发 (Implicit Triggering)

我们不再命令模型“请使用工具 A”，而是通过任务性质迫使模型寻求工具帮助。

| ❌ 旧 Case (Bad) | ✅ 新 Case (Good) | 原理 (Rationale) |
| :--- | :--- | :--- |
| **User**: "执行 echo hello" | **User**: "检查当前环境 cargo 的版本号" | 模型无法通过训练数据得知当前机器的动态环境信息，必须调 `process_start`。 |
| **User**: "计算 1+1" | **User**: "读取 a.txt 里的数字并加 1" | 模型必须产生 IO (Read) 才能获取计算输入，必须调 `file_read`。 |
| **User**: "使用 fs_mkdir 创建目录" | **User**: "在 tmp 下建个 logs 文件夹" | 任务带有明确的**副作用 (Side-effect)**，光口头答应没有用，必须调 `fs_mkdir`。 |

### 3.2 工具 API 设计：顺应直觉 (Agent-Native API)

代码必须去适应模型的大脑，减少阻力。

1. **参数扁平化 (Flattening)**：
   - 模型讨厌嵌套。
   - *Action*：在 Rust 后端 (`mcp_call`) 实现逻辑，允许模型把 `arguments` 散落在顶层，由后端自动归拢。
2. **原子化优先 (Atomic vs Facade)**：
   - 模型更喜欢 `thread_usage` 这种具体的工具，而不是 `thread(op="usage")` 这种万能入口。
   - *Action*：测试脚本和 Prompt 应兼容别名，或者对外暴露原子工具。
3. **丰富的反馈 (Rich Feedback)**：
   - 工具不仅要返回 `OK`，还要返回“成功写入 20 字节”或“当前文件内容预览”，防止模型因通过 Mock 数据不足而产生幻觉。

---

## 4. 下一步行动计划 (Action Plan)

1. **Benchmark 修正**：
   - 修改 `cases.json`，替换掉所有“常识性”或“无副作用”的简单 Case。
   - 引入 `check_version`, `get_system_time`, `generate_uuid` 等强依赖工具的任务。
2. **代码层优化**：
   - 在 Omne Agent 核心实现 `mcp_call` 的参数自动包装逻辑，彻底解决嵌套参数报错问题。
3. **生产环境策略**：
   - 在真实产品中，不部署全局拦截器。仅在极少数高风险操作（如删除核心数据）时，才引入“确认拦截”，其余时刻信任模型的判断。

---

> **总结**：
>
> 我们从试图控制模型的一举一动，进化到了**通过设计环境和任务来引导模型**。这不仅让 Benchmark 更具说服力，也让 Omne Agent 更接近一个能在真实世界解决问题的“智能体”，而非一个仅仅通过单元测试的“机器人”。
