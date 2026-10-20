# Trae CLI / CodeBuddy CLI / Qoder CLI 完整支持落地方案

> 版本：v1.0（2026-10-04）
> 状态：方案稿（评审中）
> 适用：AI Work Assistant（Tauri 2.x + Rust + React）桌面端

---

## 0. TL;DR

| CLI | 现状 | 目标 | 核心缺口 | 工作量 |
|---|---|---|---|---|
| CodeBuddy CLI | ✅ 完整读写桥 + 五重防护自动轮换（F-06/F-59） | 增强：`CODEBUDDY_API_KEY` 通道、`CODEBUDDY_INTERNET_ENVIRONMENT` 探测、apiKeyHelper 兜底 | 小（增强项） | S |
| Qoder CLI | ⚠️ 仅只读 status 桥（F-80 M4，R-3 裁决） | 升级：PAT 环境变量切号桥（`QODER_PERSONAL_ACCESS_TOKEN`），复用 decide_target 架构 | 中（新写命令层，需复核 R-3） | M |
| Trae CLI | ❌ 无 CLI 桥（仅 IDE/Work 切换 + 网关间接覆盖） | 新建：`TRAECLI_PERSONAL_ACCESS_TOKEN` 切号桥 + status 只读桥，完整对齐 CodeBuddy CLI 桥架构 | 大（新模块） | M-L |

统一原则：**凭证只进 Rust 内存与目标配置文件，不进日志/事件/UI/DB 明文**；切号桥与状态桥分离；所有切换复用同一套五重防护（`decide_target` 纯函数）。

---

## 1. 项目现状分析

### 1.1 现有支持盘点

**（1）CodeBuddy CLI 切号桥 —— 已完整（能力最强）**

- 位置：
  - 纯逻辑层 [workbuddy_cli.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/workbuddy_cli.rs)（427 行，13 个单测）
  - 有状态粘合层 [commands/workbuddy/cli.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/workbuddy/cli.rs)（375 行）
- 能力清单：
  - 写 `~/.codebuddy/settings.json` 的 `env.CODEBUDDY_AUTH_TOKEN`（`with_env_token` / `settings_env_token`），写前备份、写后回读校验、失败原子回滚（`restore_cli_settings`）
  - CLI 活动探测：递归扫 `~/.codebuddy/projects/**/*.jsonl` 取最新 mtime（`cli_recent_activity_at`）
  - **五重防护自动轮换**（`decide_target` 纯函数）：有效候选过滤 → 紧迫度排序 → 紧迫阈值 → 当前即目标 → 冷却期 → 活跃保护 → 最小剩余 → 防横跳
  - 轮换状态持久化：SQLite kv 表键 `wb_cli_rotate_state`；后台线程 `start_cli_rotate_thread()`
  - Tauri 命令：`workbuddy_cli_status` / `workbuddy_cli_bridge_set(userId)` / `workbuddy_cli_rotate_run` / `workbuddy_cli_rotate_logs(limit)`

**（2）Qoder CLI 状态桥 —— 只读（受 R-3 裁决约束）**

- 位置：[commands/qoder/cli_status.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/qoder/cli_status.rs)（88 行）
- 能力：只读解析 `~/.qoder-cn/.qoder-app-status.json`（`logged_in / name / avatar_url / version / snapshot_at` 等白名单字段），文件缺失/损坏返回 `available=false + reason`
- 明确不做（R-3 裁决）：不写回、不注入凭证 —— 「CLI 无独立凭证通道，切号仍走 IDE 存储快照管线」
- 注意：**R-3 裁决的前提（CLI 无独立凭证通道）已随 Qoder CLI 官方推出 PAT 登录而失效**，本方案第 4 节提出复核申请

**（3）CC Switch 协同 —— 间接支持**

- 位置：[commands/ccswitch.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/ccswitch.rs)
- 把本项目网关注册进 `~/.cc-switch/cc-switch.db` providers 表（固定 id `aiwork-gateway-*` / `aiwork-wb-gateway-*`），服务 Claude Code / Codex CLI。与三个目标 CLI 无直接关系，但可复用其「注册外部工具配置」的模式。

### 1.2 差距总结

| 能力 | CodeBuddy CLI | Qoder CLI | Trae CLI |
|---|---|---|---|
| 登录态读取 | ✅ settings.json env | ✅ status.json 只读 | ❌ |
| 凭证写入 | ✅ env.AUTH_TOKEN | ❌（R-3 禁止） | ❌ |
| 自动轮换 | ✅ 五重防护 | ❌ | ❌ |
| CLI 活动探测 | ✅ projects/*.jsonl | ❌ | ❌ |
| 前端状态面板 | ✅ | ✅（只读） | ❌（仅 IDE/Work） |

---

## 2. 三个 CLI 官方认证机制（外部调研结论）

### 2.1 Trae CLI（traecli）

- **PAT 登录令牌**：企业版 CLI 登录令牌，前缀 `trae-lt-`（Trae Login Token）
- **环境变量**：
  - `TRAECLI_PERSONAL_ACCESS_TOKEN` —— 主认证通道（PAT）
  - `TRAECLI_HOST` —— 可选，指定接入点/企业域名
- **全局配置文件**：`trae_cli.yaml`（`traecli config edit` 编辑），可存放凭证与代理设置
- **MCP 支持**：可作为 MCP 客户端接入
- 参考（GitHub）：`IQ-Kat/birouter` 的 PROVIDER_REFERENCE.md 中 trae 条目 —— Cloud-IDE-JWT，有效期约 14 天，需定期刷新；说明社区已在做 Trae 凭证托管/轮换，模式与本项目 CodeBuddy 桥同构。

### 2.2 CodeBuddy CLI

- **认证优先级**：`CODEBUDDY_AUTH_TOKEN` > `apiKeyHelper` > `CODEBUDDY_API_KEY`
- **settings.json 分层配置**：全局 `~/.codebuddy/settings.json` + 项目级 `.codebuddy/settings.json`；`env` 字段注入环境变量（本项目当前即利用此机制）
- **必配项**：`CODEBUDDY_INTERNET_ENVIRONMENT`（区分内网/公网环境）—— 当前桥未处理，增强点
- **会话痕迹**：`~/.codebuddy/projects/**/*.jsonl`（项目已用于活动探测）
- 参考（GitHub）：birouter 中 codebuddy-cn 条目 —— 走官方 CLI device-code flow 换取凭证后托管刷新。

### 2.3 Qoder CLI（qodercli）

- **PAT 登录**：环境变量 `QODER_PERSONAL_ACCESS_TOKEN`，**env 优先于 `/login` 本地凭证** —— 这是本方案的关键依据：写 env 即可覆盖登录态，无需触碰 IDE 存储快照
- **配置目录**：`~/.qoder`（`QODER_CONFIG_DIR` 可重定向）；`~/.qoder/settings.json` 存放如 `general.enableAutoUpdate` 等设置
- **状态面板**：`/status`；**ACP 支持**：`--acp` 可作为 Agent Client Protocol 服务端
- 注意：本项目当前解析的是 `~/.qoder-cn/.qoder-app-status.json`（IDE 侧状态文件），CLI 侧登录态与 IDE 侧是两条线 —— 新桥读 CLI 侧、写 env，与 R-3 约束的「IDE 快照管线」正交。

### 2.4 对照表

| 机制 | Trae CLI | CodeBuddy CLI | Qoder CLI |
|---|---|---|---|
| 主认证通道 | `TRAECLI_PERSONAL_ACCESS_TOKEN`（`trae-lt-`） | `CODEBUDDY_AUTH_TOKEN` | `QODER_PERSONAL_ACCESS_TOKEN` |
| env 优先级 | env 优先 | env 最高（> apiKeyHelper > API_KEY） | env > /login 凭证 |
| 配置文件 | `trae_cli.yaml` | `~/.codebuddy/settings.json`（env 字段） | `~/.qoder/settings.json` |
| 状态文件 | 暂无公开 | projects/*.jsonl（会话） | `.qoder-app-status.json`（IDE 侧，已接入） |
| 令牌形制 | `trae-lt-` 前缀长期令牌 | OAuth token（短期，需轮换） | PAT（长期） |

**共性结论**：三家都支持「环境变量注入凭证」且 env 均为最高/优先通道 → **统一采用「写配置文件 env 字段 + 复用五重防护轮换」的同一架构**，与现有 CodeBuddy 桥完全同构，工程上是一次架构复制而非重新设计。

---

## 3. 落地方案 —— CodeBuddy CLI（增强项）

现有桥已完整，只做三处小增强：

### 3.1 `CODEBUDDY_INTERNET_ENVIRONMENT` 探测与保底

- `apply_cli_token` 写入 env 时，若 settings.json 中无 `CODEBUDDY_INTERNET_ENVIRONMENT` 且系统环境变量也没有，则按当前账号池来源（公网账号）自动补写 `"INTERNET"`
- 归属：`workbuddy_cli.rs` 新增纯函数 `ensure_internet_env(settings: &mut Value) -> bool`（返回是否补写，可单测）

### 3.2 通道健康自检

- `workbuddy_cli_status` 返回体新增 `auth_channel` 字段：`env_auth_token | api_key_helper | api_key | unknown`（按官方优先级探测，token 值脱敏为 `present/absent`）
- 用途：让前端能显示「当前轮换是否真正生效」——若用户在 apiKeyHelper 层挂了自定义脚本，env 覆盖会失效，需要提示

### 3.3 apiKeyHelper 兜底（可选，后置）

- P3：提供「生成 apiKeyHelper 脚本」向导（脚本内部指向本项目网关动态取 token），覆盖「用户不希望明文 token 落 settings.json」的场景
- 风险：脚本内容即凭证出口，需过凭证红线评审，暂列观察项

---

## 4. 落地方案 —— Qoder CLI（只读桥 → PAT 切号桥）

### 4.1 R-3 裁决复核（前置）

R-3 原文理由「CLI 无独立凭证通道」已被官方 PAT 登录推翻。复核结论建议：

- **维持**：不触碰 `~/.qoder-cn/.qoder-app-status.json` 写路径（IDE 快照管线不动）
- **放行**：在 `~/.qoder/settings.json` 的 env 注入 `QODER_PERSONAL_ACCESS_TOKEN`（CLI 独立配置目录，正交于 IDE 侧）
- 落地为新的 R-3a 裁决条目，写回 AGENT.md §5.4

### 4.2 架构（复制 CodeBuddy 桥骨架）

```
src-tauri/src/qoder_cli.rs          # 纯逻辑层（新）
  const AUTH_ENV_KEY = "QODER_PERSONAL_ACCESS_TOKEN"
  const SETTINGS_DIR = ".qoder"        // 注意：区别于 IDE 侧 .qoder-cn
  settings_env_token / with_env_token  // 与 workbuddy_cli.rs 同签名
  decide_target                        // 直接复用 workbuddy_cli::decide_target（泛化提公共模块，见 4.4）
src-tauri/src/commands/qoder/cli_bridge.rs  # 有状态粘合层（新）
  qoder_cli_status        // 现有 cli_status.rs 保留（IDE 侧）+ 新增 CLI 侧登录探测（PAT present?）
  qoder_cli_bridge_set(userId)
  qoder_cli_rotate_run
  qoder_cli_rotate_logs(limit)
```

### 4.3 关键实现点

1. **settings.json 写入**：`~/.qoder/settings.json`。若不存在则创建（JSON，非 yaml）；写入前备份到 `~/.qoder/settings.json.aiwork-bak`，写后回读校验，失败回滚 —— 完全照搬 `apply_cli_token` 流程
2. **PAT 来源**：账号池条目需新增 `qoder_pat` 字段（登录抓包/手动录入）。PAT 为长期令牌，**无过期时间 → 轮换紧迫度排序退化为「按总余额排序」**，`soonest_expire_at_ms = None` 走现有空值分支
3. **CLI 活动探测**：Qoder CLI 无公开会话文件目录。替代方案：扫描 `~/.qoder/` 下 mtime 最新的非 settings 文件；探测不到则活动保护自动降级为不启用（decide_target 已支持 `cli_recent_activity_ms: None`）
4. **状态展示**：前端 Qoder 面板增加「CLI 侧」标签页：PAT 是否就位、当前生效账号、上次轮换时间

### 4.4 决策逻辑公共化（顺带重构）

`decide_target` 已被 CodeBuddy 桥验证，Qoder 桥直接复用。建议把 `decide_target` + `CliCandidate` + `RotateDecision` 提取到 `src-tauri/src/cli_rotate_core.rs`（新公共模块），workbuddy_cli.rs 与 qoder_cli.rs 均引用 —— 避免复制 300 行决策逻辑，也为第 5 节 Trae 桥铺路。

---

## 5. 落地方案 —— Trae CLI（新建桥，从零到完整）

### 5.1 架构

```
src-tauri/src/trae_cli.rs            # 纯逻辑层（新）
  const AUTH_ENV_KEY = "TRAECLI_PERSONAL_ACCESS_TOKEN"
  const HOST_ENV_KEY = "TRAECLI_HOST"
  // Trae 配置为 yaml（trae_cli.yaml），非 JSON —— 本桥主通道走「进程级/用户级环境变量」，
  // 不解析 yaml，见 5.3 的通道选型论证
  decide_target                       // 引用 cli_rotate_core.rs
src-tauri/src/commands/trae/cli_bridge.rs  # 有状态粘合层（新）
  trae_cli_status / trae_cli_bridge_set / trae_cli_rotate_run / trae_cli_rotate_logs
```

### 5.2 通道选型（关键论证）

Trae CLI 的 `trae_cli.yaml` 是用户手编辑的配置文件，**没有 CodeBuddy 式的 `env` 字段注入机制**，写入 yaml 需要引入 yaml 解析依赖且容易与用户手改冲突。而官方 PAT 本身就是为环境变量设计的（`TRAECLI_PERSONAL_ACCESS_TOKEN`）。

**结论：Trae 桥采用「用户级环境变量（Windows 用户注册表环境 / setx 等价 API）」作为写通道**，即本项目把 `TRAECLI_PERSONAL_ACCESS_TOKEN` 写入 Windows 当前用户的用户环境变量。对比：

| 方案 | 优点 | 缺点 | 采纳 |
|---|---|---|---|
| 写 `trae_cli.yaml` | 与 CodeBuddy 桥形态一致 | 需 yaml 依赖、用户手改冲突、无官方 env 字段约定 | ❌ |
| 写用户级环境变量 | 官方支持通道、无需解析 yaml、对所有终端新会话生效 | 已开终端不生效（需提示重启终端） | ✅ |
| 写进程级 env（由本项目拉起 CLI） | 最干净、隔离最好 | 用户通常自己在终端里跑 traecli，本项目拉不到 | ⚠️ 仅作网关代理场景 |

Windows 实现要点：`std::env::set_var` 只影响本进程；用户级持久化用 `winreg` crate 写 `HKCU\Environment`（`TRAECLI_PERSONAL_ACCESS_TOKEN`，REG_SZ），并广播 `WM_SETTINGCHANGE` 使 Explorer 感知 —— 新开终端即生效。写前读旧值备份到 kv 表，支持回滚。

### 5.3 实现步骤

1. **账号池扩展**：账号条目新增 `trae_pat`（`trae-lt-` 前缀校验，格式不符拒绝入池）
2. **纯逻辑层** `trae_cli.rs`：
   - `registry_token() -> Option<String>`（读 HKCU 现值）
   - `set_registry_token(token) / clear_registry_token()`（winreg + WM_SETTINGCHANGE）
   - `with_host(host: Option<&str>)`（企业版需同步写 `TRAECLI_HOST`）
   - 复用 `cli_rotate_core::decide_target`
3. **命令层** `commands/trae/cli_bridge.rs`：
   - `trae_cli_status`：PAT 就位状态（present/absent + 前缀校验）、当前指向账号、冷却/保护窗口
   - `trae_cli_bridge_set / rotate_run / rotate_logs`：与 CodeBuddy 同契约
   - 轮换状态 kv 键：`tr_cli_rotate_state`（与 `wb_cli_rotate_state` 隔离）
4. **前端**：设置页新增「Trae CLI 桥」卡片，与 CodeBuddy CLI 桥卡片同构（开关/目标账号/手动轮换/日志）
5. **提示文案**：写入后提示「新开终端生效」；提供「验证」按钮，运行 `traecli config show`（或等价只读命令）回显 host 摘要验证注入成功

### 5.4 风险与缓解

| 风险 | 缓解 |
|---|---|
| 用户级 env 写入影响全局（其他工具同名变量） | 写前备份旧值到 kv 表；卸载/关闭桥时恢复 |
| `trae-lt-` 令牌与企业版绑定，社区版账号无 PAT | 入池校验前缀；桥状态显示「该账号不支持 CLI PAT」 |
| 令牌 14 天有效（birouter 观测的 Cloud-IDE-JWT 类似行为） | PAT 形制为长期令牌；仍接入轮换的过期候选过滤，取得到 expiry 则参与排序，取不到按余额排序 |
| HKCU 写入需要权限 | 用户级注册表无需管理员；写失败降级为「仅提示手动 setx」 |

---

## 6. 统一架构与分期计划

### 6.1 目标架构

```
cli_rotate_core.rs（公共决策）      ← decide_target / CliCandidate / RotateDecision（从 workbuddy_cli.rs 提取）
     ↑                  ↑                 ↑
workbuddy_cli.rs    qoder_cli.rs      trae_cli.rs        ← 各 CLI 凭证通道适配层（写 env / 写注册表）
     ↑                  ↑                 ↑
commands/workbuddy/cli.rs  commands/qoder/cli_bridge.rs  commands/trae/cli_bridge.rs  ← 命令粘合层（账号池/状态/线程）
     └───────────────────┼────────────────────┘
              前端设置页「CLI 切号桥」统一卡片（三个 tab）
```

### 6.2 分期计划

| 期 | 内容 | 依赖 |
|---|---|---|
| M1 | `cli_rotate_core.rs` 提取 + CodeBuddy 桥改引用（回归 13 个单测） | 无 |
| M2 | CodeBuddy 增强项（INTERNET_ENV 保底 + auth_channel 自检） | M1 |
| M3 | Qoder PAT 桥（R-3a 复核落地 → qoder_cli.rs + 命令层 + 前端 tab） | M1；账号池 qoder_pat 字段 |
| M4 | Trae 桥（winreg 通道 + 命令层 + 前端 tab） | M1；账号池 trae_pat 字段 |
| M5（可选） | apiKeyHelper 向导、Trae 网关代理场景（进程级 env） | M4 |

### 6.3 红线与验收（全期通用）

- 凭证红线：token/PATH 值不进日志、不进 NDJSON 事件、不进前端 UI（只显示 present/absent + 尾 4 位）、不进 SQLite 明文（账号池凭证字段沿用现有 token store 加密通道）
- 所有写入（settings.json / 注册表）写前备份、写后回读校验、失败回滚
- 命令契约命名对齐现有 `workbuddy_cli_*` 风格：`qoder_cli_*`、`trae_cli_*`
- 每个纯逻辑层模块单测覆盖 decide_target 各分支 + 通道读写回滚分支
- AGENT.md 同步更新：§5 命令契约、§5.4 Qoder 约定（R-3 → R-3a）、§8 切换器约定新增两桥条目

---

## 7. 参考项目与出处

- **birouter**（GitHub: IQ-Kat/birouter）：统一多 provider 凭证路由器（237+ providers），其 PROVIDER_REFERENCE.md 含 trae（Cloud-IDE-JWT ~14d）/ codebuddy-cn（官方 CLI device-code flow）/ qoder（OAuth）条目 —— 证明三家凭证托管/轮换在社区已可行，且「本地代理/env 注入 + 定期刷新」是通行模式
- Trae CLI 官方文档（docs.trae.cn）：CLI 登录令牌（`trae-lt-`、`TRAECLI_PERSONAL_ACCESS_TOKEN`、`TRAECLI_HOST`）、`traecli config edit`
- CodeBuddy CLI 官方文档（codebuddy.ai）：认证优先级、settings.json 分层与 env 注入、`CODEBUDDY_INTERNET_ENVIRONMENT`
- Qoder CLI 官方文档（docs.qoder.com）：PAT env 优先、`~/.qoder` 配置目录、ACP 支持
