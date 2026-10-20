# F-80 Qoder 支持设计方案（Qoder CN IDE / Qoder CN CLI；QoderWork CN 即 Qoder CN IDE，非独立端）

> **文档版本**: v1.4.2（2026-09-27）
> **需求来源**: [Issue #36](https://github.com/smart-open/TraeWorkAssistant/issues/36)（2026-09-25，用户请求补 QoderWork CN 签到；已回复纳入后续规划）
> **状态**: 实施中：M0/M1/M2/M3 ✅ 交付；**M4 ✅ 交付**（v1.4.2 导出导入 / env_reset 补齐，cargo test 598 通过 + 前端 build 通过）；M5 未启动；「Qoder 入网关池」立项为独立任务（动工硬前置 R-13，方案见 §5.11）
> **v1.4.1 产品同一性澄清（2026-09-27 需求方实测确认）**: **QoderWork CN 就是 Qoder CN 本体**（官方全家桶文案别名），非第三个独立产品。本文按"三端"撰写的 QoderWork 独立端规划（TargetApp::QoderWork 变体、独立数据目录 R-1 Authfile 分支、L2 MITM QoderWork 组、独立档案/存储）历史表述**一并作废**；issue #36 的「QoderWork CN 签到」即 Qoder CN 签到，已由 M1 `qoder-checkin` 承接闭环。实测目录：`%LOCALAPPDATA%\Qoder CN\` 仅 Installer/Launcher 引导器（无用户数据），用户数据在 `%APPDATA%\QoderCN\`（已由 QoderIde 通道完整覆盖，`QODER_IDE_ITEMS` 含 state.vscdb / machineid / Local State / SharedClientCache 等 14 项）
> **关联**: `docs/backlog.md`（登记时以实际可用编号为准，当前最高 F-79，本文按 F-80 撰写）；参照实现 `src-tauri/src/tasks/wb_checkin.rs`（Buddy 全链路）

---

## 0. 范围界定

### 0.1 本期支持对象（三端，2026-09-26 与需求方确认）

| 端 | 形态 | 技术栈 | 本期能力 |
|---|---|---|---|
| **Qoder CN IDE** | 编码 IDE 桌面端（原通义灵码升级，全家桶 Desktop） | Electron / VS Code fork | 账号发现 + 登录态快照切换 + 签到凭证源 |
| **Qoder CN CLI** | 终端 CLI（npm 包 `@qodercn-ai/qoderclicn`，命令 `qodercn`） | Node.js | PAT 凭证接入 + 积分查询 + （可选）切号桥 |
| ~~QoderWork CN~~ | = Qoder CN IDE（v1.4.1 澄清：官方全家桶文案别名，同一产品） | — | 无独立端；签到主入口即 M1 `qoder-checkin` |

### 0.2 明确不在本期范围（边界）

- ❌ **国际版 Qoder**（qoder.com 体系）：Credits 与 CN 不互通，账号体系独立，不做。
- ❌ **原灵码线**（commodityCode 含 `lingma_` 前缀订单）的独立管理：仅做**产品线识别与提示**（其 Credits 不能在 QoderWork/CLI 使用），不单独接管。
- ❌ **JetBrains 插件**：无独立客户端本地态，共享全家桶账号，随 IDE 账号自然覆盖。
- ❌ **Qoder 接入 API 网关池**：遵守 backlog §四"多通道网关统一接入（QClaw/QwenWork 等）远期再议"的既有结论，本期签到/积分不依赖网关。
- ❌ QoderWake / Cloud Agents / Mobile / QoderWake 数字员工。
- ⏸️ **macOS 端**：本期不实施（`mac_supported: false` 灰度起步，沿用 F-75 门控防串台），**但代码全程保持跨平台兼容**——所有平台差异点按 §5.9 收敛到可替换边界，后续版本规划实施 macOS 端（2026-09-26 需求方确认）。

---

## 一、背景与目标

### 1.1 需求背景与价值

1. **用户价值**：Qoder 每日签到活动（2026-06-13 起）每日 100 Credits，资源包**独立 30 天有效期、FEFO 扣减、当天未签不补签**——漏签即永久损失，自动化直接提高积分利用率；2026-09-18 起新增"每日登录奖励"（100 通用 Add-on Credits，每日 10:00 开窗至次日 10:00），官方声明暂无结束时间。
2. **产品价值**：补齐阿里生态（现有字节系 Trae/豆包 + 腾讯系 Buddy），从"两厂五应用"升级为"三厂八端"；与 README"后续规划扩展更多 AI 应用"路线一致。
3. **紧迫性注记**：Trae 侧积分签到规则已调整（W-01 排除记录），Trae 签到自动化价值缩水，用户需求向 Qoder 转移（Issue #36 即此背景）。

### 1.2 目标

| 编号 | 目标 | 对应里程碑 |
|---|---|---|
| G1 | 三端账号接入：发现 / 导入 / 凭证管理 / 状态展示 | M1 / M3 |
| G2 | 自动签到：sash campaigns 幂等 claim，覆盖双活动 | **M1（issue 承诺的最小闭环）** |
| G3 | 积分看板：余额 / 到期 / 日志，区分产品线，官方 PAT 通道优先 | M2 |
| G4 | IDE 账号切换：复用 Icube 快照管线 | M3 |
| G5 | 调度：应用内 scheduler + Windows schtasks 双轨 + 启动补签（OS 注册收敛门面，macOS launchd 后续补实现） | M1 |

### 1.3 非目标

- 不做批量注册、验证码绕过等灰产工具能力（见 §七 合规红线）。
- ✅ **多账号并发使用**（v1.2 纳入范围，需求方 2026-09-27 确认）：每账号绑定稳定设备指纹（QoderPatcher 模式），设计见 §5.10，风险见 §七 风险 1。
- 不做 Qoder Credits 的网关化 / 2api。
- 不做原灵码线（`lingma_`）订单的购买管理。

---

## 二、情报基线

> 标注：【已确认】= 官方文档或社区实测来源；【推测】= 旁证推断，M0 必须验证。完整来源见附录 B。

### 2.1 产品与账号体系【已确认】

- Qoder CN（原通义灵码）2026-05-20 更名，全家桶五端：**Qoder CN Desktop（IDE）、JetBrains 插件、QoderWork CN、Qoder CN CLI、Qoder CN Mobile**。
- 自 2026-06-20（IDE v1.3.0）起个人版统一为**全家桶账号**（手机号注册，qoder.com.cn）；原阿里云账号登录入口关闭，老账号 1:1 转 Credits；原 VS Code 插件停用。
- **Credits 共享**（个人版 Free/Pro/Pro+）：五端共享消耗；企业 Teams/Enterprise VPC 目前仅 QoderWork CN + CLI 共享。
- **两条产品线不互通**：原灵码订单（`lingma_` 前缀）Credits 仅在原灵码线内生效；原灵码控制台 `qoder.console.aliyun.com`，全家桶 `qoder.com.cn`，两边用量页面独立。
- **CN 与国际版不互通**（定价不同，Credits 不等价）。
- 扣减规则：FEFO（最先到期优先）；同到期先 Plan 后 Add-on；失败请求不扣费；Plan Credits 以**购买日**为周期重置归零。
- ⚠️ **官方公告将于 2027-09-05 停止运营**（qoder.com.cn/download；公告对象原文即「QoderWork」，v1.4.1 澄清后即 Qoder CN 全家桶桌面端）——架构不绑死单一端，凭证通道（IDE 存储 / CLI PAT）独立可用。

### 2.2 签到活动【已确认】

| 活动 | 起始 | 入口 | 收益 | 窗口 | 备注 |
|---|---|---|---|---|---|
| QoderWork CN 每日签到 | 2026-06-13 | QoderWork 客户端**右上角浮窗** | 100 Credits/天（独立 30 天有效期包） | 每日 0 点（UTC+8）刷新 | 不补签 |
| 每日登录奖励 | 2026-09-18 | Qoder CN 桌面端**左下角用量面板 → 礼物图标** | 100 通用 Add-on Credits/天 | **每日 10:00 开窗，至次日 10:00** | 仅个人版；活动 Surface 在 claimable=true 时自动弹出 |

**调度设计结论**：两个活动的接口层同源（sash campaigns 体系，【已确认·社区实测】主进程轮询 campaigns 判断 claimable）。统一**每日 10:15 单次调度**即可同时覆盖（10:15 时两活动均处于可领窗口），失败进入 30 分钟重试冷却（对齐 `RETRY_COOLDOWN_MS` 模式）。无需为 0 点刷新的旧活动单独加跑次。

### 2.3 接口与凭证【已确认·社区逆向】

**双区域端点**（CN 域）：
- OpenAPI：`openapi.qoder.com.cn`
- Chat/Center：`gateway.qoder.com.cn`

**签到/活动（sash 体系）——本期核心端点**：

```
GET  {openApi}/sash/api/v1/me/campaigns
     响应: { showCampaign, claimable, campaignUrl,
             campaigns: [{ campaignId, campaignKey, claimStatus: CLAIMABLE|CLAIMED,
                           actionType: "CLAIM_BENEFIT", benefit: { amount }, startAt, endAt }] }

POST {openApi}/sash/api/v1/me/campaigns/{campaignId}/claim
     幂等：重复调用返回 data.status=="CLAIMED"、data.replayed==true（无副作用）
```

**鉴权与关键请求头**：
- `Authorization: Bearer {token}`
- **`Cosy-ClientType` 必带**（逆向脚本取 `"10"`；**缺失时服务端返回 `campaigns: []` 静默空列表**——这是最隐蔽的坑）；`"5"` 为 2api 项目所用，CN 客户端真实取值 M0 抓包确认（缺口 R-4）。
- Electron 主进程请求自带 `Cosy-MachineId` / `Cosy-MachineToken` 设备指纹头；纯 HTTP 重放是否被风控拦截待实测（R-5）。
- token 三前缀：`pt-`（PAT）/ `jt-`（job token，短时）/ `dt-`（device token）。PAT 经 `POST {openApi}/api/v1/jobToken/exchange` 换 `jt-`（非 COSY 签名）。
- 设备流：`POST /api/v1/deviceToken/poll` / `/api/v1/deviceToken/refresh`（body `{"refresh_token":...}`）；浏览器授权页 `https://qoder.com.cn/device/selectAccounts`。

**用量查询**：`GET {openApi}/api/v2/quota/usage`（Bearer）——逆向所得，无稳定性承诺。

**IDE 客户端存储**（国际版实测，CN 同构性 R-8 验证）：
- 数据目录：`%APPDATA%\com.qodercn.app.stable`（国际版为 `com.qoder.app.<channel>`）；exe：`%LOCALAPPDATA%\Programs\Qoder CN\.qoder-versions\<ver>\Qoder CN.exe`
- 凭据：`auth.v1.dat`（`v10` 前缀 + 12 字节 nonce + AES-GCM 密文）+ `Local State`（`os_crypt.encrypted_key`，DPAPI 解包得 AES-GCM 密钥）；会话 JSON 结构 `{token, refreshToken, expiresAt}` —— 与本项目 `Creds` 模型天然对齐
- 环境变量 `QODER_AUTH_DIR` 可覆盖 auth 目录（国际版脚本实测）
- 日志：`<data_dir>\logs\<session>\main.log`（记录 campaign claimable 状态，可做兜底信号源）
- ⚠️ 风控注记（社区实测）：活动页 Cookie 仅含风控基建（arms_uid/isg/tfstk）无登录态，直调页面接口 401。~~绝不伪造 auth.machine-id~~ → **v1.2 决策变更**：多账号并发要求每账号绑定稳定指纹（QoderPatcher 模式，§5.10），原红线改为风险项管理（§七 风险 1）

**CLI**【已确认·官方】：
- 配置目录 `~/.qoder-cn`（`QODERCN_CONFIG_DIR` 覆盖）；登录 `/login`（浏览器授权或粘贴 PAT）；`/usage` 查 Credits、`/status` 看配额
- PAT 自助创建：`https://qoder.com.cn/account/integrations`（`pt-` 前缀，仅关闭页面前可见一次）
- 环境变量 `QODERCN_PERSONAL_ACCESS_TOKEN` 优先级高于 `/login` 凭据——**官方认可的无头自动化通道**

### 2.4 情报缺口（M0 侦察清单驱动）

见 §六 R-1 ~ R-10。核心三条：
1. QoderWork CN 的数据目录与凭据文件形态（决定 Chromium 还是 Authfile 布局）；
2. Qoder CN IDE 是否存在 `User/globalStorage/state.vscdb`、storage.json 登录键是否同为 `iCubeAuthInfo://` 前缀（决定 Icube 管线能否直接复用）；
3. CN 端 `Cosy-ClientType` 真实取值与 sash 接口对设备头的校验强度（决定纯 HTTP 签到可行性）。

---

## 三、产品与账号体系分析

### 3.1 三端关系

```
        全家桶账号（手机号，qoder.com.cn）
        Credits 五端共享（FEFO）
      ┌──────────┬──────────┬──────────┐
Qoder CN IDE    QoderWork CN   Qoder CN CLI
(vscode fork)   (办公智能体)    (终端)
本地凭据:        本地凭据:       本地凭据:
auth.v1.dat     待侦察(R-1)     ~/.qoder-cn (R-3)
+Local State                   PAT env 可注入
接入管线:        接入管线:       接入管线:
Icube(待证)     Chromium 或     PAT / 切号桥
                Authfile(待裁)
```

- **同一全家桶账号**贯穿三端 → 账号池以**账号（uid/手机号）**为主键，端仅作为凭证来源与签到入口的属性，不各自建池。
- 签到收益是**账号级** Add-on Credits：在任一端 claim 即全端受益。**签到任务不要求三端都接入**，M1 只需一条可用凭证通道（QoderWork 或 IDE 任一）即可闭环 issue 承诺。

### 3.2 凭证获取策略（三层优先级）

| 层级 | 通道 | 适用端 | 合规性 | 说明 |
|---|---|---|---|---|
| **L1** | 客户端本地存储解密 | IDE（auth.v1.dat，DPAPI+AES-GCM）、QoderWork（R-1 裁决形态） | 中（读本机已登录用户自己的凭据） | 与现有 Buddy auth 扫描 / 豆包 DPAPI 解密同思路；设备头从存储透传；多账号并发场景按 §5.10 使用每账号稳定指纹 |
| **L2** | MITM 抓包透传 | QoderWork / IDE 主进程请求 | 中 | 复用 `device_proxy`（DEFAULT_TARGETS 加 Qoder 域名组），参照 `try_capture_doubao_credentials` 模式；捕获真实请求头保真度最高 |
| **L3** | 官方 PAT | CLI / 全端积分查询 | **高（官方认可）** | 引导用户在 `qoder.com.cn/account/integrations` 自建 PAT，`pt-` → jobToken 换 `jt-`；**M2 积分查询首选通道**。sash claim 是否接受 PAT 鉴权待验证（R-6）；若可用，PAT 即成为最合规的签到通道 |

### 3.3 应用域建模决策

对齐 Buddy 模式（一个前端 Tab 承载多个后端应用变体：Buddy Tab = WorkBuddy + CodeBuddy 双 TargetApp）：

- **前端**：单一 `AppKey = 'qoder'`，一个 Tab（QODER_NAV 五页），不按端拆 Tab。
- **后端**：`TargetApp` 新增两个变体 **`QoderIde`** 与 **`QoderWork`**（均参与快照切换编排）；**CLI 不进 TargetApp**（不参与登录态快照，走 PAT/切号桥路线）。
- **账号 id 前缀**：`"qd-" + sha256[..12]`（对齐 `wb-` 惯例）。
- 前端页面内用端类型标签（`IDE / Work / CLI`）区分凭证来源。

---

## 四、总体架构与复用映射

| Qoder 端 | 复用布局/模块 | 改造量评估 |
|---|---|---|
| **Qoder CN IDE** | `Layout::Icube` 全管线（`switcher/icube.rs` 快照白名单、`vscdb.rs` 全局键合并、WAL 防复活、`machine.rs` 6 层设备重置）；`icube_auth.rs` 凭据扫描 | **低**。加档案 + 候选目录 + 键名前缀验证；若 storage.json 键名不同构则参数化 ICUBE_ITEMS（新 `QODER_IDE_ITEMS`），不盲用 |
| **Qoder CN CLI** | 路线 A：PAT 环境变量注入（官方）；路线 B：切号桥（模板 `workbuddy_cli.rs`，F-06/F-59 八重防护纯函数架构）；路线 C：目录隔离（F-66 模式，`QODERCN_CONFIG_DIR` 天然支持） | **低～中**。M4 可选项 |
| **QoderWork CN** | 视 R-1 裁决：Chromium 内嵌登录 → `Layout::Chromium`（`chromium.rs` 快照即隔离，**零设备重置成本**）+ MITM 域名组；独立 auth 文件 → `Layout::Authfile`（`authfile.rs` 三层快照，但注意 `machine.rs` 对 Authfile 布局的设备重置是 F-48 预留空白） | **中**。取决于存储侦察结果 |
| **签到引擎** | 全新 `tasks/qoder_checkin.rs`，流程骨架对照 `wb_checkin.rs`（状态预检 → 领取 → 错误分类 → 冷却落盘），端点换 sash 体系，**claim 天然幂等**比 Buddy 已签容错更干净 | **中** |
| **积分引擎** | 全新 `tasks/qoder_credits.rs`，对照 `wb_credits.rs`（多接口 + 兜底 + 600s 缓存 + stale-on-error F-59 模式），输出契约对齐 `WbCreditsResult` | **中** |
| **调度** | `tasks/scheduler.rs` TASKS 表 + schtasks 双轨 + 启动补签（F-55 模式） | **低**，纯注册 |

---

## 五、模块级详细设计

### 5.1 应用域注册点清单（全量，前后端各 6 / 5 处）

**Rust 侧：**

| # | 动作 | 文件 | 内容 |
|---|---|---|---|
| B1 | 修改 | `src-tauri/src/switcher/mod.rs` | `TargetApp` 枚举加 `QoderIde, QoderWork` + `parse()` 分支（`"QoderIde" => …`，未知回退 TraeWork 不变） |
| B2 | 修改 | `src-tauri/src/switcher/profile.rs` | `profile_for()` 加两条档案（见 5.5），`profiles_dir: data/data/profiles_qoder`；`mac_supported: false`（灰度起步值，跨平台兼容要求见 5.9） |
| B3 | 修改 | `src-tauri/src/models.rs` | `Settings` 加 `qoder_ide_path / qoderwork_path`（+ 可选 `qoder_cli_pat`）；`default_proxy_domains()` 加 Qoder 域名组（`qoder.com.cn / openapi.qoder.com.cn / gateway.qoder.com.cn / qodercn.com 等，M0 抓包定稿） |
| B4 | 新增 | `src-tauri/src/commands/qoder/{mod,common,accounts,checkin,credits}.rs` | 仿 `commands/workbuddy/` 拆分；`QoderAccount / QoderSettings`（kv `qoder_settings`） |
| B5 | 修改 | `src-tauri/src/store/schema.rs` + `store/docs.rs` | `SCHEMA_VERSION 2→3`（`PRAGMA user_version` 迁移）+ DDL 四表（见 5.4）+ `ROW_TABLES` 白名单 + `qoder_pool_load/save`、`qoder_token_store_load/upsert`、`qoder_checkin_results_load/save`、`qoder_credits_history_*` 出口 |
| B6 | 修改 | `src-tauri/src/main.rs` + `tasks/mod.rs` + `tasks/scheduler.rs` | `mod qoder;`；invoke_handler 追加 `qoder_*` 命令；`run_cli_task` 分发加 `qoder-checkin / qoder-credits-snapshot`；TASKS 加两条（见 5.7）；setup 接线启动补签 |
| B7 | 修改 | `src-tauri/src/device_proxy/mod.rs`（M0 临时 / M2 正式） | `DEFAULT_TARGETS` 加 Qoder 域名组；`handler.rs` 新增 `try_capture_qoder_credentials()`（对照豆包模式：请求头 `Authorization` + auth 相关 Cookie 双侧捕获 → kv `qoder_captured_credentials`） |

**前端侧：**

| # | 动作 | 文件 | 内容 |
|---|---|---|---|
| F1 | 修改 | `src/types.ts` | `AppKey` 加 `'qoder'`；`ViewKey` 加 `qoder-overview/accounts/checkin/credits/settings`；`APP_HOME_VIEW.qoder='qoder-overview'`；新增 `QoderAccountView / QoderSettings / QoderCheckinRecord / QoderCreditPackage / QoderCreditsResult` 等 DTO（字段名严格 snake_case 对齐） |
| F2 | 修改 | `src/components/Sidebar.tsx` | `APP_TABS` 加 `{key:'qoder', label:'Qoder', title:'Qoder CN（IDE / Work / CLI）'}`；新增 `QODER_NAV` 五项；三分派改四分派 |
| F3 | 修改 | `src/App.tsx` | `renderView` 加 qoder 分支 + `pages/qoder/*` import |
| F4 | 修改 | `src/store.ts` | `defaultSettings()` 加 `qoder_*` 键；`switchTo / saveCurrentLogin` 联合类型加 `'QoderIde' | 'QoderWork'` |
| F5 | 修改 | `src/lib/tauri.ts` | 新增 `api.qoder` 命名空间（对照 `api.workbuddy`）；切换类 targetApp 联合类型同步扩展；页面级监听 `qoder-checkin-progress`（切换类复用既有 `switch-progress`，无需新事件） |

### 5.2 签到引擎 `tasks/qoder_checkin.rs`（M1 核心）

**端点表**（结构对照 `wb_checkin.rs::Urls`）：

```rust
struct QoderUrls {
    open_api: String,   // https://openapi.qoder.com.cn （R-7 确认；预留主备域名双探测位）
    // sash 活动
    campaigns: String,  // {open_api}/sash/api/v1/me/campaigns
    claim:    String,   // {open_api}/sash/api/v1/me/campaigns/{campaign_id}/claim
    // 用量
    usage:    String,   // {open_api}/api/v2/quota/usage
}
```

**选项与轮次**：

```rust
struct QoderCheckinOpts { uids: Vec<String>, skip_checked: bool, lazy_hours: u32 }
impl QoderCheckinOpts { pub fn daily() -> Self { Self { uids: vec![], skip_checked: true, lazy_hours: 24 } } }
// 轮次锁：static QODER_ROUND_LOCK: tokio::sync::Mutex<()>（对照 WB_ROUND_LOCK）
```

**`checkin_do()` 核心流程**（单账号）：

```
1. 取凭证: effective_creds()（客户端存储解密 vs token store 双源，谁新用谁）
   → 无凭证: 直接短路 kind="fail"，message="无可用凭证（{note}）"（auth 仅指 401 可自愈场景，不走 auth）
2. ensure_fresh(): token 过期 → deviceToken/refresh 刷新一次（401 → 仅重试一次，对照 wb_checkin）
3. GET campaigns，请求头必须带:
   Authorization: Bearer {token}
   Cosy-ClientType: {R-4 确认值，缺省 "10"}     ← 缺失会导致静默空列表！
   Cosy-MachineId / Cosy-MachineToken: {从客户端存储/抓包透传，仅当可获取}（R-5 验证是否必需）
   User-Agent: 对齐客户端真实 UA（M0 抓包留存）
4. 响应解析（dig 信封穿透，注意 fs_utils::dig 候选键禁写 "data"）:
   campaigns 为空 → 视为"活动不可用"，kind="fail"，note="empty_campaigns"（与已签区分，触发 UI 提示）
   过滤 claimStatus==CLAIMABLE 的 campaignId 列表（双活动自然全覆盖）
5. 逐个 POST claim（间隔 1~3s 随机抖动 jitter）:
   data.status==CLAIMED → 成功；data.replayed==true → already（幂等，非错误）
   奖励数额取 benefit.amount；缺失时用签到前后 /quota/usage 余额差值兜底（对照 deep_reward_dig）
6. 错误分类 kind ∈ "success" / "already" / "fail" / "auth"（对齐 wb 四值；auth 仅指 HTTP 401
   可自愈场景——触发一次重试，重试仍失败落库时归并为 fail；无凭证为 fail 短路，不产生 auth）
7. 结果写 qoder_checkin_results（90 天滚动）；NDJSON emit "qoder-checkin-progress"
   行格式 {stage, status: info|ok|warn|error|running|skip, message, time}（与 Buddy 前端组件兼容）
```

**调度时刻设计**：默认 `10:15`（单次覆盖双活动，见 §2.2 结论）。任务名 `qoder-checkin`。

**风控合规内建**（区别于纯脚本项目）：
- 单账号默认启用；多账号为显式开启项，UI 固定提示条款风险；
- 请求间隔 jitter、失败退避（30 分钟冷却，同 `RETRY_COOLDOWN_MS`）、绝不重试轰炸；
- 设备指纹头单账号默认只透传；**多账号并发显式开启时按 §5.10 为每账号绑定稳定指纹**（非随机轮换）；缺设备头导致失败时明确报"需要客户端在线一次以刷新设备态"，不静默降级。

### 5.3 积分引擎 `tasks/qoder_credits.rs`（M2）

**三通道取数（按可用性降级）**：

| 通道 | 鉴权 | 接口 | 说明 |
|---|---|---|---|
| A（首选，官方） | PAT `pt-` → `POST /api/v1/jobToken/exchange` 换 `jt-` | `GET /api/v2/quota/usage` | UI 提供 PAT 引导卡（`qoder.com.cn/account/integrations`，提示"仅关闭页面前可见一次"） |
| B | 客户端 token（L1/L2 捕获） | 同上 | 随客户端登录态可用；无稳定性承诺，响应结构 M0 固化（R-7） |
| C（兜底） | — | CLI `/usage` 输出解析 | 仅当本机装有 CLI 且已 `/login`；解析脆弱，仅展示降级徽标 |

**输出契约**（对齐 `WbCreditsResult`，前端可复用积分页组件骨架）：

```json
{ "ok": true, "cached": false,
  "accounts": [{ "user_id", "name", "ok", "plan_credits", "addon_credits",
                 "packages": [{ "amount", "expire_at", "source: plan|addon_checkin|addon_gift" }],
                 "source": "pat|client_token|cli_parse|fetch_failed|none", "fetched_at" }],
  "total_balance": 0,
  "product_line_hint": "family_bucket | lingma_legacy | mixed" }
```

- 缓存：kv `qoder_credits_cache`，`CACHE_TTL_SECS=600` + **stale-on-error**（F-59 模式），按 account_id 区分。
- 快照定时写 `qoder_credits_history`（365 天裁剪），支撑趋势图与到期日历（复用 `ExpiryCalendar` 组件）。
- **产品线识别**：检测到 `lingma_` 特征（原灵码订阅）时 `product_line_hint` 给出"该订阅 Credits 不在 QoderWork/CLI 生效"提示，避免用户困惑（官方 FAQ 高频问题）。

### 5.4 账号体系 `commands/qoder/` 与存储

**数据模型**（对照 `WorkBuddyAccount` 裁剪）：

```rust
struct QoderAccount {
    id: String,                 // "qd-" + sha256(token|uid)[..12]
    uid: String,                // /api/v1/userinfo 返回；手机号掩码做展示
    nickname: String,
    phone_masked: String,
    plan: String,               // free | pro | pro+ | teams(只读提示)
    credential_source: String,  // ide_store | qoderwork_store | mitm | pat | cli
    token_expires_at: Option<i64>,
    needs_relogin: bool, relogin_reason: Option<String>,
    group_id: Option<String>, note: Option<String>,
    credits_balance: Option<i64>, credits_fetched_at: Option<i64>,
    device_profile: Option<QoderDeviceProfile>,  // v1.2 §5.10：每账号稳定指纹（machine_id/device_id/umid/mac）
}
```

**导入通道**：
1. **IDE 存储发现**：`icube_auth.rs` / `trae_apps.rs` 的候选目录列表加入 `com.qodercn.app.stable`（R-2 验证键名前缀；若非 `iCubeAuthInfo://` 则新写 Qoder 侧发现函数）；`auth.v1.dat` 解密复用 Chromium 解密链路思路（DPAPI + AES-GCM，参照 `local_capture.rs` 的 Windows DPAPI 实现）；解密入口收敛为平台门面（Windows DPAPI / macOS Keychain 分支预留，见 5.9），AES-GCM 主体跨平台复用。
2. ~~**QoderWork 存储/MITM 捕获**：R-1 裁决后实现（MITM 走 B7 捕获函数）。~~（v1.4.1 作废：QoderWork 即 Qoder CN 本体，无独立存储端；MITM 捕获经 R-5 实证已不需要）
3. **PAT 手工录入**：`qoder_account_import_pat(name, pat)`——最简通道，M1 即可支持（配合 R-6 验证 PAT 能否驱动 sash claim）。
4. 幂等入池复用 `find_uid_or_id` 模式（uid 优先 / id 兜底、保留旧 id），防止换发 token 重复入池导致快照/分组引用悬空。

**存储（schema v3）**：

```sql
-- store/schema.rs 新增，ROW_TABLES 白名单同步
CREATE TABLE qoder_accounts        (pk TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE qoder_tokens          (pk TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE qoder_checkin_results (pk TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);  -- 90 天滚动
CREATE TABLE qoder_credits_history (pk TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL);  -- 365 天裁剪
```

- 行文档格式与读写出口严格走 `store::db(&data_dir)`（禁散落 JSON）；敏感 token 收敛进 `vault.rs`（Stronghold），SQLite 侧仅存脱敏视图——对齐既有脱敏红线（`mask_secret`，token 不进日志/事件/UI）。
- kv 键一览：`qoder_settings`、`qoder_tokens_meta`、`qoder_credits_cache`、`qoder_usage_official_cache`（预留）、`qoder_captured_credentials`（MITM）、`qd_cli_rotate_state`（M4 预留）。

### 5.5 登录态切换（M3）

**档案表新增两条**（`switcher/profile.rs`，`profile_for()` 分派）：

```rust
// Qoder CN IDE —— Icube 布局（R-2 通过后）
AppProfile {
    app_name: "QoderIde",
    layout: Layout::Icube,
    data_dir: roaming_dir("com.qodercn.app.stable"),   // Win: %APPDATA%\...；mac: ~/Library/Application Support/...（5.9 预留）
    profiles_dir: data/data/profiles_qoder,
    settings_path_key: "qoder_ide_path",
    graceful_wait_secs: 5,
    proc_names: ["Qoder CN"], exe_names: ["Qoder CN.exe"],
    exe_candidates: ["%LOCALAPPDATA%\\Programs\\Qoder CN\\.qoder-versions\\<ver>\\Qoder CN.exe"],  // 版本化目录，需通配
    mac_supported: false, mac_bundle_ids: [],   // 灰度起步；macOS 实施时置 true + 补 bundle id（5.9），字段本期即预留
}
// QoderWork CN —— 布局待 R-1 裁决（Chromium 优先：快照即隔离、零设备重置成本）
```

**关键改造与防串台**：
- `switcher/icube.rs`：`ICUBE_ITEMS` 中 `aha`、`Partitions\trae-webview` 等为 Trae 专有——**新增 `QODER_IDE_ITEMS` 参数化白名单**而非直接复用（保留 `storage.json / state.vscdb(+wal/shm) / machineid / Local State / Local Storage / Session Storage` 等通用项，M0 按 Qoder 实际目录裁剪）；`restore_icube` 的 WAL 删除与对称恢复防护照搬。
- `switcher/vscdb.rs`：全局键合并的 `KEY_PROJECT_FOLDERS` / `KEY_RECENT_PATHS` 键名是否同构待 R-2；Qoder 侧若无 solo-lite 分区键则合并逻辑天然安全。
- `switcher/machine.rs`：Icube 布局 6 层设备重置可用；**若 QoderWork 裁决为 Authfile 布局，设备重置是 F-48 预留空白**——M3 范围内对 QoderWork 禁用设备重置入口并在 UI 标注"暂不支持"，不阻塞主线。
- 编排层 `switch_flow`（预检快照槽 → stop_app → backup last → 防误覆盖守卫 → restore → 恢复后校验 → set_current → start_app → confirm_switch）**整体复用**，仅 `trae_apps.rs` 类账号发现函数需加 Qoder 分支。
- `commands/switch.rs`：JWT 预检仅 TRAE 双应用，Qoder 放行（同 WB/CB 处理）。

### 5.6 CLI 支持（M4，可选）

- **路线 A（M1 就绪，随 PAT 通道）**：`QoderAccount(credential_source="pat")` + 环境变量 `QODERCN_PERSONAL_ACCESS_TOKEN` 注入说明；提供"复制为 CLI 环境变量"按钮。
- **路线 B（切号桥）**：仿 `workbuddy_cli.rs` 新增 `qoder_cli.rs` 纯逻辑层（`decide_target` 八重防护 + 积分紧迫度排序）+ `commands/qoder/cli.rs`；写 `~/.qoder-cn` 凭据或 settings env 键——**以 R-3 侦察结果定**（`~/.qoder-cn` 目录结构与 token 格式）。
- **路线 C（目录隔离，F-66 模式）**：`QODERCN_CONFIG_DIR` 天然支持按账号独立目录——若 CLI token 为明文落盘，此路线比写 token 更干净，M4 评估二选一。

### 5.7 调度

**应用内调度器**（`tasks/scheduler.rs::TASKS`）：

| key | 名称 | 默认时刻 | kind | 门控 |
|---|---|---|---|---|
| `qoder-checkin` | Qoder 签到 | **10:15** | fix | `qoder_auto_checkin`（settings，默认 true） |
| `qoder-credits-snapshot` | Qoder 积分快照 | 23:40 | credits | `qoder_credits_sync_enabled`（默认 true） |

- 配置键：`qoder_checkin_hhmm` / `qoder_credits_sync_hhmm`——**必须在 `state.rs::settings()` 零值回填**（时刻为空 = 从未配置 → 回填默认开；对照 `wb_growth_hhmm` 的教训）。
- schtasks 双轨（macOS 对应 launchd，OS 注册统一走 `register_os_task()` 门面，见 5.9）：`AIWorkAssistant_QoderCheckin_<HHMM>`（DAILY）+ TR 命令行 `cmd /c set "AIWORKDATA_DIR=<dir>" && "<exe>" --task-run qoder-checkin`；`tasks/mod.rs::run_cli_task` 分发表加 `qoder-checkin / qoder-credits-snapshot`。
- 启动补签：`commands/qoder::startup_auto_checkin`（F-55 模式：启动 60s 后 + 轮次锁），覆盖"开机即补当日"场景。

### 5.8 前端页面（对照 `pages/buddy/` 裁剪复刻）

| 页面 | 内容 | 相对 Buddy 的差异 |
|---|---|---|
| `QoderOverview.tsx` | 统计卡 + 近 30 天签到趋势 + 积分榜 + 配置导航 Step（装客户端 → 导入账号/PAT → 开调度） | 无成长中心（Qoder 无对应玩法） |
| `QoderAccounts.tsx` | 账号池列表 + 凭证来源徽标（IDE/Work/MITM/PAT）+ 切换右键菜单（IDE/Work 双应用）+ PAT 导入弹框 | 无环境重置 16 项（M3 后按需补）；无会话迁移 |
| `QoderCheckin.tsx` | 签到控制卡（手动触发 + 进度 NDJSON 流）+ 双活动说明卡（0 点签到 / 10 点登录奖励） | `QODER_ROUND_LOCK` 前端互斥提示对照 `BuddyCheckin.tsx` |
| `QoderCredits.tsx` | KPI 卡（Plan/Add-on/总余额）+ 近 7 日趋势 + 到期日历（`ExpiryCalendar` 复用）+ 产品线提示横幅 | 数据源三通道徽标（pat/client_token/cli_parse） |
| `QoderSettings.tsx` | 客户端路径 + 签到/快照定时 + 自动签到开关 + 合规提示（条款风险固定展示） | — |

### 5.9 平台兼容设计（macOS 后续实施预留）

> 原则：Windows 先行交付，**所有平台差异点收敛到可替换边界**——macOS 端实施时零重构、只补实现，不出现架构级返工。

| 差异点 | Windows（本期实现） | macOS（预留实现） | 兼容性要求（本期就做） |
|---|---|---|---|
| 数据目录 | `%APPDATA%\com.qodercn.app.stable` | `~/Library/Application Support/com.qodercn.app.stable`（待侦察确认） | 路径一律经 `dirs` 抽象取值（`roaming_dir()` 已跨平台），**禁止硬编码 `%APPDATA%` 等字面量** |
| 凭据解密 L1 | `Local State`→DPAPI→AES-GCM→`auth.v1.dat` | Keychain `Safe Storage`→AES-GCM（同 Chromium macOS 链路） | 解密入口收敛为 `decrypt_local_secret()` 单点门面；AES-GCM 主体天然跨平台 |
| 进程发现/停止 | `proc_names` / `exe_names`（sysinfo） | 同左 + bundle id 辅助 | `sysinfo` 跨平台；路径拼接一律 `PathBuf::join`，不手拼分隔符 |
| 调度注册 | schtasks DAILY | launchd LaunchAgent plist | 注册收敛到 `register_os_task()` 门面，本期 Windows 分支完整实现，macOS 分支留 TODO 桩 |
| OS 档案字段 | `mac_supported: false` | 置 `true` + 补 `mac_bundle_ids` | 档案结构**本期即持久化预留字段**，F-75 灰度门控逻辑直接复用，不写死平台 |
| MITM | 系统证书信任 | 钥匙串信任 | 复用既有 `device_proxy` 跨平台层，Qoder 域名组不引入平台耦合 |
| CLI | `~/.qoder-cn`（R-3 侦察） | 同构 | CLI 三条路线均天然跨平台 |

- **R 侦察注记**：本机为 Windows，macOS 侧目录 / Keychain 细节不在 M0 范围；macOS 实施前补 **R-11（macOS 侦察）**，产出物格式同 R-1 / R-8。

### 5.10 多账号并发与设备指纹（v1.2 新增，需求方 2026-09-27 确认）

> 需求：多账号**并发使用**（非仅切换）。Qoder 服务端以设备指纹做"同一设备"归并，多账号共存必须为每账号绑定**稳定且互异**的设备身份。核心参考：QoderPatcher（okky-x0f/qoder-creator）+ QoderGateway 账号池，实现级细节已归档 `docs/completed/Qoder外部项目调研归档.md`。

**设计要点**：

1. **每账号稳定绑定（照抄 QoderGateway 已验证模型）**：账号池记录新增 `device_profile`（machine_id / device_id / umid；mac 仅 macOS 需要，Windows-only 未实现），账号导入时生成一次、之后恒定复用（四条入池路径 accounts/ide_store/oauth/data_io 同语义 + ensure_pool_profiles 惰性回填兜底）；请求时 machine_id 取库，machine_token / machine_type 可现场随机（QoderGateway 实测二者与 machine_id 无服务端强绑定仍可通过，R-12 复核 CN 端）。
2. **指纹格式合法性（QoderPatcher 同源算法）**：
   - `machine_id` = 32 位小写 hex（实现取 sha256(uuid4)[..32]，原 QoderPatcher `md5(random)` 种子空间仅 1e7，移植按归档 §6.1 建议增熵）；
   - `ms_deviceid` / `umid` = uuid4 标准格式；`mac` = 6 字节随机冒号分隔（仅 macOS 需要）；
   - 落盘格式（M3 实装三层，`switcher/machine.rs::apply_qoder_fingerprint`，R-12 实测读取位）：裸文本写 `<data_dir>/machineid`（无 BOM，不存在则创建）；`<data_dir>/User/globalStorage/storage.json` 点号键 `telemetry.machineId / sqmId / devDeviceId`（仅替换已存在键，三键同值绑定 = 单标识域模型）；state.vscdb ItemTable `storage.serviceMachineId`（upsert）。
3. **请求头映射**：sash 通道 `Cosy-MachineId` 取账号 device_profile.machine_id；COSY 签名链（cosy-key / machinetoken / clientip 硬编码 `169.254.198.161`）仅 algo 端点需要，签到不引入，留作门槛升级备选。
4. **本地存储联动（M3）**：IDE/桌面端切换或并发时同步覆写本地指纹文件（对齐 `machine.rs` 六层重置思路，Qoder 侧文件清单以 R-12 实测为准）；CLI 路线无需覆写桌面文件（`~/.qoder/.auth/id` 可直写，其 `user` 文件解密 key=machine_id[:16] 必须与 id 文件一致）。
5. **风险接受**：属条款灰色用法（§七 风险 1），设置页固定风险提示；不做批量注册/验证码绕过。

**侦察前置 R-12**：CN 端指纹"真源"文件清单（Electron data 目录 vs `~/.qoder/.auth/`）、服务端对 machine_id ↔ token 绑定校验强度、cosy-clientip 是否可变。

### 5.11 Qoder 入网关池接入技术方案（v1.4.2 新增；R-13 闭合后立项动工）

> 定位：将 Qoder 账号池接入应用级 API 网关（对齐 WorkBuddy `api_server` 已有能力），对外暴露 Qoder 模型资源。**硬前置 = R-13 闭合**——Qoder 官方对话 API（gateway.qoder.com.cn）请求形态目前零情报，缺口未闭合前不写上游转发层，杜绝无端点的假实现。

**复用清单（现成能力，直接挂接）**：
- `api_server` ApiPool 调度引擎：多上游负载 / 失败切换 / ErrKind 错误分类 / retry 表；
- `wb_sse` 三协议透传（SSE / 流式帧 / 长连接语义）；
- `api_keys` 鉴权与配额管理（网关入口不变）；
- Qoder 账号基建（§5.4 / §5.10）：池存储、双源凭证 `ensure_fresh`、每账号稳定指纹 `effective_creds` 单出口——上游凭据选取与续期零新造。

**新写清单（R-13 闭合后动工）**：
1. `api_server/qoder_upstream.rs`：Qoder 上游转发层（请求构造、鉴权头映射、流式协议对接、错误归一化入 ErrKind）；
2. 对话执行路径模块：按 R-13 固化的请求形态实现（端点常量进附录 A 速查表）；
3. `sync_from_qoder`：Qoder 池账号 → 网关上游账号同步（复用导入幂等合并语义）；
4. `qoder_upstream_accounts`：网关侧上游账号管理命令域；
5. `TargetPool` 扩展：target 增加 `qoder` 类别（路由/资源枚举）；
6. 前端资源页：模型资源列表与上游健康展示（对齐 Buddy 资源页模式）。

**侦察 SOP（R-13，复用 §六 MITM 设施）**：`proxy_start`（device_proxy）→ 系统代理指向 → Qoder 客户端发起真实对话（含流式输出）→ `handler.rs` 落盘 → 提取 URL / 方法 / 鉴权头 / 请求体 / 流式帧格式（token 脱敏）→ 结论落 `docs/completed/f80-qoder-recon/` 并回填附录 A。

---

## 六、M0 侦察计划（M1 先决，产出物驱动）

> 原则：先固化后编码。每项产出物落 `docs/completed/f80-qoder-recon/`（截图 + 抓包 HAR + 结论 md），缺口未闭合前对应模块不动工。

| # | 缺口 | 验证手段 | 产出物 → 影响的设计决策 |
|---|---|---|---|
| R-1 ✅ | Qoder CN 数据目录与凭据形态 | 枚举 `%APPDATA%`/`%LOCALAPPDATA%`；procmon 观察登录时文件访问 | 目录清单 + 凭据文件样本 → **Chromium vs Authfile 布局裁决**（✅ 本机枚举闭合：`%LOCALAPPDATA%\Qoder CN\` 仅 Installer/Launcher 引导器无用户数据；用户数据 `%APPDATA%\QoderCN\` 已由 QoderIde/Icube 管线覆盖。v1.4.1 闭合：QoderWork 即 Qoder CN，无独立端、无额外布局） |
| R-2 ✅ | IDE `state.vscdb` / `storage.json` 键名同构性 | 打开 `%APPDATA%\com.qodercn.app.stable`；检查 `User/globalStorage/`；diff `iCubeAuthInfo://` 前缀 | 键名清单 → **Icube 管线直接复用 or 参数化 ICUBE_ITEMS**（✅ 已落地：参数化 `QODER_IDE_ITEMS`，随 M3） |
| R-3 ✅ | CLI `~/.qoder-cn` 目录结构与 token 格式 | `/login` 前后目录 diff；检查是否明文 | 文件结构说明 → **切号桥 vs 目录隔离路线裁决**（✅ 已裁决：`.qoder-cn` 无明文凭证、无独立凭证通道 → status 只读桥，随 M4） |
| R-4 ✅ | CN 端 `Cosy-ClientType` 真实值 | MITM 抓 `/sash/api/v1/me/campaigns` 请求头 | 头部样本 → **端点表常量定稿**（`Cosy-ClientType: "10"`） |
| R-5 ✅ | sash 接口对 `Cosy-MachineId/MachineToken` 校验强度 | 真实头重放 vs 缺头对比 | 结论 → **纯 HTTP 签到可行性 / 是否需 CDP 兜底**（经 QoderGateway 实证：服务端无强绑定校验，纯 HTTP 可行，无需 CDP） |
| R-6 ✅ | PAT（pt-/jt-）能否驱动 sash claim | 用 PAT exchange 后调 claim（**幂等安全**） | 结论 → **PAT 能否成为最合规签到通道**（`/api/v1/me/jobToken` 抓包固化，PAT 独立闭环可用） |
| R-7 ✅ | `/api/v2/quota/usage` CN 域响应结构 | Bearer 直调 + 主备域连通性（openapi / api2 备用） | 响应样本 → **credits 解析器与缓存结构定稿**（三通道已出数） |
| R-8 ✅ | IDE `auth.v1.dat` CN 版是否同构国际版 | 存储分析 + 国际版解密逻辑（sunp-1 方案）套测 | 解密验证 → **L1 凭证通道实现方式**（✅ CN 凭证真源实测为 state.vscdb `secret://aicoding.auth.userInfo`，DPAPI+AES-GCM 解密落地 `commands/qoder/ide_store.rs`） |
| R-9 ✅ | 双活动 campaignId/campaignKey 枚举 | 抓 `GET /me/campaigns` 全量响应（两活动各一次） | campaign 清单 → **双活动过滤逻辑与 UI 文案**（campaigns 动态拉取 + claim 幂等，双活动已跑通） |
| R-10 ✅ | device flow 轮询参数（poll 入口、间隔、授权页行为） | 手动走一遍 `/login` + 抓包 | 时序图 → **ensure_fresh 刷新实现**（设备流导入已实现） |
| R-12 ✅ | 本地设备指纹字段全清单与请求头映射（多账号并发前置，v1.2 新增） | 外部仓库预研（qoder-creator / QoderGateway，已归档 `docs/completed/Qoder外部项目调研归档.md`）+ 本机数据目录比对 + 抓包头对照 | 指纹字段清单 → **§5.10 每账号稳定指纹实现**（外部预研 ✅ 归档；本机实测五点 ✅ v1.4 闭合——指纹本地存储覆写随 M3 落地） |
| R-13 ⏳ | Qoder 官方对话 API（gateway.qoder.com.cn）请求形态：URL / 方法 / 鉴权头 / 请求体 / 流式帧格式（v1.4.2 新增，网关池立项硬前置） | MITM SOP：`proxy_start` → Qoder 客户端真实对话（含流式）→ `handler.rs` 落盘提取（token 脱敏） | 请求形态样本 → **§5.11 网关池接入实现**（缺口未闭合前不写上游转发层） |

> **侦察进度（2026-09-27 审计）**：✅ 已闭合 6 项（R-4/R-5/R-6/R-7/R-9/R-10）。其中 R-5 经 QoderGateway 实证「服务端对 machine_id ↔ token 无强绑定校验」→ 纯 HTTP 签到可行、M4 CDP 兜底关闭；R-9 以 campaigns 动态拉取 + claim 幂等替代静态枚举；R-12 外部预研已归档并支撑 §5.10 落地（◐ 半闭合）。⏳ 未闭合 4 项（R-1/R-2/R-3/R-8）均属 M3/M4 范围，按「缺口未闭合前对应模块不动工」原则随里程碑顺延；R-12 本机实测五点见 `Qoder外部项目调研归档.md` §6.2。
> **侦察进度（v1.4 更新）**：✅ 闭合 11 项（新增 R-2/R-3/R-8/R-12 本机实测）；R-1 ◐（本机枚举闭合，QoderWork 档案因 2027-09 停运延后另议）。M3/M4 全部缺口已随实现闭合。
> **侦察进度（v1.4.1 终版）**：✅ 12 项全闭合（R-1 随产品同一性澄清终审——QoderWork 即 Qoder CN，`%LOCALAPPDATA%\Qoder CN\` 仅引导器、用户数据 `%APPDATA%\QoderCN\` 已覆盖，无独立端缺口）。
> **侦察进度（v1.4.2）**：上述 12 项维持全闭合；新增 R-13 ⏳（Qoder 网关池立项硬前置，闭合前不写上游转发层）。

**MITM SOP（复用现有设施）**：`proxy_start`（`device_proxy`）→ 系统代理指向 → Qoder 客户端操作签到/用量页 → `handler.rs` 请求日志落盘 → 提取端点与头部（脱敏 token）。Qoder 域名组先以 `default_proxy_domains()` 临时追加项注入。

---

## 七、风险与合规

| # | 风险 | 等级 | 对策 |
|---|---|---|---|
| 1 | **平台风控**：条款明确"同一设备/手机号/支付宝账号"多维去重；`Cosy-MachineId/Token` 设备指纹与服务端校验关联 | 高 | jitter + 失败退避；多账号并发采用**每账号稳定指纹**（§5.10，非随机轮换，降低异常检测面）；UI 明示条款与封号风险；不提供批量/注册类能力 |
| 2 | **活动终止/规则变更**：EN 帮助页曾有 "[Ends July 30]" 陈旧标题先例；活动可随时下线 | 中 | claim 幂等 + `empty_campaigns` 显式区分；设置页明示"功能以活动实际可用为准"；端点表常量集中可改 |
| 3 | **接口无稳定性承诺**：sash / quota/usage 均为逆向所得，可能随版本变更 | 中 | M0 固化 + 响应 dig 宽容解析 + `source` 降级徽标；R-5 若纯 HTTP 被封，兜底 CDP 方案（社区实测 `--remote-debugging-port 9334` + growth-page target，M4 评估） |
| 4 | **Qoder CN（全家桶文案别名 QoderWork）2027-09-05 停运** | 中（长期） | 架构不绑死 QoderWork：签到引擎面向 sash 账号体系而非特定端；IDE/CLI 凭证通道独立可用 |
| 5 | **产品线混淆**（原灵码 vs 全家桶不互通） | 低 | `product_line_hint` + UI 横幅（官方 FAQ 高频困惑点） |
| 6 | **Icube 白名单误伤**：Qoder fork 内部目录与 Trae 不完全同构，盲目复用 ICUBE_ITEMS 可能漏抓/错删 | 中 | 参数化 `QODER_IDE_ITEMS`，M0 实测后定稿；restore 对称恢复防护兜底 |
| ~~7~~ | ~~Authfile 布局设备重置空白~~（v1.4.1 作废：QoderWork 即 Qoder CN，Icube 布局已实现，无 Authfile 分支） | — | — |

---

## 八、里程碑与任务拆解

### M0 — 侦察固化（P0，M1 先决）
- [x] ✅ M1 依赖项全部闭合：R-4/R-5/R-6/R-7/R-9/R-10 + R-12 外部预研归档（产出物：端点表常量定稿于附录 A、Cosy 头家族、jobToken 三通道、campaigns 动态拉取 + claim 幂等、device flow 时序、§5.10 指纹模型）
- [x] ✅ M3/M4 依赖项闭合（v1.4.1 终版）：R-1/R-2/R-3/R-8/R-12 全部闭合（R-1 终审于产品同一性澄清：QoderWork 即 Qoder CN，无独立端缺口）
- **退出标准**：§五中所有标注"待 R-x"的决策点全部落定（M1 相关 ✅ 已落定）

### M1 — 签到 MVP（P1，对应 issue #36 承诺的最小闭环）
- [x] ✅ `store/schema.rs` SCHEMA_VERSION 2→3 + 四表（qoder_accounts / qoder_tokens / qoder_checkin_results / qoder_credits_history）+ docs.rs 出口
- [x] ✅ `tasks/qoder_common.rs`（Creds / effective_creds 双源 / ensure_fresh / build_auth_headers 含 Cosy 头；v1.2：effective_creds 挂接 §5.10 指纹注入为唯一出口）
- [x] ✅ `tasks/qoder_checkin.rs`（campaigns → claim 幂等 → 错误四分类 → 冷却落盘 → NDJSON）
- [x] ✅ `commands/qoder/{mod,common,accounts,checkin}.rs` + PAT 导入通道（`credential_source="pat"`；R-6 通过，PAT 可独立完成签到闭环）
- [x] ✅ 调度三件套：TASKS 注册 + schtasks 双轨 + 启动补签；`state.rs` 零值回填
- [x] ✅ 前端：F1~F5 注册点 + `QoderCheckin.tsx`（Overview 统计卡）
- [x] ✅ 🆕 claim 失败复查恢复（v1.4.1 补齐）：claim 网络异常（status=0）后复查 campaigns（GET 幂等安全，绝不重发 claim），该活动已变 CLAIMED 视为 already（`campaign_is_claimed` 纯函数 + `recheck_claimed`，recon §3.2 cpa-multi-plugins 方案）
- **验收**：手动触发签到成功领取双活动收益，结果入表 ✅；调度时刻自动执行 ✅；重复执行幂等不报错 ✅

### M2 — 积分看板（P1）
- [x] ✅ `tasks/qoder_credits.rs` 三通道 + 600s 缓存 + stale-on-error + 快照任务
- [x] ✅ PAT 引导卡（Overview 引导 + 账号页 integrations 跳转 + 粘贴导入 + "仅关闭页面前可见一次"提示）
- [x] ✅ `QoderCredits.tsx`（KPI + 趋势 + ExpiryCalendar + 产品线横幅）+ `QoderOverview.tsx` 完整版
- **验收**：三通道任一可用即可出数 ✅；缓存命中/降级徽标正确 ✅；到期日历与 FEFO 规则一致 ✅

### M3 — 账号池与切换（P2）
- [x] ✅ 账号发现通道：PAT ✅ / OAuth 设备流（client 捕获）✅ / IDE 存储 ✅（`qoder_ide_scan`：state.vscdb `secret://aicoding.auth.userInfo` DPAPI+AES-GCM 解密，R-8 闭合）/ QoderWork 存储 ✅（v1.4.1 澄清：即 Qoder CN 本体存储，由 IDE 存储通道覆盖，无独立端）；vault 收敛 ✅（qoder_tokens 统一存储）
- [x] ✅ `QoderIde` Icube 档案（参数化 `QODER_IDE_ITEMS`，R-2 闭合）+ 切号快照保存/恢复（`data/profiles_qoder/<account_id>/`，恢复自动注入绑定指纹 §5.10）；`QoderWork` 档案 ✅（v1.4.1 澄清：同一产品，`TargetApp::Qoder` 单档案覆盖，无独立变体）
- [x] ✅ `QoderAccounts.tsx`：列表 / PAT·OAuth·IDE 扫描三通道导入 / 设备指纹列与弹框 / 一键切换 / 快照管理 / CLI 状态条（M4）/ 底部通道与刷新说明
- [x] ✅ 多账号并发（§5.10）：指纹生成 ✅ / 每账号持久化（一次生成永不轮换）✅ / 请求注入（effective_creds 单出口 + 签到·列表·导入三路 ensure 漏斗）✅ / 本地存储覆写 ✅（切换恢复时注入 machine_id，R-12 本机实测闭合）
- [x] ✅ 🆕 敏感字段出库剥离：出库视图白名单不含 token 明文（`QoderAccountView` 仅含 `phone_masked`/`token_kind`/`has_credential`/指纹前 8 位；完整指纹仅用户主动查看弹框展示）（补自 recon §7 QoderGateway 方案，已满足）
- **验收**：IDE 双账号切换后 storage.json/vscdb 身份正确、WAL 无复活、项目列表保留；恢复失败自动回滚 last 槽

### M4 — CLI 桥与增强（P3，可选）
- [x] ✅ CLI 桥（按 R-3 裁决落地，v1.4）：`~/.qoder-cn` 实测无明文凭证（`.auth/` 无 token）→ **无独立 CLI 凭证通道**，落地 `qoder_cli_status` status 只读桥（`commands/qoder/cli_status.rs`，白名单透传 + 丢 schema_version，绝不写回、绝不注入凭证）+ `QoderAccounts.tsx` CLI 状态条；`/usage` 解析兜底通道 ⏳ 按需另议
- [x] ✅ CDP 兜底方案评估 → **已关闭**：R-5 经 QoderGateway 实证纯 HTTP 可行，无需 CDP 兜底
- [x] ✅ 🆕 导出导入 / env_reset（v1.4.2）：`commands/qoder/data_io.rs` 导出（`kind=aiwork-qoder-pool` + version:1 + exported_at，可选凭证副本——含凭证文件等同密码）/ 导入（kind 强校验 + id 规则 `qd-<12位hex小写>` 白名单 + uid 优先幂等原位更新 + **指纹红线：device_profile 仅本地为空才补入，绝不覆盖（覆盖 = 轮换指纹）** + group_id/note/needs_relogin/credits_* 仅新增采用 + 凭证经 `creds_of` 宽容解析回写 token store，version≠1 拒写）+ `commands/qoder/env_reset.rs` 8 项语义块（vscdb_auth / storage_json / machine_identity / local_storage / network_cookies / session_storage / shared_client_cache / cli_auth `~/.qoder-cn`；Qoder 无 SSO 注销对应物故无 Keycloak 步骤；执行前 `graceful_kill_app("Qoder CN")`，process.rs 补 Qoder 进程画像）+ `QoderAccounts.tsx` 工具栏三入口（导出弹框凭证警示 / hidden file input 导入 / 重置 = 拉清单默认勾选 exists → 逐项勾选（未检测到 Badge）→ 二次确认 → 逐项结果 Badge）
- [x] ✅ 🆕 6h 定时刷新兜底（v1.4）：`tasks/qoder_refresh.rs` 全池 `ensure_fresh(lazy_hours=7)`——7h 惰性窗 > 6h 调度间隔，任一 tick 必落窗（凭证永不过夜）；调度器新档位 `SchedPlan::EveryHours(6)`（kind "refresh"，无 settings 键恒开、空池空转，对齐 doubao-keepalive 惯例）+ `run_cli_task` CLI 分发；顺带 best-effort 积分 fresh 拉取（同日快照覆盖不产生多余历史点）；顺带修正调度日志「（每日 每小时）」为「（每小时/每6小时）」
- [ ] （独立立项）Qoder 入网关池——v1.4.2 由需求方立项为独立任务：**动工硬前置 R-13**（gateway.qoder.com.cn 对话 API 请求形态零情报，缺口未闭合不写上游转发层、不做无端点的假实现）；复用/新写清单见 §5.11，不在 F-80 验收范围

### M5 — macOS 端实施（后续规划，另起排期）
- [ ] ⏳ 前置：R-11 macOS 侦察闭合（数据目录 / Keychain / launchd / bundle id）
- [ ] ⏳ 按 §5.9 差异表补平台分支：`mac_supported` 置 true + `mac_bundle_ids` 填充、`decrypt_local_secret()` Keychain 分支、`register_os_task()` launchd 分支
- **预期**：得益于 5.9 兼容设计，仅需补平台分支实现，无架构级改动（未启动，维持后续规划）

---

## 九、验收标准（总）

1. **功能**：双活动每日自动领取成功（幂等重跑无副作用）；积分余额/到期/趋势与官网 Usage 一致；IDE 账号切换身份隔离正确。
2. **健壮**：凭证失效 → `needs_relogin` 明确提示；接口空列表/结构变更 → 显式 fail 而非静默；缓存 stale-on-error 不白屏。
3. **合规**：设备指纹仅用于多账号并发的每账号稳定绑定（§5.10），无随机轮换规避路径；多账号风险提示不可跳过；token 全链路脱敏（日志/事件/UI）。
4. **工程**：所有持久化走 `store::db`；schema 迁移可从 v2 平滑升级；`cargo test` 覆盖 checkin_do 分类逻辑、dig 解析、credits 解析器（内嵌 `#[cfg(test)]` 模式）；前端 invoke 参数命名逐字对齐（`Option<bool>` 教训）。

---

## 十、审查记录

| 日期 | 版本 | 结论 | 备注 |
|---|---|---|---|
| 2026-09-26 | v1.0 | 初稿，待评审 | M0 启动前需确认：F-80 编号登记、R-6 PAT 签到通道优先级 |
| 2026-09-26 | v1.1 | 修订：macOS 由"硬排除"调整为"本期不实施 + 全程跨平台兼容"（需求方确认后续规划实施）；新增 §5.9 平台兼容设计、M5 里程碑 | 新增 R-11（macOS 侦察，M5 前置） |
| 2026-09-27 | v1.2 | 需求变更（需求方确认）：**多账号并发使用纳入范围**，每账号绑定稳定设备指纹（QoderPatcher 模式，§5.10）；原"绝不伪造指纹"红线调整为风险项管理（§七 风险 1、§九 验收 3）；新增 R-12 侦察项与 M3 并发任务；外部项目调研归档 `docs/completed/Qoder外部项目调研归档.md` | 多账号并发核心参考：qoder-creator（指纹）、QoderGateway（凭证/账号池） |
| 2026-09-27 | v1.3 | 完成度审计（对照代码实况逐项标注：✅ 完成 / ◐ 部分 / ⏳ 未动 / 🆕 补充）：M1、M2 全部完成；M3 并发指纹 3/4 子项完成（本地存储覆写待 R-12）；M0 闭合 6 项 + R-12 半闭合，R-1/R-2/R-3/R-8 随 M3/M4 顺延；M4 CDP 兜底关闭（R-5 实证纯 HTTP 可行）；M5 未启动。自外部侦察补 3 项遗漏任务：M1 claim 失败复查恢复、M3 敏感字段出库剥离（已满足）、M4 6h 定时刷新兜底 | §5.10 指纹实现已落地（qoder_device.rs + 三路 ensure 漏斗，cargo test 581 通过） |
| 2026-09-27 | v1.4 | **M3/M4 交付**（cargo test 588 通过 0 失败 + tsc/vite build 通过）：M3——`commands/qoder/ide_store.rs`（IDE 存储发现/导入，R-2/R-8 闭合）、`commands/profile.rs` Qoder 档案（data/profiles_qoder，恢复注入 machine_id，R-12 本机实测闭合）、QoderAccounts 切换/快照 UI；M4——R-3 裁决落地 `qoder_cli_status` 只读桥（无独立 CLI 凭证通道，白名单透传绝不写回）+ 前端 CLI 状态条、`tasks/qoder_refresh.rs` 6h 兜底刷新（LAZY_HOURS=7）+ 调度器 `SchedPlan::EveryHours(6)` 新档位 + run_cli_task 分发 + 日志格式修正；QoderWork 存储/档案因 2027-09 停运延后另议（R-1 ◐） | 本机实测结论：CN IDE 凭证真源 = state.vscdb `secret://aicoding.auth.userInfo`（DPAPI+AES-GCM）；`~/.qoder-cn/.qoder-app-status.json` 仅登录态元数据无凭证 |
| 2026-09-27 | v1.4.1 | **产品同一性澄清（需求方实测确认：Qoder CN 即 QoderWork）**：`%LOCALAPPDATA%\Qoder CN\` 仅 Installer/Launcher 引导器（无用户数据），真实用户数据在 `%APPDATA%\QoderCN\`——R-1 终审闭合，12 项侦察全闭合；历史 QoderWork 独立端表述（TargetApp 变体、R-1 Authfile 分支、L2 MITM QoderWork 组、QoderWork 存储/档案延后项、风险 7）一并作废；代码核实零功能改动（QODER_IDE_ITEMS 覆盖完整、work_exe_candidates 默认路径即 Launcher 形态、TargetApp 仅 Qoder 单变体自洽），仅 3 处注释同步 + 本文档 13 处修订 | 实测路径：`C:\Users\tianw\AppData\Local\Qoder CN\Qoder CN Launcher`；issue #36「QoderWork CN 签到」即 Qoder CN 签到（M1 qoder-checkin 已闭环） |
| 2026-09-27 | v1.4.2 | **M4 收尾（cargo test 598 通过 0 失败 + tsc/vite build 通过）**：①导出导入 `data_io.rs`（aiwork-qoder-pool 格式；uid 幂等合并；指纹红线"仅本地为空才补入"与 WorkBuddy「非空才覆盖」语义相反——Qoder 指纹入池生成一次永不轮换）+ env_reset `env_reset.rs` 8 项语义块（Qoder 无 SSO 注销对应物，无 Keycloak 步骤）+ `QoderAccounts.tsx` 三入口 UI；②「Qoder 入网关池」由需求方立项为独立任务：**不写无端点的假实现**，新增 R-13 侦察项（gateway.qoder.com.cn 对话 API 形态，MITM SOP 固化）与 §5.11 接入技术方案（复用 ApiPool 引擎 / wb_sse / api_keys / Qoder 账号基建；新写 qoder_upstream 等六项） | 修复 data_io.rs merge_account E0500 借用冲突（or_else 二次借用 → 两次顺序查找）；process.rs images_for_app 补 "Qoder CN" 进程画像（Qoder CN.exe / Qoder CN Launcher.exe） |

---

## 附录 A：端点速查表（CN 域，M0 定稿前均为待验证）

| 用途 | 方法 | 端点 | 鉴权 | 备注 |
|---|---|---|---|---|
| 活动列表 | GET | `{openApi}/sash/api/v1/me/campaigns` | Bearer + `Cosy-ClientType` | 缺 ClientType 返回空列表 |
| 活动领取 | POST | `{openApi}/sash/api/v1/me/campaigns/{campaignId}/claim` | 同上 | **幂等**，`replayed=true` 为重复 |
| 用量查询 | GET | `{openApi}/api/v2/quota/usage` | Bearer | 逆向所得 |
| 用户信息 | POST | `{openApi}/api/v1/userinfo` | Bearer | uid/nickname（M1 实装为 POST，见 `qoder_common::fetch_userinfo`） |
| PAT 换 job token | POST | `{openApi}/api/v1/jobToken/exchange` | PAT | `pt-` → `jt-` |
| 设备 token 轮询/刷新 | POST | `{openApi}/api/v1/deviceToken/poll` / `/refresh` | — | body `{"refresh_token":...}` |
| PAT 管理 | — | `https://qoder.com.cn/account/integrations` | — | 官方自助入口 |
| 授权页 | — | `https://qoder.com.cn/device/selectAccounts` | — | device flow |

## 附录 B：参考来源

**官方**：
- Credits 共享规则：help.aliyun.com/zh/lingma/credits
- QoderWork CN 签到条款：help.aliyun.com/zh/lingma/product-overview/daily-check-in-100-credits-reward-program-terms
- 每日登录奖励：docs.qoder.cn/events/100credits
- 产品族与更新日志：help.aliyun.com/zh/lingma/product-overview-qoder/（v1.3.0 统一账号 / VS Code 插件停用）
- CLI 认证与安装：docs.qoder.cn/cli/authentication、docs.qoder.cn/cli/installation
- FAQ（双账号体系 / lingma_ 前缀）：help.aliyun.com/zh/lingma/faq
- QoderWork 停运公告：qoder.com.cn/download

**社区逆向（接口情报）**：
- github.com/sunp-1/qoder-checkin（sash 端点 + auth.v1.dat 解密 + Cosy-ClientType）
- github.com/thintsing/agent-checkin（CN Desktop CDP 实测 + 数据目录 + 风控红线）
- github.com/decolua/9router（双区域端点表 + COSY 常量 + token 分流）
- github.com/GitOfUser/qoderwork_checkin（UI 坐标点击兜底可行性）

**外部参考实现（v1.2 调研归档，详见 `docs/completed/Qoder外部项目调研归档.md`）**：
- github.com/okky-x0f/qoder-creator（QoderPatcher 指纹伪造 + 三平台桌面数据目录映射 —— 多账号并发核心参考）
- github.com/bzym2/QoderGateway（COSY 授权链 / drt-/jrt- token 刷新 / 本地凭据解密 / 账号池稳定绑定 —— 凭证与多账号核心参考）
- github.com/caigee-cmd/cli2api（CN 签到直连实现，与本方案 R-4/R-6 结论互证）
- github.com/mmqz/cpa-multi-plugins（campaigns 契约最全：旧端点已全球禁用、CLAIMABLE 为唯一权威）
- github.com/avaritiachaos/qoder-proxy（本地服务安全清单：DNS rebinding / timing-safe 比较）

**内部参照**：
- `docs/backlog.md`（编号体系 / F-48 Authfile 设备重置预留 / F-55 启动补签 / F-59 stale-on-error / F-66 CLI 隔离 / §四 网关远期再议）
- `src-tauri/src/tasks/wb_checkin.rs`、`tasks/wb_credits.rs`、`tasks/wb_common.rs`（签到/积分/凭证模板）
- `src-tauri/src/switcher/{profile,icube,vscdb,chromium,authfile,machine}.rs`（切换管线）
- `src-tauri/src/device_proxy/{mod,handler,local_capture}.rs`（MITM 捕获）
- `src-tauri/src/workbuddy_cli.rs`（CLI 切号桥模板）
