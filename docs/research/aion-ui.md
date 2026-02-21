# AionUi（example/agent-gui/aionui）能力与设计分析

> Snapshot: `example/agent-gui/aionui` @ `5abe63b0c076`
>
> 结论先行：AionUi 的价值不在“它支持很多模型”，而在于它把 **各种 terminal-based agent** 统一成一个 cowork 工作台：自动检测 CLI、每个会话独立上下文、本地 SQLite 存储、强预览面板（9+ 格式）、并提供 `--webui --remote` 把桌面能力远程化。对 `OmneAgent vNext` 来说，最值得抄的是：**把 agent runner 当成可插拔适配层（检测/配置/协议）+ 把 artifacts/preview 当成一等公民 + 远程控制需要明确安全模型**。

---

## 1) 产品定位：CLI agent 的 Cowork（不是 IDE）

`example/agent-gui/aionui/README.md` 明确宣称：

- 统一 UI 驱动 Gemini CLI / Claude Code / Codex / Qwen Code / Goose 等
- 多会话并行（每会话独立上下文）
- 本地存储（SQLite）
- 预览面板：PDF/Word/Excel/PPT/Markdown/HTML/Diff/图片等
- WebUI：`AionUi --webui` / `AionUi --webui --remote`

对 `OmneAgent`：这就是 RTS 控制台的“态势感知”组件：**你产出的不是 token，是文件、diff、日志、网页与图像**。没有 preview 面板，你的系统就是瞎跑。

---

## 2) CLI/协议检测：把“能不能跑”从用户脑子里拿掉

### 2.1 ACP detector：启动时扫描可用 CLI

`AcpDetector` 的策略很务实：

- 维护一个 `POTENTIAL_ACP_CLIS` 列表（候选 CLI 命令名 + backend id）
- 启动时并行执行 `which/where <cmd>`，把存在的 CLI 加入 detected list
- 只要检测到任何 ACP 工具，就额外插入一个 “Gemini CLI”（内置）
- 支持把“自定义 agent”（带 cliPath/acpArgs）追加到列表（从配置读取）

实现证据：`example/agent-gui/aionui/src/agent/acp/AcpDetector.ts`。

这比“让用户自己填一堆路径/参数”强：检测是系统职责，配置是覆盖/扩展。

### 2.2 IPC 暴露检测结果

通过 `ipcBridge.acpConversation.getAvailableAgents` 将检测结果提供给 renderer/UI，形成“设置页可见、可选”的闭环。

实现证据：`example/agent-gui/aionui/src/process/bridge/acpConversationBridge.ts`。

对 `OmneAgent`：这就是我们需要的 adapter 形态：**runner 发现/选择是控制面能力，不应该散落在一堆脚本文档里**。

---

## 3) MCP：把工具生态做成统一入口

`McpService` 会遍历检测到的 agents，并让每个 agent 实例去 `detectMcpServers()`，统一收集结果（还会额外加入 “native Gemini CLI” 用于 MCP detection）。

实现证据：`example/agent-gui/aionui/src/process/services/mcpServices/McpService.ts`。

对 `OmneAgent`：如果 vNext 要做“可扩展工具/资源/提示”，MCP 是现实世界的标准接口；我们不一定要复刻 AionUi 的实现，但要复刻“把 MCP 当一等能力”的态度。

---

## 4) Preview 历史：artifacts 的版本化（非常值钱）

`previewHistoryService` 做了一个很实用的设计：

- 用 `PreviewHistoryTarget`（workspace/filePath/title/type…）拼接 identity
- 对 identity 做 sha1 digest，作为磁盘目录名（稳定、短）
- 维护一个 `index.json`，记录最多 N 个版本（bounded history）
- 保存/列出/读取快照内容

实现证据：

- `example/agent-gui/aionui/src/process/services/previewHistoryService.ts`
- `example/agent-gui/aionui/src/common/types/preview.ts`

对 `OmneAgent`：这几乎就是我们想要的 `Artifacts` 子系统：**turn/task 产出物的可预览、可回滚、可对比**。RTS 风格下你会同时产生大量 artifacts，没有“历史与索引”，用户根本找不到东西。

---

## 5) WebUI：把桌面能力远程化（但别装作没有安全风险）

项目明确支持：

```bash
AionUi --webui
AionUi --webui --remote
```

实现证据：`example/agent-gui/aionui/README.md`、`example/agent-gui/aionui/WEBUI_GUIDE.md`、`example/agent-gui/aionui/package.json`。

对 `OmneAgent`：远程化对 RTS 控制台很诱人（手机/平板看进度、远程接管），但它强制你回答三个问题：

1. 认证/鉴权怎么做？
2. 远程能看到哪些文件/日志/密钥？
3. 工具执行是否需要审批？审批来自谁？

如果你回答不了，就别做 `--remote`。

---

## 6) 对 OmneAgent vNext 的启示（只取精华）

1. **runner 适配层要可插拔**：检测/配置/协议三件事要系统化。
2. **artifacts/preview 是产品核心，不是锦上添花**：没有预览，RTS 控制台就是盲飞。
3. **MCP/工具生态要预留标准接口**：不要做死在一套私有工具 schema 上。
4. **远程控制是高风险能力**：要么把安全模型讲清楚，要么别做。

