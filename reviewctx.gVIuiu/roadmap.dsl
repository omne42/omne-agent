workspace "AI Gateway Platform" "双产品独立部署架构 (深度治理与控制面隔离版)" {

    model {
        properties {
            "structurizr.groupSeparator" "/"
        }

        // =========================================================
        // 最顶层：北向角色
        // =========================================================
        nb_clients = person "应用 / SDK 用户 / 内部平台" "北向业务调用方" "api"
        nb_admins = person "平台管理员 / 运营 / 安全团队" "平台管控与运营方" "api"
        nb_ai_operator = person "AI 智能体 / 机器操作员 (AI Client / Operator)" "Machine-first 的机器调用与自治运维方" "api"

        // =========================================================
        // 核心大生态系统
        // =========================================================
        aiGateway = softwareSystem "AI 网关与企业平台生态" "包含 L0/L1 基础网关 与 L2 商业企业平台" {

            // ---------------------------------------------------------
            // 【独立产品 B】Enterprise Platform (L2 企业产品)
            // ---------------------------------------------------------
            group "【独立产品 B】Enterprise Platform (L2 企业产品)" {

                group "L2 仓库边界与入口" {
                    repo_l2 = container "L2 企业平台代码仓库" "独立企业版代码" "" "repo"
                    ent_portal = container "管理后台 (Portal) / BFF" "企业级管理入口" "" "enterprise"
                }

                group "L2 舰队与多环境管控" {
                    gitops_engine = container "GitOps 引擎 (IaC Pipeline)" "监听 PR 合并，自动化触发配置灰度流水线" "" "enterprise"
                    gateway_fleet_manager = container "网关舰队管理 (Fleet Manager)" "跨 Region 统管、配置 Bundle 快照统一发布者" "" "enterprise"
                    tenant_admin = container "多租户管控 (Tenant Admin)" "复杂组织树 / 环境隔离" "" "enterprise"
                }

                group "L2 治理、风控与财务中心 (复杂流程保留在 L2)" {
                    ent_identity = container "企业身份集成 (Identity)" "SSO / SCIM / 组同步" "" "enterprise"
                    ent_govern = container "治理中心 (Governance)" "全局配置与生命周期管控" "" "enterprise"
                    policy_center = container "策略中心 (Policy Center)" "策略编排 / 灰度治理 / 例外申请" "" "enterprise"
                    approval_center = container "审批中心 (Approval Center)" "对接企微/飞书等硬性审批流" "" "enterprise"
                    ent_finops = container "财务与审计总线 (FinOps)" "全局资金与合规管控" "" "enterprise"
                    billing_center = container "计费中心 (Billing Center)" "事前 Token 预检 / 业务 ROI 看板 / 熔断" "" "enterprise"
                    security_console = container "安全风控中心 (Security)" "事前安全策略定义 / 事后审计追踪" "" "enterprise"
                }

                group "L2 AI 资产与质量评价中心" {
                    ent_prompt_hub = container "企业资产枢纽 (Prompt Hub)" "提示词库 / 模型目录 / 策略规则库" "" "enterprise"
                    ent_eval_audit = container "评估工作台 (Eval Studio)" "PromptOps CI 钩子 / 沙箱跑测 / 影子测试" "" "enterprise"
                }

                group "L2 业务流程编排与智能体生态" {
                    client_sdk = container "业务客户端 SDK (Client SDK)" "北向集成应用层 SDK (封装 HTTP/WS 请求)" "" "frozen"
                    ent_apps = container "企业级 AI 应用 (Apps)" "内部控制台与模型市场" "" "enterprise"
                    agent_runtime = container "智能体引擎 (Agent Runtime)" "基础：Tool Loop/基本编排/Schema" "" "enterprise"
                    agent = container "企业流程编排 (Orchestrator)" "跨系统集成工作流 / 发布管控" "" "enterprise"
                    workflowplugins = container "工作流基石插件 (Plugins)" "结构化输出注入 / 编排算子" "" "plugin"
                    mcp_hub = container "企业 MCP 枢纽 (MCP Server)" "标准 MCP 协议内部数据源与 API 挂载注册" "" "enterprise"
                    rag_service = container "RAG 知识库挂载服务" "开箱即用的向量数据库挂载" "" "enterprise"
                    connector_hub = container "企业集成中心 (Connectors)" "IdP / SIEM / ERP 告警连接器" "" "enterprise"
                    warehouse_export = container "数据仓库导出 (Export)" "清洗并导出微调语料" "" "enterprise"
                    ent_store = container "企业级存储 (Stores)" "高可用数据库集群" "Database" "store"
                }
            }

            // ---------------------------------------------------------
            // 【核心产品 A】AI Gateway (L0/L1 基础网关)
            // ---------------------------------------------------------
            group "【核心产品 A】AI Gateway (L0/L1 基础网关)" {

                group "北向 API 矩阵 (Default Data Plane)" {
                    nb_models = container "GET /v1/models" "模型列表 (Machine-friendly 能力自描述探测)" "" "api"
                    nb_chat = container "POST /v1/chat/completions" "对话补全" "" "api"
                    nb_health = container "health / ready" "健康探针与机器状态面" "" "api"
                }

                group "L1 控制面 (Control Plane - 机器友好的声明式运维表面)" {
                    l1_control_api = container "声明式控制 API (Control API)" "接收唯一声明式快照 (Snapshot) 下发同步" "" "api"
                    l1_explain_simulate = container "诊断与调试表面 (Explain/Simulate)" "路由 Explain / 流量 Dry-run 模拟预测" "" "api"
                    l1_event_export = container "结构化事件导出 (Event Export)" "输出机读友好的生命周期与审计结构化事件" "" "api"
                    admin_ui_lite = container "轻量管理台 (Admin UI Lite)" "Provider Health 面板 / 基础监控" "" "api"
                    l1_instance_registry = container "实例与部署注册表 (Registry)" "网关节点与路由元数据" "" "api"
                    l1_user_mgmt = container "基础消费者管理 (Principal)" "只读授权快照缓存 / 受限的自动化操作支持" "" "api"
                    key_mgmt = container "密钥管理 (Key Mgmt)" "分发与校验 / 别名管理" "" "api"
                    l1_prompt_mgmt = container "运行时 Prompt 缓存" "版本映射 / 快速回滚" "" "api"
                }

                group "L1 数据面主干 (Default Pipeline)" {
                    repo_l1 = container "L1 轻量服务代码仓库" "提供基础网关基座" "" "repo"
                    l1_http = container "HTTP 传输网关 (Transport)" "稳态 SSE / 限流缓冲 / Backpressure" "" "api"
                    l1_auth = container "服务认证 (Service Auth)" "API Keys / Tokens 快速校验" "" "api"
                    sync_guardrails = container "同步前置护栏 (Preflight)" "不阻塞流的同步鉴权与硬性规则拦截" "" "api"
                    exact_cache = container "精确命中缓存 (Exact Cache)" "L1 默认快速代理命中与成本节约记账" "" "api"
                    l1_policy = container "约束与策略管道 (Base Policy)" "产出约束条件、Fallback 链与基础权重" "" "api"
                    route_mgmt = container "在线路由引擎 (Route Mgmt)" "依据规则与权重评分执行最终选路" "" "api"
                    l1_obs = container "服务可观测性 (Observability)" "Trace Correlation / 请求重放" "" "api"
                    usage_export = container "用量推流管道 (Usage Sink)" "Token 消耗与 Cache 避归成本异步导出" "" "api"
                    l1_store = container "服务运行时存储 (Store)" "高速查询 SQLite / Postgres" "Database" "store"
                }

                group "L1 可选服务与高级扩展 (Optional Add-ons)" {
                    edge_ingress = container "边缘接入层 (Edge Ingress)" "WASM/Workers 就近轻量接入 (可选)" "" "api"
                    nb_embed = container "POST /v1/embeddings等" "Embeddings / Images / Audio 等扩展 API" "" "api"
                    l1_websocket = container "流代理 (WebSocket/WebRTC)" "支持 OpenAI Realtime 等音频流" "" "api"
                    stream_guardrails = container "流式内联护栏 (Stream Safety)" "对输出流实时扫描阻断 (避免 TTFT 阻塞)" "" "api"
                    semantic_cache = container "语义匹配缓存 (Semantic Cache)" "向量库相似度匹配与缓存 (可选增强)" "" "api"
                    batch_worker = container "异步批处理 (Batch Worker)" "处理大规模离线批量调用" "" "api"
                }

                group "网关主干与内部封装层" {
                    embedded_sdk = container "内部基座 (Embedded SDK)" "网关底层核心引擎调用契约封装" "" "frozen"
                    gateway = container "网关核心引擎 (Gateway)" "协议调度中枢与生命周期管理" "" "frozen"
                }

                group "可选插件集 (远期演进)" {
                    productplugins = container "产品插件 (Product Plugins)" "网关与基座功能扩展" "" "plugin"
                    capabilitypacks = container "能力包 (Capability Packs)" "Embedding/Image/Realtime 支持" "" "plugin"
                    providerpacks = container "提供商包 (Provider Packs)" "大厂提供商原生 Rust 适配包" "" "plugin"
                    authplugins = container "认证插件 (Auth Plugins)" "特定提供商原生鉴权" "" "plugin"
                    wasm_plugins = container "WASM 沙箱 (WASM Plugins)" "不可信第三方扩展与局部变形 (远期可选)" "" "plugin"
                }

                group "L0 仓库边界与核心装配层 (AI-native)" {
                    repo_l0 = container "L0 转化内核代码仓库" "提供高性能调度内核" "" "repo"
                    catalog = container "能力自描述目录 (Catalog)" "Machine-first 的静态支持矩阵与模型能力字典" "" "frozen"
                    runtime_registry = container "运行时注册表 (Registry)" "基于 Catalog 衍生的动态装配状态" "" "frozen"
                    evidence_assets = container "兼容性与测试资产 (Evidence)" "验证能力支持矩阵与边界 (Golden Cases)" "" "repo"
                    config = container "内核配置 (Config)" "可变参数解析" "" "frozen"
                    providers = container "适配器组合绑定 (Providers)" "适配器注册表与组合式 Provider Binding" "" "frozen"
                    provider_transport = container "传输基座 (Transport)" "连接池 / 幂等键 / 退避重试" "" "frozen"
                    runtime = container "运行时装配 (Runtime)" "动态解析与派发" "" "frozen"
                    provider_options = container "厂商能力透传 (Options)" "Options Envelope 与原生能力透传双轨机制" "" "frozen"
                    session_transport = container "流与会话协商 (Session/Frame)" "Session/Frame 级流语义映射与 Realtime 协商" "" "frozen"
                }

                group "统一目标与原生协议映射 (分离互操作底线与一等面)" {
                    surf_proj_chat = container "统一转化终态 (Universal)" "对齐近期统一目标" "" "protocol"
                    surf_oai_chat = container "OpenAI 接口协议 (Chat)" "chat.completions 兼容面" "" "protocol"
                    surf_oai_responses = container "OpenAI Responses 协议" "原生 Agent/Responses 一等支持面" "" "protocol"
                    surf_gemini = container "Google 接口协议" "Gemini generateContent 原生" "" "protocol"
                    surf_anthropic = container "Anthropic 接口协议" "Claude Code / Messages API 原生" "" "protocol"
                }

                group "核心底座 (L0 最底层)" {
                    llmcore = container "大模型调用核心 (llm_core)" "模型无关的原子调用" "" "frozen"
                    contracts = container "机器优先契约层 (Contracts)" "Tool Calls/Usage 等统一结构化契约" "" "frozen"
                    foundation = container "底层支撑库 (Foundation)" "低分配缓冲 / Bytes 流分块 / 内存池" "" "frozen"
                }
            }
        }

        // =========================================================
        // 最底层：南向外部依赖 (各大厂 API 与外部企业基建)
        // =========================================================
        up_openai = softwareSystem "云端厂商 (OpenAI / Azure)" "GPT-4o / Azure OpenAI API" "ext"
        up_anthropic = softwareSystem "云端厂商 (Anthropic / CC)" "Claude 3.5 Sonnet / Opus 等" "ext"
        up_google = softwareSystem "云端厂商 (Google Gemini)" "Gemini 1.5 Pro / Flash 等" "ext"
        up_local = softwareSystem "本地/开源框架 (Local LLMs)" "vLLM / Ollama / Triton" "ext"
        up_more = softwareSystem "其他提供商 (Other APIs)" "Cohere / Mistral 等扩展池" "ext"

        up_idp = softwareSystem "企业身份源 (IdP)" "Active Directory / Okta" "ext"
        up_siem = softwareSystem "安全日志中心 (SIEM)" "Splunk / Datadog 等审计归档" "ext"
        up_approval = softwareSystem "企业硬审批系统" "企业微信 / 飞书 / 钉钉工作流" "ext"
        up_warehouse = softwareSystem "数据仓库 / 对象存储" "S3 / Snowflake (语料存储)" "ext"
        up_vcs = softwareSystem "代码托管平台 (VCS)" "GitHub / GitLab (GitOps 变更源)" "ext"
        up_ci = softwareSystem "CI/CD 流水线" "Jenkins / GitLab CI (自动化测试)" "ext"

        // =========================================================
        // 跨层连接关系 (彻底规范依赖，建立 Control Plane 边界)
        // =========================================================

        // 北向流量流入
        nb_ai_operator -> nb_models "能力探测"
        nb_ai_operator -> nb_health "健康检查"
        nb_ai_operator -> nb_chat "发起 AI 业务调用"
        nb_ai_operator -> l1_explain_simulate "调用 Explain 诊断与 Dry-run"
        nb_ai_operator -> l1_control_api "受限的自动化管控 (API)" "" "spi"

        nb_clients -> edge_ingress "[可选接入] 边缘节点调用" "" "spi"
        edge_ingress -> l1_http "回源"
        nb_clients -> nb_models "直连调用"
        nb_clients -> nb_chat "直连调用"
        nb_clients -> nb_embed "直连调用"
        nb_admins -> nb_health "探活"
        nb_admins -> admin_ui_lite "查看监控"
        nb_admins -> ent_portal "登录 L2 后台"

        // 依赖与集成边界 (解耦强依赖，厘清 SDK 分野)
        repo_l2 -> client_sdk "构建并打包给上层"
        repo_l2 -> embedded_sdk "内部集成使用底座契约"
        repo_l1 -> repo_l0 "依赖内核源码"
        repo_l2 -> ent_portal "构建出"
        repo_l1 -> l1_http "构建出"
        repo_l0 -> runtime "构建出"

        // L2 内部控制流与 IaC 驱动
        up_vcs -> gitops_engine "触发 PR 合并事件"
        up_ci -> ent_eval_audit "提交沙箱 Golden Cases 跑测"
        gitops_engine -> gateway_fleet_manager "驱动 L2 Bundle 灰度下发"

        ent_portal -> ent_identity "调度"
        ent_portal -> ent_govern "调度"
        ent_portal -> ent_finops "调度"
        ent_portal -> ent_prompt_hub "调度"
        ent_portal -> ent_eval_audit "调度"

        ent_govern -> policy_center "依赖"
        ent_govern -> approval_center "依赖"
        ent_finops -> billing_center "依赖"
        ent_eval_audit -> security_console "依赖"

        gateway_fleet_manager -> tenant_admin "统管集群"
        ent_apps -> agent "触发流程"
        agent -> mcp_hub "发现并调用标准 MCP 服务"
        agent -> rag_service "知识增强"
        agent -> approval_center "发起人工审核"

        agent -> agent_runtime "编排任务"
        workflowplugins -> agent_runtime "提供算子"

        ent_identity -> ent_store "存取"
        ent_govern -> ent_store "存取"
        ent_finops -> ent_store "存取"
        ent_prompt_hub -> ent_store "存取"
        ent_eval_audit -> ent_store "存取"

        // 南向外部系统集成 (精确切分)
        connector_hub -> up_idp "接入身份源"
        connector_hub -> up_siem "对接安全归档"
        approval_center -> up_approval "打通企微飞书审批"
        warehouse_export -> up_warehouse "导出微调语料"

        // ====================================================================
        // L2 到 L1 的控制面注入 (核心修正：单向单点写入 + 紧急直连通道)
        // ====================================================================
        ent_identity -> gateway_fleet_manager "提交租户与授权配置"
        policy_center -> gateway_fleet_manager "提交降级策略与权重 Bundle"
        ent_prompt_hub -> gateway_fleet_manager "提交发布态 Prompt"
        security_console -> gateway_fleet_manager "提交前置拦截策略"
        ent_eval_audit -> gateway_fleet_manager "提交路由镜像规则"

        gateway_fleet_manager -> l1_control_api "【唯一常规通道】统一下发版本化只读快照配置"
        ent_portal -> l1_control_api "【紧急通道 / Break-glass】绕过管控直连修改配置" "" "spi"

        // Agent 调用链路 (使用客户端 SDK)
        agent_runtime -> client_sdk "作为 HTTP 客户端调用标准接口"
        client_sdk -> nb_chat "封装并发送对话请求"

        // 反向审计与计费流
        billing_center -> usage_export "收集账单 Token 推流 (含避归核算)"
        security_console -> l1_event_export "消费结构化审计与管控变更事件"

        // L1 Control Plane 到内部组件的同步执行
        l1_control_api -> l1_instance_registry "更新实例表"
        l1_control_api -> l1_user_mgmt "刷新本地权限快照"
        l1_control_api -> l1_policy "下发策略管道约束条件"
        l1_control_api -> l1_prompt_mgmt "更新本地 Prompt"
        l1_control_api -> route_mgmt "重载规则条件与基础权重"
        l1_control_api -> sync_guardrails "更新同步硬阻断规则"
        l1_control_api -> stream_guardrails "更新流式正则匹配规则"
        l1_control_api -> l1_explain_simulate "为配置变更提供 Dry-run 校验"
        l1_control_api -> l1_event_export "推送控制面变更事件"
        admin_ui_lite -> l1_control_api "读取配置与监控指标"
        route_mgmt -> l1_explain_simulate "支持路由 Explain 解析与调试"

        // L1 核心内部管线处理 (Data Plane 转发)
        nb_models -> l1_http "路由"
        nb_chat -> l1_http "路由"
        nb_embed -> l1_http "路由"
        nb_health -> l1_http "路由"
        nb_chat -> l1_websocket "[协议升级] WebSocket/WebRTC (可选)" "" "spi"

        l1_http -> gateway "请求分发"
        l1_websocket -> gateway "流式分发"

        // 网关主干调度链 (厘清主干与 SPI 可选插件的关系)
        gateway -> l1_auth "1. 快速鉴权"
        gateway -> l1_user_mgmt "2. 查本地快照授权"
        gateway -> sync_guardrails "3. 前置硬性快速拦截"
        gateway -> exact_cache "4. 高速精确代理缓存命中"
        gateway -> l1_policy "5. 获取降级条件与熔断约束"

        l1_policy -> route_mgmt "输出路由约束条件"
        gateway -> route_mgmt "6. 依据规则与约束评分选路"

        gateway -> l1_prompt_mgmt "7. 运行时拉取并组装模板"

        // SPI 可选扩展注入点
        gateway -> semantic_cache "[SPI/可选增强] 向量语义缓存命中" "" "spi"
        gateway -> stream_guardrails "[SPI/可选增强] SSE 流式内联拦截" "" "spi"
        gateway -> batch_worker "[SPI/可选增强] 派发离线异步任务" "" "spi"

        gateway -> l1_obs "最后：记录 Trace"
        gateway -> l1_event_export "产生结构化生命周期事件"

        l1_obs -> usage_export "触发外发推流"
        exact_cache -> usage_export "避免 Token 成本计费"
        semantic_cache -> usage_export "避免 Token 成本计费"
        l1_auth -> key_mgmt "比对真实密钥"

        l1_auth -> l1_store "落库"
        l1_policy -> l1_store "落库"
        l1_obs -> l1_store "落库"
        l1_user_mgmt -> l1_store "落库"
        l1_prompt_mgmt -> l1_store "落库"
        l1_instance_registry -> l1_store "落库"

        // L1 / L0 上下贯穿 (内部封装层)
        embedded_sdk -> surf_proj_chat "依赖统一输出标准"
        gateway -> surf_proj_chat "依赖统一输出标准"

        // L0 内核处理逻辑
        surf_proj_chat -> runtime "移交运行时装配"

        // Catalog 源头地位验证
        catalog -> runtime_registry "初始化动态装配状态表"
        evidence_assets -> catalog "验证能力边界与矩阵"
        runtime -> catalog "查询静态模型能力字典"

        runtime -> runtime_registry "查询存活状态与动态能力"
        runtime -> config "读取动态配置"
        runtime -> provider_options "携带并解析扩展透传能力"
        runtime -> providers "派发给指定适配器组合"
        runtime -> contracts "依赖机器优先契约"
        runtime -> foundation "依赖低分配底座"

        providers -> session_transport "建立流式会话与帧协商"
        providers -> config "读取配置"
        providers -> provider_transport "网络退避与重试"
        providers -> provider_options "解析厂商透传选项"
        providers -> llmcore "执行核心处理"
        providers -> contracts "转化机器首选契约语义"
        providers -> foundation "依赖底座"

        // 协议面转化 (明确不视作独立进程)
        providers -> surf_oai_chat "映射到 OpenAI Chat 协议"
        providers -> surf_oai_responses "映射到 OpenAI Responses"
        providers -> surf_gemini "映射到 Gemini 协议"
        providers -> surf_anthropic "映射到 Anthropic 协议"

        surf_oai_chat -> up_openai "透传至 OpenAI / Azure"
        surf_oai_chat -> up_local "兼容请求至本地 OpenAI-compatible 引擎"
        surf_oai_responses -> up_openai "透传至 OpenAI / Azure"
        surf_gemini -> up_google "请求至 Google 云"
        surf_anthropic -> up_anthropic "请求至 Anthropic / CC"
        providerpacks -> up_more "可选扩展接入"

        // 底座依赖整理
        provider_transport -> foundation "依赖底座"
        session_transport -> foundation "依赖缓冲池"
        provider_options -> foundation "依赖"
        config -> contracts "依赖"
        config -> foundation "依赖"
        runtime_registry -> contracts "依赖"
        llmcore -> foundation "依赖"
        contracts -> foundation "依赖"

        // 插件系统挂载 (主视图中将被隐藏)
        productplugins -> gateway "增强"
        productplugins -> embedded_sdk "增强"
        providerpacks -> providers "注册原生 Rust 适配器"
        capabilitypacks -> runtime "注册多模态能力"
        authplugins -> providers "挂载认证机制"
        wasm_plugins -> runtime "沙箱隔离扩展"
    }

    // =========================================================
    // 分镜头视图定义
    // =========================================================
    views {

        // 视图 1：全局高层架构
        container aiGateway "View_01_SystemContext" {
            include *
            autoLayout tb
            description "【全局概览】AI 网关与企业平台 - 宏观系统依赖上下文"
        }

        // 视图 2：核心运行时（终极紧凑版排版）
        container aiGateway "View_02_Runtime_Compact" {
            include *
            exclude repo_l2 repo_l1 repo_l0 evidence_assets up_ci up_vcs gitops_engine
            exclude workflowplugins productplugins capabilitypacks providerpacks authplugins wasm_plugins
            autoLayout tb
            description "【核心紧凑图】分离了默认主干与 SPI 可选增强，呈现规整的数据面瀑布流，明确 Catalog 核心与 SDK 分野"
        }

        // 视图 3：专场 - L2 企业商业化平台（聚光灯打在控制面下发、客户端 SDK 集成上）
        container aiGateway "View_03_L2_Focus" {
            include nb_admins nb_clients nb_ai_operator ent_portal gitops_engine up_vcs up_ci gateway_fleet_manager tenant_admin ent_identity ent_govern policy_center approval_center ent_finops billing_center security_console ent_prompt_hub ent_eval_audit ent_apps agent_runtime agent mcp_hub rag_service connector_hub warehouse_export ent_store up_idp up_siem up_approval up_warehouse l1_control_api l1_event_export usage_export nb_chat client_sdk
            autoLayout tb
            description "【L2 专场】解决多写者冲突：聚焦 Fleet Manager 唯一快照下发，保留了所有复杂业务合规、治理流程及 Agent 编排"
        }

        // 视图 4：专场 - L1/L0 基础网关、透传机制与协议双轨
        container aiGateway "View_04_L1_L0_Focus" {
            include nb_clients nb_ai_operator edge_ingress nb_models nb_chat nb_embed nb_health l1_http l1_websocket l1_control_api l1_explain_simulate l1_event_export admin_ui_lite l1_instance_registry l1_user_mgmt key_mgmt l1_prompt_mgmt l1_auth l1_policy route_mgmt sync_guardrails stream_guardrails exact_cache semantic_cache l1_obs usage_export batch_worker l1_store embedded_sdk gateway catalog runtime_registry config providers provider_transport session_transport provider_options runtime surf_proj_chat surf_oai_chat surf_oai_responses surf_gemini surf_anthropic llmcore contracts foundation up_openai up_google up_anthropic up_local up_more evidence_assets
            autoLayout tb
            description "【L1/L0 专场】展现 AI-Native 面貌：L1 Snapshot + Explain/Simulate + Event Export，L0 Machine-first Contracts 与能力自描述透传"
        }

        styles {
            element "Person" {
                shape Person
                background #2A4365
                color #ffffff
            }
            element "SoftwareSystem" {
                background #1A365D
                color #ffffff
                shape RoundedBox
            }
            element "Database" {
                shape Cylinder
            }
            element "repo" {
                background #e3f2fd
                color #1565c0
                stroke #1565c0
                shape Folder
            }
            element "api" {
                background #e8f5e9
                color #2e7d32
                stroke #2e7d32
            }
            element "enterprise" {
                background #ffebee
                color #c62828
                stroke #c62828
            }
            element "frozen" {
                background #f5f5f5
                color #424242
                stroke #424242
            }
            element "ext" {
                background #fff3e0
                color #ef6c00
                stroke #ef6c00
            }
            element "store" {
                background #fff8e1
                color #f9a825
                stroke #f9a825
            }
            element "plugin" {
                background #f3e5f5
                color #7b1fa2
                stroke #7b1fa2
                shape Component
            }
            element "protocol" {
                shape Hexagon
                background #ffffff
                color #d84315
                stroke #d84315
            }
            relationship "spi" {
                color #9e9e9e
                style Dashed
            }
        }
    }
}
