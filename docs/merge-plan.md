# 三项目合并方案与实施记录（AI Work 中枢 / AIWorkHub）

> 基线：`TraeWorkAssistant`（AI Work 助手 v3.7.13，Tauri 2 + React 18 + Rust）
> 并入：`qoder2api-hub`（Qoder 单进程 Python 网关 v1.4.3）、`workbuddy2api-hub-v1.7.12`（WorkBuddy/wb-proxy）
> 新工程目录：`E:\study\gitproject\aiwork\AIWorkHub`
> 本文档同时是**合并决策依据**与**构建验收记录**。

---

## 1. 合并结论（一句话）

以 TraeWorkAssistant 的桌面外壳（多账号 / 登录态切换 / 设备隔离 / 三协议网关 / 定时任务）为**唯一宿主**，
把两个 Python hub 里「桌面端确实缺、且与宿主架构兼容」的算法与运维能力**回收为 Rust 模块**，
补齐宿主唯一缺失的入口能力——**一键导入其他智能体配置**，形成单一可构建产品。

不做的事（避免双真源 / 双进程）：

| 不做 | 理由 |
|---|---|
| 把 Python hub 作为子进程内嵌 | 两套 HTTP 栈、两套账号池、两套落盘会立刻产生「哪个是真值」的冲突；桌面端已有 axum 网关 |
| 把 `dashboard.html` 搬进 React | hub 的 dashboard 是单文件内联（4.8k / 14.7k 行），信息架构已由宿主各应用页覆盖，只回收指标口径 |
| 迁移 hub 的账号池文件格式 | 宿主 `commands/qoder/*`、`commands/workbuddy/*` 已有成熟账号模型 + 加密导出；hub 账号改由「额外目录」识别为上游网关接入 |
| 改 `productName` / `identifier` | 保持 `AI Work 助手` + `com.aiwork.assistant` 不变——原生原地升级依赖产品名一致，数据目录仍为 `%APPDATA%\AIWorkAssistant`，v3.7.x 用户升级后数据零迁移；版本号只升到 4.0.0 以标识能力线切换 |

---

## 2. 目录与来源映射

```
E:\study\gitproject\aiwork\
├── TraeWorkAssistant\        # 原基线（保持不动，作为对照与回滚）
├── qoder2api-hub\            # 能力来源（只读分析，未修改）
├── workbuddy2api-hub-v1.7.12\# 能力来源（只读分析，未修改）
└── AIWorkHub\                # ← 合并后的新工程（本次交付）
```

新增/改动的文件（均在 `AIWorkHub` 内）：

| 文件 | 类型 | 来源/依据 |
|---|---|---|
| `src-tauri/src/commands/agent_import.rs` | 新增 ~2.2k 行 | **强制能力**；无上游可抄，按各智能体真实配置格式实现 |
| `src-tauri/src/api_server/upstream_pool.rs` | 新增 | `wb_upstream_pool.py`（SWRR）+ `qoder_accounts.py`（三族退避） |
| `src-tauri/src/api_server/pricing.rs` | 新增 | `wb_pricing.py`（内容寻址策略 / 分档 / timeline 引用） |
| `src-tauri/src/api_server/activity.rs` | 新增 | `wb_activity.py`（白名单投影 + 脱敏 + 环形） |
| `src-tauri/src/api_server/modelsdev.rs` | 新增 | `wb_modelsdev.py`（四级查找，未知输出不编造） |
| `src-tauri/src/api_server/atrest.rs` | 新增 | `wb_atrest.py`（AES-256-GCM + 逐字段 AAD） |
| `src-tauri/src/commands/hub.rs` | 新增 | 上述模块的 Tauri 命令薄壳 + 出口/端点探测（`wb_ipintel.py` / `wb_probes.py`） |
| `src-tauri/src/api_server/custom_route.rs` | 改 | 接入上游组选点与预流式换端；成功请求记成本 |
| `src-tauri/src/api_server/mod.rs` | 改 | 注册 5 个新模块 |
| `src-tauri/src/commands/mod.rs` | 改 | 注册 `agent_import` / `hub` |
| `src-tauri/src/main.rs` | 改 | 注册 32 个新命令（智能体接入 10 + 枢纽增强 22） |
| `src-tauri/Cargo.toml` | 改 | 新增 `toml = "0.8"`（Codex 配置解析，免得手写 TOML 漏表形态） |
| `src/pages/agents/AgentHub.tsx` | 新增 | 强制能力页面 |
| `src/pages/hub/HubCenter.tsx` | 新增 | 增强包页面（4 个 Tab） |
| `src/lib/tauri.ts` / `src/types.ts` | 改 | `api.agents` / `api.hub` 封装 + 24 个 DTO |
| `src/components/Sidebar.tsx` / `src/App.tsx` | 改 | 全局菜单（不随应用 Tab 切换）+ 路由 case |

---

## 3. 强制能力：一键导入其他智能体配置

### 3.1 为什么必须新增

宿主原能力只有**单向**：`commands/ccswitch.rs` 把本网关注册进 CC Switch 的 provider 表。
缺的是**反向**：把本机其它智能体已经配好的上游（中转站 / 官方 Key / 兄弟网关）一次性收进本机资源池，
并把本网关写回目标客户端。这正是本次合并的硬性要求。

### 3.2 覆盖矩阵（`AGENTS` 注册表，13 个）

| 智能体 | 读取路径（`~`/`%APPDATA%` 展开） | 解析要点 | 写回 |
|---|---|---|---|
| Claude Code | `~/.claude/settings.json`、`~/.claude.json`、`…/Claude/claude_desktop_config.json` | `env.ANTHROPIC_BASE_URL/AUTH_TOKEN/API_KEY/MODEL` + 顶层扁平写法 + `mcpServers` | ✅ `env` 段合并 |
| Codex CLI | `~/.codex/config.toml`、`~/.codex/auth.json` | `model`/`model_provider`/`[model_providers.*]`（+ `env_key` 取环境变量真值）+ MCP | ✅ `config.toml` 行级合并 + `auth.json` |
| Gemini CLI | `~/.gemini/settings.json`、`~/.gemini/.env` | `.env` 的 `GEMINI_BASE_URL/OPENAI_BASE_URL/API_KEY` + MCP | 片段 |
| Qwen Code | `~/.qwen/settings.json`、`~/.qwen/.env` | 同上（`OPENAI_*` 优先） | 片段 |
| OpenCode | `~/.config/opencode/opencode.json`、`~/.opencode.json` | `provider.*.options.baseURL/apiKey` + `models` + `mcp` | ✅ `provider.aiwork` |
| Continue | `~/.continue/config.json`、`config.yaml` | `models[]`（`apiBase/apiKey/model/provider/title`）+ MCP | ✅ `models[]` 追加 |
| Cline | `%APPDATA%/Code/.../saoudrizwan.claude-dev/settings/*.json` | `mcpServers` + `providers[]` | 片段 |
| Roo Code | `%APPDATA%/Code/.../rooveterinaryinc.roo-cline/settings/*.json` | 同上 | 片段 |
| Cursor | `~/.cursor/mcp.json`、`%APPDATA%/Cursor/User/{mcp,settings}.json` | `mcpServers` | 片段 |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` | `mcpServers` | 片段 |
| Cherry Studio | `%APPDATA%/CherryStudio/config.json` | `mcpServers` / `providers[]` | 片段 |
| Chatbox | `%APPDATA%/xyz.chatboxapp.app/config.json` | 同上 | 片段 |
| Aider | `~/.aider.conf.yml` | `openai-api-base` / `openai-api-key` / `model` | ✅ 扁平键重写 |

「片段」= 该客户端把凭据存在系统密钥环/应用内部库，直接改文件会被覆盖，因此返回**可复制片段**而不是假装写入。

### 3.3 额外目录（兄弟项目并入通道）

`agent_scan_all(extra_roots)` 对任意目录做启发式识别：

- `settings*.json` 含 `port`（+ 可选 `api_key/token`）→ 生成上游 `http://127.0.0.1:<port>/v1`；
- `*catalog*.json` / `catalog/*.json` → 递归收集模型名并入该上游的 `models`；
- `accounts/*.json` → 只计数并提示「账号池请走各应用页导入」，**不接管**账号（避免双真源）。

这样 `qoder2api-hub`、`workbuddy2api-hub-v1.7.12\wb-proxy` 都能作为「模型来源 + 上游」被一键并入。

### 3.4 命令契约（10 条）

| 命令 | 语义 |
|---|---|
| `agent_scan_all(extra_roots?)` | 内置注册表 + 额外目录全量扫描 |
| `agent_scan_one(key)` | 单智能体重扫 |
| `agent_import_preview(provider_ids, opts?, extra_roots?)` | 只读计划：同名冲突判定、Key 复用判定 |
| `agent_import_apply(provider_ids, opts?, extra_roots?)` | 落库：`custom_models` + `api_keys`（`modify` 原子改）+ `agent_mcp_registry` + 历史 |
| `agent_connect(target, opts?)` | 写回目标智能体（写前 `<file>.bak_aiwork_<ts>` 备份，幂等） |
| `agent_export_bundle()` | 脱敏接入包（只含 Key 掩码，完整 Key 不导出） |
| `agent_import_history()` | 最近 50 条导入记录 |
| `agent_mcp_registry()` / `agent_mcp_clear()` | 统一 MCP 注册表读/清 |
| `agent_gateway_info()` | 本网关 base/port/默认模型/接入 Key 名/provider id |

选项（`AgentImportOptions`，前端可勾）：建自定义模型（默认开）、建专用 `ck_` Key（默认关）、
新模型默认启用（默认开）、同名覆盖（默认关）、合并 MCP（默认开）。

### 3.5 红线与测试

- 只读识别 + 写前备份；Key 前端只回显 `sk-1****abcd`；
- 同名模型 canonical 冲突默认**跳过并说明**，绝不静默改道；
- 12 个单测覆盖：YAML 子集解析（含嵌套/序列）、`.env`、Claude/Codex/Continue 解析、
  额外目录网关识别、`config.toml` 合并幂等、导入幂等与历史累积、Key 复用不重复。

---

## 4. 增强能力（从两个 hub 回收）

### 4.1 上游组：多端点 + 熔断（`upstream_pool.rs`）

| 参数 | 值 | 来源 |
|---|---|---|
| 选点 | nginx SWRR（`cw += weight`，取最大，`-= total`） | `wb_upstream_pool.py` |
| soft 退避 | `min(600·2^(n-1), 7200)` | `qoder_accounts.py` |
| breaker | 阈值 3，`min(1800·2^(n-3), 21600)` | 同上 |
| degrade | 阈值 5，`min(600·2^(n-5), 7200)` | 同上 |
| 失败窗口 | 600s（窗口外重新计数） | `wb_probes/accounts` |
| 401/403 | 直接压到 `BREAKER_MAX`（6h） | 端点凭据错误不应高频重试 |
| 402 | 停放至次日 **UTC+8 10:00** | `qoder_accounts.ready()` |
| 运行态 | **只存内存**（fails/cw/until 不落盘） | 两个 hub 共同教训：落盘会复活早已自愈的冷却 |

网关接入点：`custom_route.rs::make_custom_request_pooled` —— 先 SWRR 选点，预流式失败按族记账并换端；
`4xx`（除 401/402/403/429）判定为请求错误，不再换端。**未配组 → 完全走原单源语义**。

### 4.2 成本核算（`pricing.rs`）

内容寻址 `policy_id = sha256(model|currency|flat_in|flat_out|bands|usd_cny)[:12]`，
分档按「覆盖用量的最小上限档」计价（`upto_input=0` 为兜底档，必须最后），明细只存策略引用 →
**调价不改写历史账**。`record()` 挂在 `custom_route` 的两条成功路径（流式/非流式），
`summary()` 按日×模型聚合，并把「有请求但无策略」的模型列入 `unpriced`（不静默记 0）。

### 4.3 活动时间线（`activity.rs`）

`ALLOWED_FIELDS` 白名单投影 → 未知字段丢弃；`sk-`/`ck_`/`Bearer`/`api_key=` 正则脱敏 → 截断 200；
kv 环形 500 条 + `data/activity.jsonl` 追加（超 2MB 单代轮转）。

### 4.4 模型目录（`modelsdev.rs`）

四级查找：远程 `models.dev/api.json`（24h 缓存、离线不阻断）→ 家族前缀兜底（最长前缀优先）→
本地目录值 → 默认 1M；**未知输出上限返回 `None`**，UI 显示「未收录（不编造）」。

### 4.5 出口探测与端点探测（`commands/hub.rs`）

出口 IP 走 `api.ipify.org` + `ipinfo.io`（60s 缓存、失败只降级不报错）；
上游端点探测发最小 chat 请求并**按族记账**（探测结果影响真实调度，而不是只看一眼）。

### 4.6 加密信封（`atrest.rs`）

`key = sha256(seed)`，`key_id = hex(sha256(key))[:16]`，AES-256-GCM + 12B nonce；
AAD = `"AWAAD\0" | 0x01 | LP("AWEF1") | LP("sym-v1") | u32(1) | LP(key_id) | code | 0x00 | 0x00`
（4 字节大端长度前缀；与 hub 的 `WB-AAD` 前缀区分，跨产品不互通）。改任何字段（含 `code`）都会解密失败。

---

## 5. 明确未移植（附理由）

> 依据是三份只读分析报告（已随工程归档）：
> [`research/_merge_report_qoder.md`](research/_merge_report_qoder.md)（qoder2api-hub 模块/算法/数据模型）、
> [`research/_merge_report_wb.md`](research/_merge_report_wb.md)（wb-proxy 22 模块与 10 项可移植能力）、
> [`research/_ext_points.md`](research/_ext_points.md)（宿主扩展点手册：导航/命令/存储/网关/REST 全链路）。

| hub 能力 | 不移植理由 |
|---|---|
| `wb_proxy.py` 单文件 20k 行路由 | 桌面端已有 axum 分层 + 统一鉴权中间件 |
| `wb_upstream_pool` 的连接池本体 | 那是为绕 urllib 的 `Connection: close`，hyper/ureq 自带连接复用 |
| `wb_tasks` 成长任务 19 项硬编码 id | 2026-09 实测值，易腐化；宿主 `tasks/wb_checkin.rs` 已在维护 |
| `qoder_sign.py` COSY 签名 | 宿主 `tasks/qoder_sign.rs` 已实现并随版本跟进 |
| `qoder_fingerprint.derive_id` | 同上（`tasks/qoder_device.rs` + 真机身份策略） |
| hub 面板密码 / LAN 模式 / 启动脚本 / 零依赖手写密码学 | 桌面端无此场景（且手写 AES/RSA 是为 alpine 无 pip 付的代价） |
| `dashboard.html` | 只回收指标口径，UI 由宿主各应用页承载 |
| 账号池文件格式 | 避免与宿主账号体系形成双真源（改为「额外目录 → 上游」通道） |

---

## 6. 构建与验收

```powershell
# 前端
cd E:\study\gitproject\aiwork\AIWorkHub
npm install
npx tsc --noEmit          # 类型检查
npx vite build            # 产物 dist/

# Rust（本机 .cargo\bin 的 rustc/cargo 是 0 字节坏桩，需直接用 toolchain 路径）
$env:PATH="C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;$env:PATH"
cd src-tauri
cargo check               # 依赖 + 类型检查
cargo test                # 单测（新增 30+ 用例：agent_import / upstream_pool / pricing / activity / modelsdev / atrest）

# 完整安装包（需 MSVC Build Tools + WebView2）
npm run tauri build
node scripts/rename_release.mjs
```

验收记录见 `docs/build-report.md`（逐条命令 + 结果 + 产物路径）。

---

## 7. 使用路径（三步闭环）

1. **导入**：侧栏「智能体接入」→ 自动扫描 → 勾选（默认全选）→「一键导入」；
   有兄弟项目时在上方「额外目录」填 `…\qoder2api-hub` / `…\wb-proxy` 再扫描；
2. **加固**：侧栏「枢纽增强 → 上游池」把导入的模型扩成多端点组（权重/策略），
   「成本核算」补单价策略（否则该模型会在「未定价」里被点名）；
3. **下发**：回到「智能体接入」，选目标智能体 →「一键接入本网关」，
   自动写回 Claude Code / Codex / OpenCode / Continue / Aider（写前备份），
   其余客户端复制片段粘贴即可。
