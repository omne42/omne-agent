# 任务：external-toolchain-installer

## 相关文档与用途

- `openspec/changes/external-toolchain-installer/proposal.md`：目标、边界与非目标。
- `docs/start.md`：全链路目标口径。
- `crates/toolchain-runtime/src/lib.rs`：待收敛的内置安装实现。

## 1. 文档与仓库准备

- [ ] 在 `p/` 下创建独立仓库 `toolchain-installer`。
- [ ] 新仓库补齐 README（做什么/为什么做/怎么做/验收标准）。
- [ ] 新仓库补齐接口契约文档（CLI 入参、JSON 输出、退出码）。
- [ ] 新仓库文档明确“调用方无关（caller-agnostic）”边界与可复用范围。
- [ ] 新仓库补齐安全边界文档（白名单、反滥用、限流策略）。

## 2. 新仓库实现（安装器）

- [ ] 实现 `toolchain bootstrap` 能力（至少覆盖 `git`、`gh`）。
- [ ] 实现平台识别与目标资产选择（Linux/macOS/Windows）。
- [ ] 实现公共来源候选顺序与失败重试。
- [ ] 实现完整性校验（哈希或等价可验证机制）。
- [ ] 输出稳定 JSON 结构（供任意调用方消费，`omne-agent` 仅是其中之一）。

## 3. 新仓库实现（可选网关）

- [ ] 提供 Cloudflare Worker 路由实现（固定路径参数）。
- [ ] 禁止任意 URL 代理（不支持 `?url=` 类参数）。
- [ ] 仅允许白名单工具、版本与目标平台组合。
- [ ] 默认返回重定向响应，避免转发大文件流量。

## 4. omne-agent 接入

- [ ] `toolchain-runtime` 优先调用外部安装器。
- [ ] 保留 `omne toolchain bootstrap --json` 关键字段兼容。
- [ ] 无外部安装器时给出明确错误/回退说明。
- [ ] 保证 `crates/app-server` 不引入安装逻辑。

## 5. 测试与验证

- [ ] 新仓库单元测试通过。
- [ ] 新仓库端到端测试通过（mock 下载源 + 安装落盘）。
- [ ] 新仓库调用方契约测试通过（最小模拟调用方消费 JSON）。
- [ ] `omne-agent` 单元测试与集成测试通过。
- [ ] `cargo check -p omne` 与 `cargo test -p omne` 通过。

## 6. DoD 自检命令

- [ ] 外部安装器接入扫描：  
  `rg -n "toolchain-installer|OMNE_TOOLCHAIN_INSTALLER" crates/toolchain-runtime crates/agent-cli`
- [ ] 调用方无关约束扫描（应无输出）：  
  `rg -n "only for omne-agent|omne-agent only|专供 omne-agent|专属 omne-agent" /root/autodl-tmp/zjj/p/toolchain-installer`
- [ ] app-server 边界扫描（应无输出）：  
  `rg -n "install_(git|gh)|toolchain bootstrap|fetch_latest_github_release|download_with_candidates" crates/app-server`
- [ ] 反滥用关键字扫描：  
  `rg -n "allowlist|whitelist|no open proxy|no \\?url=|rate limit|redirect" /root/autodl-tmp/zjj/p/toolchain-installer`
- [ ] CLI 契约字段扫描：  
  `rg -n "\"schema_version\"|\"target_triple\"|\"items\"|\"status\"" /root/autodl-tmp/zjj/p/toolchain-installer`
