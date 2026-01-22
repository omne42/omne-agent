# Daemon（常驻 server）vs 每次启动一个 `pm-app-server`（进程模型取舍）

> 问题背景：如果用户同时在很多工作目录里用 `pm`，当前“每次启动一个 `pm-app-server` 子进程（stdio JSON-RPC）”会带来明显的内存与启动开销；50 个项目就可能变成 50 份 tokio runtime + 事件索引 + 缓存。

本文只讨论**进程模型**，不讨论强隔离（我们明确不需要强隔离）。

---

## 1) 方案 A：每个 `pm` 启动一个 `pm-app-server`（当前形态）

### 优点

- **实现简单**：client/server 生命周期绑定；不需要 daemon discovery、版本协商、热升级、health check。
- **少一层安全面**：没有常驻端口/套接字；不需要额外的鉴权/ACL（本机上仍建议最小化暴露面）。
- **调试直观**：挂了就重启；日志/事件目录都在当前项目 `pm_root` 下，定位简单。

### 缺点

- **启动开销高**：每次命令都要拉起完整 runtime，延迟明显。
- **内存无法复用**：50 个项目 ≈ 50 个后端进程；即便空闲也占内存。
- **watch/tui 体验受限**：要持续观察就必须保持该进程活着；多项目同时 watch 会把问题放大。

---

## 2) 方案 B：常驻 daemon（OpenCode 风格：一个 server，多 client attach）

### 优点

- **内存与启动成本摊薄**：一个进程复用 runtime/连接池/解析器/索引；多项目并发主要是“数据量”和“IO”，而不是重复的进程开销。
- **更像“控制面”**：天然支持多 client（TUI/REPL/脚本）同时 attach，同一 thread 的事件订阅也更自然。
- **更容易做后台能力**：例如长时间跑的 hooks、持续 tail/follow、stuck 检测等。

### 缺点

- **复杂度上升**：需要 discovery（socket 放哪）、生命周期（启动/停止/重启/升级）、客户端重连、并发访问（锁/一致性）。
- **跨项目边界必须硬防**：daemon 一旦支持多 `pm_root`，就需要把“每个 thread 的根目录/可写根”当成一等公民，否则就是安全事故（即便我们不追求强隔离）。
- **配置层级更棘手**：同一 daemon 服务多个项目时，必须明确每个请求/每个 thread 使用哪套配置（否则会把 A 项目的 key 用到 B 项目上）。

---

## 3) 当需要同时服务 50 个项目时，谁更占优势？

结论：**daemon 优势更大**，原因很简单：

- 50×“进程固定开销”远大于 50×“项目级数据目录”的开销（目录本来就要存在）。
- 我们不需要强隔离，真正要硬防的是**路径边界与配置归属**；这更适合在一个常驻控制面里统一实现并审计。

---

## 4) 配置与目录约束（必须写死的前提）

- 项目级覆盖配置统一放在 `./.codepm_data/`（见 `docs/codepm_data.md`）：
  - `.codepm_data/config.toml`：可提交；必须显式 `[project_config].enabled = true` 才生效
  - `.codepm_data/.env`：secrets；必须 gitignore；必须在 file tools 层默认拒绝读取
- 推荐的优先级（从高到低）：
  1. thread 级覆盖（`thread/configure`）
  2. project 覆盖（`.codepm_data/config.toml` + `.codepm_data/.env`，且必须 enabled）
  3. 进程环境变量（env）
  4. 默认值

---

## 5) 推荐落地路径（不发明新语义）

核心原则：**不要为了 daemon 改协议语义**。只改变 transport：

- 继续以 `pm-app-server`（JSON-RPC + 事件流）为唯一控制面。
- `pm` client 优先连接本机 socket（daemon 模式），失败再 fallback 到“spawn 子进程（stdio）”。
- daemon 内部以 `pm_root`（项目的 `.codepm_data/`）为命名空间，ThreadStore/locks 都按 root 分区。

落地口径（v0.2.0 现状）：

- unix socket 路径：`<pm_root>/daemon.sock`
- 启动（前台常驻）：

```bash
$ pm-app-server --pm-root ./.codepm_data --listen ./.codepm_data/daemon.sock
```

- 客户端：`pm` 会默认尝试连接 `daemon.sock`；连接失败则退回到“每次 spawn 一个 `pm-app-server`”的行为（保持 JSON-RPC 语义不变）。
