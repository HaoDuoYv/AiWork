# Trae CLI 桥接落地方案（企业后端模拟）

> 版本：v1.0（2026-10-01）
> 状态：分析完成，待 Phase 0 验证
> 前置分析：基于 trae-cli 0.120.52（windows/amd64）二进制逆向 + 本机实测 + 官方文档

---

## 📋 一、背景与结论摘要

### 1.1 需求起源

Trae 官方发布了 CLI（trae-cli，文档 `https://docs.trae.cn/cli`），希望参考本项目已有的 CodeBuddy CLI 切号桥（`workbuddy_cli.rs` 五重防护自动轮换），为 Trae 实现同等能力。

### 1.2 分析结论（一句话版）

- **个人账号（trae.cn 个人版）不能直接使用 Trae CLI**：CLI 只连接企业 API 体系（`api.enterprise.trae.cn`），个人版 API（`api.trae.cn`）在 CLI 二进制中 0 次出现。
- **桥接方案技术可行**：通过「PAT 环境变量注入登录 + HTTPS_PROXY 流量劫持 + 伪造企业 API 响应 + llm/proxy 对话桥接到项目网关」，可让 CLI 成为项目网关的一个 agent 客户端。
- **工程量对比**：Buddy CLI 切号桥 = 写一个 settings.json 字段；Trae CLI 桥接 = 整层后端模拟，工作量差两个数量级。
- **建议**：先执行 Phase 0（低成本观察验证）拿到实测数据后再决定是否全量投入。

---

## 🎯 二、Trae CLI 情报汇总（全部实证）

### 2.1 基本信息

| 项 | 值 | 来源 |
|---|---|---|
| 版本 | 0.120.52 | 本机 `trae-cli doctor` 实测 |
| 实现语言 | Go（cobra 风格 CLI） | 二进制符号表 |
| 内部代号 | coco（`code.byted.org/nextcode/coco/tenant/trae/cli`） | 二进制包路径 |
| 安装位置 | `%LOCALAPPDATA%\trae-cli\bin\` | 本机实测 |
| 可执行别名 | `trae-cli.exe` / `traecli.exe` / `trae-agent.exe` / `ta.exe` | 本机实测 |
| ripgrep 绑定 | `%LOCALAPPDATA%\trae-cli\ripgrep\rg.exe`（15.0.0） | doctor 输出 |
| 自更新通道 | `lf-cdn.trae.com.cn/obj/trae-com-cn/trae-cli` | 二进制提取 |
| 用户文档 | `docs.trae.cn/cli`（文档站）+ volcengine 86677 系列（更全） | Web 检索 |

### 2.2 认证体系（二进制逆向证据）

**登录链路**（auth 包函数提取）：

```
oauthLogin → openBrowser → startAuthServer(getAvailablePort 本地回调)
  → ExchangeToken（POST 企业 API /api/v3/trae/oauth/ExchangeToken）
  → keyring 持久化（go-keyring → Windows 凭据管理器 windowsKeychain）
  → GetUserInfo → isAllowedProductType 校验 → 放行/拒绝
```

**关键校验函数与数据结构**：

| 符号 | 含义 |
|---|---|
| `auth.isAllowedProductType` | 登录后校验账号 ProductType（套餐门控核心） |
| `util.QuotaCheckResult` / `Entitlement` / `ProductType` | 权益/配额检查链 |
| `json:"ProductType"` / `json:"Avatar,omitempty"` / `json:"JoiningTime"` | UserInfo 结构体 JSON tag（可提取 schema） |
| `has_plan` / `planType` / `flagship` | 套餐相关字段 |

**服务端拒绝文案原文**（二进制提取）：

> 当前版本暂不支持CLI功能，请联系管理员升级套餐或切换到IDE使用。

**登录态存储**：Windows 凭据管理器（go-keyring，`keyring_store.go` / `keyring_windows.go` / `windowsKeychain.credName`）。未登录时任何操作报：

```
failed to get models from provider: failed to get value from keyring: secret not found in keyring
```

**PAT（CLI 登录令牌）路径**：

- 环境变量：`TRAECLI_PERSONAL_ACCESS_TOKEN`（文档要求 `trae-lt-` 前缀；**二进制中无本地格式校验**，纯透传服务端）
- 可选：`TRAECLI_HOST`（企业专属域名覆盖，`ensureHTTPSScheme` 会强制 https scheme）
- 官方文档明确「套餐要求：旗舰版」，仅能在 `console.enterprise.trae.cn` 企业控制台生成

### 2.3 网络行为

**后端域名隔离**（决定性证据）：

| 域名 | 二进制出现次数 | 说明 |
|---|---|---|
| `api.trae.cn`（个人版 API） | **0** | 项目签到/积分/套餐接口全走此域名 |
| `api.enterprise.trae.cn`（企业 API） | 2 | CLI 默认后端 |
| `console.enterprise.trae.cn` | 1 | 企业控制台（PAT 生成入口） |
| `lf-cdn.trae.com.cn` | 1 | 自更新 CDN |

**接口清单**（二进制路径提取）：

| 接口 | 用途 | 桥接优先级 |
|---|---|---|
| `/api/v3/trae/oauth/ExchangeToken` | 登录换取 token | P0（登录链） |
| `/api/v3/trae/GetUserInfo` | 用户信息 + ProductType | P0（登录链） |
| `/api/v3/trae/CheckLogin` | 登录态周期校验 | P0（登录链） |
| `/api/v1/llm/proxy` | **对话流量统一代理端点** | P0（对话链） |
| `/api/v1/config/get_network_proxy` | 服务端下发代理配置 | P1（外围兜底） |
| `/api/v1/data/data_report` | 遥测数据上报 | P1（外围兜底） |
| `/api/v1/mcp_whitelist/list` | MCP 白名单 | P1（外围兜底） |
| `/api/ide/v1/tenant/get_tenant_user_config` | 租户用户配置 | P1（外围兜底） |
| `/api/ide/v1/check_custom_model_quota` | 自定义模型配额检查 | P1（外围兜底） |
| `/api/ide/v1/tenant/report_audit_log` | 审计日志上报 | P2（可静默丢弃） |

**代理支持**（桥接的官方口子）：

- 二进制含 `ProxyFromEnvironment` + `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` 字符串
- Go 标准库默认信任 Windows 系统证书池 → 自签 CA 装入根证书库即可 MITM
- 未发现 uTLS / 证书锁定（pinning）特征（Phase 0 仍需实证）

### 2.4 配置体系

| 配置文件 | 路径 | 说明 |
|---|---|---|
| 全局配置（主） | `%APPDATA%\trae_cli\trae_cli.yaml`（官方文档） | Windows 路径 |
| 全局配置（镜像） | `~/.trae/trae_cli.yaml` | doctor 显示的 config 目录；CLI 尝试 auto-symlink 关联两路径（Windows 无开发者模式时 symlink 失败但有 WARN 降级，双路径均可读） |
| 项目级配置 | `.trae/traecli.yaml` | 工作目录级 |
| 自定义模型 | `models[].open_ai{base_url,api_key,model}` / `models[].claude{base_url,model,api_key}` | **官方支持的模型源配置** |

**⚠️ 实测结论**：自定义模型配置**不绕过登录墙** —— 未登录时 `-p` 与 `models` 均先走 keyring 取登录态并失败退出。自定义模型只是登录后的「模型源」。

**会话/杂项**：

- `TRAECLI_SESSION_ID` / `COCO_SESSION_ID` 环境变量存在
- 会话数据目录：`~/.trae/`（agents/commands/plans/skills/worktrees 等，doctor 首次使用时创建）
- 命令行 flags：`-p`（print 管道模式）、`--output-format json/stream-json`、`--resume`、`--session-id`、`--permission-mode`、`-y`（yolo）等 —— 对管道集成友好

### 2.5 个人版 Trae IDE 的隔离证明

本机 `~/.trae-cn/trae-jwt-token` 解码：

```json
{"data":{"id":"1335275560894795","tenant_id":"7o2d894p7dr0o4","type":"user","user_id":"1335275560894795"},"exp":...,"iss":"trae"}
```

- payload 仅含 `id` + `tenant_id`，无套餐字段；ProductType 是登录后 GetUserInfo 从服务端获取的
- 该 JWT 属于 api.trae.cn 签发体系，CLI 认证链（企业 API）无兑换通路
- 本机旁证：CLI 已安装但 `~/.trae` 从未创建、凭据管理器无 trae 条目（登录从未走通）

---

## 🔧 三、桥接方案总体设计

### 3.1 架构总览

```
┌─────────────┐  HTTPS_PROXY env   ┌──────────────────────────────────┐
│   trae-cli  │───────────────────▶│  本软件 MITM 代理层                │
│  (Go 客户端) │                    │  （device_proxy + ca.rs 自签 CA）  │
└─────────────┘                    └──────────────┬───────────────────┘
                                                  │ 按域名分流
                    ┌─────────────────────────────┼──────────────────────┐
                    ▼                             ▼                      ▼
        console.enterprise.trae.cn    api.enterprise.trae.cn    lf-cdn.trae.com.cn
        （PAT 注入登录，无需劫持页面）     ├─ ExchangeToken → 伪造 token   （update 检查）
                                      ├─ GetUserInfo → 伪造 ProductType    │
                                      ├─ CheckLogin → 固定通过             ▼
                                      └─ /api/v1/llm/proxy          放行或锁定
                                                    │
                                                    ▼
                                      ┌──────────────────────────────┐
                                      │  项目 API 网关调度层            │
                                      │  Trae 池 / Buddy 池 / 自定义    │
                                      │  （复用现有 /v1/* 调度管线）     │
                                      └──────────────────────────────┘
```

### 3.2 关键设计决策

| # | 决策 | 理由 |
|---|---|---|
| D1 | 登录注入用 **PAT 环境变量**，不伪造浏览器 OAuth 登录页 | PAT 是官方支持的免浏览器路径（Docker/CICD 场景）；二进制无本地格式校验，任意串透传；省去整页 OAuth 模拟 |
| D2 | 流量劫持用 **HTTPS_PROXY 环境变量**，不用 hosts+443 | hosts 方案需监听 443（提权）；Go 程序支持 ProxyFromEnvironment，环境变量随 CLI 进程注入即可，粒度可控、无需提权 |
| D3 | TLS 终结用 **自签 CA 装系统根证书库**（复用 ca.rs） | Go 默认信任系统池；ca.rs 已有 `gen_server_config` 能力，只需扩展目标域名 |
| D4 | 对话出站**直接对接项目网关调度**（Trae 池 raw-chat / OpenAI 兼容转换） | 复用现有调度、SSE 转换、积分管理、401 自愈整条管线，风控面与现有网关用法同源 |
| D5 | **版本锁定**：劫持 update 检查响应 | CLI 迭代快（0.120.x），协议漂移是最大维护成本；锁定版本 + 手动跟进升级 |
| D6 | MITM 白名单**只加企业域名**，个人版 api.trae.cn 流量不碰 | 最小侵入；IDE 正常使用完全不受影响 |

### 3.3 登录链伪造细节（Phase 1 核心）

**PAT 登录时序（预测，Phase 0 实证修正）**：

1. 用户设置 `TRAECLI_PERSONAL_ACCESS_TOKEN=<任意串>`（+ `TRAECLI_HOST` 可选）
2. CLI 调 `ExchangeToken`（或直接以 PAT 为凭证调 CheckLogin/GetUserInfo）
3. MITM 拦截 → 返回伪造成功响应（token / 过期时间）
4. CLI 调 `GetUserInfo` → MITM 返回伪造 UserInfo：
   - `ProductType` = 合法放行值（具体枚举值从二进制/动态观察确定）
   - `has_plan` = true、Avatar/JoiningTime 等字段按提取到的 JSON tag 填充
5. `isAllowedProductType` 通过 → CLI 写 keyring → 登录态建立
6. 后续周期 `CheckLogin` → MITM 固定返回通过

**已知未知（需 Phase 0 确定）**：

- ExchangeToken 请求体结构（headers：X-Log-Id、DeviceID、签名？body：code/token 格式？）
- GetUserInfo 响应完整 schema（JSON tag 部分可提取，字段间约束未知）
- CheckLogin 调用时机与频率（启动时？每请求前？定时？）
- llm/proxy 请求体（模型选择如何表达、鉴权头格式）与响应流式协议（SSE 事件结构）

---

## 🚀 四、分阶段落地计划

### Phase 0：观察与验证（前置闸门，成本低、信息价值最高）

**目标**：确认 MITM 可行性 + 拿到全部请求样本 + 定逆向难度。

任务分解：

1. **证书锁定验证**：ca.rs 生成自签 CA → 装系统根证书库 → device_proxy 挂 `HTTPS_PROXY` 拦截 CLI 对 `api.enterprise.trae.cn` 的请求 → 若 TLS 握手成功则无锁定（预期成功）；失败则方案降级评估
2. **请求样本采集**：分别触发 `trae-cli doctor` / `models` / `-p "hi"`（PAT 注入 / 不注入两种）→ 记录每个请求的 method/path/headers/body
3. **响应 schema 反推**：对各接口先返回 500/空体，观察 CLI 报错信息与重试行为，反推必需字段；结合二进制 JSON tag 提取拼接 schema
4. **llm/proxy 协议观察**：PAT 登录被拦的情况下对话请求是否仍发出（可能先 CheckLogin 失败即止）→ 确定 llm/proxy 观察是否依赖 Phase 1 先行

**验收标准**：

- [ ] MITM 拦截 CLI→企业 API 的 TLS 链路稳定（无证书锁定）
- [ ] 4 个 P0 接口的请求样本（headers + body 完整记录）
- [ ] GetUserInfo 至少一组「报错反推」的最小必需字段清单
- [ ] 撰写《Phase 0 观察报告》→ 决策是否进入 Phase 1

### Phase 1：登录链伪造

**目标**：CLI `/status` 显示已登录。

任务分解：

1. 本软件新增 `trae_cli_bridge` 模块（建议放 `src-tauri/src/device_proxy/` 下或独立 `trae_cli/` 模块）：
   - MITM 规则扩展：白名单加 `api.enterprise.trae.cn`（域名白名单机制已有，参考 `device_proxy/mod.rs` 的 `host_in_targets`）
   - 假后端路由：ExchangeToken / GetUserInfo / CheckLogin 三个 handler
   - 伪造 UserInfo 常量表（ProductType 等字段）
2. 登录注入入口（Tauri command 或前端按钮）：设置进程级环境变量启动 CLI（或生成启动脚本/快捷方式，内嵌 `TRAECLI_PERSONAL_ACCESS_TOKEN` + `HTTPS_PROXY`）
3. keyring 落盘验证：登录成功后读取 Windows 凭据管理器条目（`cmdkey /list` 过滤 trae）确认 token 已持久化

**验收标准**：

- [ ] `trae-cli doctor` 不再报登录错误，`/status` 显示已登录（伪造身份）
- [ ] 登录态重启 CLI 后仍有效（keyring 生效）
- [ ] 本软件重启后 MITM 可恢复劫持（CA 持久化）

### Phase 2：对话桥接（llm/proxy → 项目网关）

**目标**：`trae-cli -p "hi"` 经账号池出话。

任务分解：

1. llm/proxy 请求解析：模型名映射到网关统一模型目录（复用 `unified_catalog.rs` 档位体系）
2. 出站对接：llm/proxy 请求体 → 网关调度管线（`api_server` 现有 Trae 池 raw-chat / 转换逻辑），错误处理复用 401 自愈
3. 响应回填：网关 SSE 输出 → llm/proxy 期望的流式格式（Phase 0 拿到的协议结构）；先非流式后流式
4. 用量/积分口径：对话计费走网关现有 `api_usage.json` 口径（days 桶）

**验收标准**：

- [ ] `trae-cli -p "hi"` 返回模型回复（走 Trae 池账号）
- [ ] 流式对话正常（SSE 不中断、无乱序）
- [ ] 网关用量统计正确累加
- [ ] 网关侧调度日志完整（模型、档位、账号）

### Phase 3：外围兜底与稳定化

**目标**：长期稳定运行 + 与账号管理体系整合。

任务分解：

1. 外围接口兜底：`get_network_proxy`（返回空配置）、`data_report`（200 空响应）、`mcp_whitelist`（空列表）、`tenant/get_tenant_user_config`、`check_custom_model_quota` 等 P1 接口最小响应，防 CLI 启动卡死
2. update 检查劫持：`lf-cdn.trae.com.cn` 版本接口返回当前版本（锁定协议版本）；或 NO_PROXY 放行 + 用户约定不升级
3. **CLI 账号轮换整合**（对标 Buddy 五重防护）：
   - 轮换目标：PAT/登录态是伪造的（对 CLI 而言无差别），**实际轮换的是网关侧调度账号**——复用现有网关池轮换即可，CLI 侧无需感知
   - 可选增强：MITM 层对 llm/proxy 请求注入会话标记 → 网关按 CLI 会话维度统计用量
4. 前端整合：环境配置页新增「Trae CLI 桥接」卡片（状态/启停/日志，对齐 BuddySettings CLI 卡片模式）
5. 健壮性：MITM 断连时 CLI 的降级行为测试；NO_PROXY 白名单确保本软件自身出站流量不回环

**验收标准**：

- [ ] CLI 连续会话 24h 无启动卡死/崩溃
- [ ] 网关侧账号轮换对 CLI 透明
- [ ] 前端卡片可观测桥接状态与流量统计

---

## 🧩 五、与现有基建的复用映射

| 现有模块 | 复用点 | 改造量 |
|---|---|---|
| `device_proxy/mod.rs`（MITM 代理） | 流量劫持主框架、`host_in_targets` 域名白名单 | 白名单加企业域名；新增 llm/proxy 分流 handler |
| `device_proxy/ca.rs`（自签 CA） | TLS 终结证书生成（`gen_server_config`） | 目标域名换 `api.enterprise.trae.cn` |
| `api_server/`（网关） | 对话出站调度、模型目录、档位映射、401 自愈 | 新增 llm/proxy 入站适配器（协议转换） |
| `api_server/wb_sse.rs` 等 SSE 转换 | 流式响应回填 llm/proxy 格式 | 新增一种 SSE 格式适配 |
| `workbuddy_cli.rs`（五重防护） | 轮换决策纯函数参考；本轮实际轮换发生在网关池，CLI 无感 | 逻辑复用而非代码复用 |
| `store`（SQLite kv） | 桥接状态、伪造 schema 缓存、日志 | 新增 kv 键 |
| `notify`（通知渠道） | 桥接异常通知（TaskFail 事件） | 复用 |

---

## ⚠️ 六、风险与维护策略

| 风险 | 等级 | 缓解 |
|---|---|---|
| 响应 schema 逆向不确定（无企业账号抓包） | 高 | Phase 0 报错反推 + 二进制 JSON tag 拼接 + dlv 动态调试兜底；闸门决策 |
| CLI 协议漂移（迭代快） | 高 | D5 版本锁定；协议 schema 落 kv 表（可热更）；升级前 diff 二进制字符串 |
| 证书锁定（未完全排除） | 中 | Phase 0 第一项验证；若存在 → 评估 uTLS 伪装或放弃 |
| 强制更新破坏协议 | 中 | update 劫持 + 禁用 CLI 自更新 |
| 风控 | 低 | 对话流量与现有网关用法同源；个人版 API 不触碰；伪造身份仅存在于 CLI↔本软件之间，不出网 |
| 外围接口缺失导致启动卡死 | 中 | Phase 3 兜底清单 + 超时快速失败 |

**维护成本预估**：Phase 1-2 完成后，每次 CLI 升级需 re-diff 协议（llm/proxy 结构最易变），预计半天级适配；长期成本与 CLI 官方迭代节奏强相关。

---

## 📌 附录 A：本会话实证记录（原始证据）

### A.1 二进制提取命令

```powershell
$rg = "$env:LOCALAPPDATA\trae-cli\ripgrep\rg.exe"
$bin = "$env:LOCALAPPDATA\trae-cli\bin\trae-cli.exe"

# 环境变量名
& $rg -a -o "TRAE[A-Z_]*TOKEN|TRAECLI_[A-Z_]+|[A-Z]+_AUTH_TOKEN" $bin
# → TRAECLI_PERSONAL_ACCESS_TOKEN / TRAECLI_HOST / TRAECLI_SESSION_ID / ANTHROPIC_AUTH_TOKEN

# API 端点
& $rg -a -o "/api/v[0-9]/[a-zA-Z_/]{3,50}" $bin
# → /api/v1/llm/proxy、/api/v3/trae/{CheckLogin,GetUserInfo,oauth/ExchangeToken} 等

# 域名计数（个人版 0 次出现的决定性证据）
& $rg -a -c "api\.trae\.cn" $bin        # → 无输出（0）
& $rg -a -c "enterprise\.trae\.cn" $bin # → 2

# 拒绝文案
& $rg -a -o ".{100}请联系管理员升级套餐.{100}" $bin
# → 「当前版本暂不支持CLI功能，请联系管理员升级套餐或切换到IDE使用。」

# 代理支持
& $rg -a -c "ProxyFromEnvironment" $bin # → 1
```

### A.2 本机实测日志

```
# 未登录时 models / -p 均失败（登录墙在一切之前）
{"level":"ERROR","msg":"failed to get models from provider",
 "error":"failed to get value from keyring: secret not found in keyring"}

# auto-symlink：%APPDATA%\trae_cli\trae_cli.yaml ↔ ~/.trae/trae_cli.yaml 双路径关联
{"level":"WARN","msg":"auto-symlink: create symlink failed", "target":"C:\\Users\\tianw\\.trae\\trae_cli.yaml",
 "error":"A required privilege is not held by the client."}
```

### A.3 官方文档要点

- 「CLI 登录令牌」：套餐要求**旗舰版**；仅限创建者本人；企业控制台（console.enterprise.trae.cn）生成；`trae-lt-` 前缀；环境变量 `TRAECLI_PERSONAL_ACCESS_TOKEN`（+ 企业可选 `TRAECLI_HOST`）
- 全局设置：Windows 路径 `%APPDATA%\trae_cli\trae_cli.yaml`；`-c k=v` 运行时覆盖
- 自定义模型：`models[]` 支持 OpenAI / Claude 两种格式（base_url + api_key + model）

### A.4 与 Buddy CLI 切号桥的对比

| 维度 | CodeBuddy CLI（已实现） | Trae CLI（本方案） |
|---|---|---|
| 凭证落盘 | `~/.codebuddy/settings.json` 的 `env.CODEBUDDY_AUTH_TOKEN`（明文 JSON，直写） | Windows 凭据管理器 keyring（需 CredWrite API 或走登录注入） |
| 切号动作 | 改一个 JSON 字段 | 网关侧池轮换（CLI 无感）或重走 PAT 注入 |
| 会话活动扫描 | `~/.codebuddy/projects/**/*.jsonl` mtime | `~/.trae/` 会话目录（结构待 Phase 0 确认） |
| 后端 | 官方服务 | **本软件伪造的企业后端** |
| 工作量 | ~2 文件 | 整层协议模拟（4 阶段） |
