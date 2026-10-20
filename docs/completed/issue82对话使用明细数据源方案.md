# issue #82 方案分析：对话详细信息获取 & 看板空缺数据源

> 调研日期：2026-10-11
> 关联 issue：https://github.com/smart-open/TraeWorkAssistant/issues/82
> 参照项目：star620/TraeTools（Avalonia/.NET 9）、shuishuipingan/qoder2api-hub（Python 零依赖网关）
> 本地参照文档：`Qoder外部项目调研归档.md`（F-80 外部调研归档）、`开源生态仓库价值分析.md`（28+5 仓库价值分析）

---

## 0. 结论速览

issue #82 的两个诉求（① 对话明细：消耗积分 / token / 概述 / 时间 / 模型；② Qoder Token 统计）在三平台的数据可得性差异极大：

| 平台 | 对话明细（会话级） | 积分按日明细 | Token 按日明细 | 对话概述 | 官方接口支持度 |
|------|------------------|-------------|---------------|---------|--------------|
| **Trae** | ✅ 已有接口（本项目已接但只落日聚合） | ✅ 365 天×模型 | ✅ 365 天（日聚合） | ✅ `user_input_preview` | **最好**——会话级全字段 |
| **Buddy** | ⚠️ 行级计费明细（requestId/model/credit/date），无 token 无概述 | ✅ 31 天（行级聚合） | ❌ 官网行无 token 字段（本地源可补） | ❌ 官网无；本地会话文件可补 | **中等**——计费明细有、token 无 |
| **Qoder** | ❌ 官方无任何按会话/按日明细接口 | ⚠️ 仅快照差分推算 | ❌（网关测量是唯一路径） | ❌ 无 | **最差**——只有余额/逐包聚合 |

**核心判断**：
1. **Trae 对话明细是"零接口风险"功能**——本项目 [usage_history.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/usage_history.rs) 已在生产调用同一接口，只是把会话级数据聚合成日粒度落盘，会话级字段被丢弃。扩展落盘即可，无需任何新的逆向工作。
2. **Buddy 对话明细是"已有数据源换一种切法"**——`get-user-request-usage` 返回行级明细，项目聚合成日合计，行级（requestId 粒度）落盘即得对话明细（缺 token/概述，可由本地会话源补充）。
3. **Qoder 对话明细/Token 统计没有官方路径**——qoder2api-hub 的 token 统计本质是**网关本地测量**（从上游响应/SSE 提取计费字段），不是官方接口。本项目要补 Qoder 数据只有两条路：启用网关 Qoder 池（管道已铺）或继续快照差分。

---

## 1. Trae：对话详细信息获取方案（证据最完整）

### 1.1 接口（TraeTools 逆向 + 本项目生产验证双重确认）

**会话级用量明细**（对应官网 `www.trae.cn/dashboard#usage` 页面）：

```
POST https://api.trae.cn/trae/api/v1/pay/query_user_usage_group_by_session
Authorization: Cloud-IDE-JWT <jwt>
Cookie: X-Cloudide-Session=<session>        （可选）
Origin/Referer: https://www.trae.cn
Content-Type: application/json

{"start_time":<unix秒>, "end_time":<unix秒>, "page_size":20, "page_num":1, "usage_type":[7]}
```

响应结构（TraeTools `UsageModels.cs` 字段注释 + 本项目代理日志实测 2026-09-13 一致）：

```json
{
  "total": 123,
  "user_usage_group_by_sessions": [{
    "session_id": "…",
    "usage_time": 1789009361,
    "model_name": "DeepSeek-V4-Flash 正式版",
    "mode": "Max",
    "use_max_mode": false,
    "credits_float": 1.5,
    "cost_money_float": 0.15,
    "user_input_preview": "帮我写一个…",
    "extra_info": {
      "input_token": 12000,
      "output_token": 3400,
      "cache_read_token": 9000,
      "cache_write_token": 800
    },
    "product_type_list": [2]
  }]
}
```

**字段 → issue 诉求映射**：

| issue 诉求 | 响应字段 | 备注 |
|-----------|---------|------|
| 时间 | `usage_time`（Unix 秒） | 服务端时刻 |
| 模型 | `model_name` + `mode`/`use_max_mode` | 含 Max 模式标记 |
| 消耗积分 | `credits_float`（兜底 `amount_float`） | 项目已实测 |
| 消耗 token | `extra_info.input_token / output_token / cache_read_token / cache_write_token` | 四分量齐全 |
| 对话概述 | `user_input_preview` | **用户首条消息预览**（TraeTools 注释明确） |
| 会话标识 | `session_id` | 会话聚合键 |
| 折合金额 | `cost_money_float` | TraeTools 有用、issue 未明确要求 |

**工程约束**（本项目 [usage_history.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/usage_history.rs) L29-L39、L123-L155 实测固化）：
- 区间按 **30 天分块**（过大区间 400/9004 参数校验拒绝）；
- `page_size=20`（对齐官方控制台）、单块分页上限 50 页防失控；
- 头部必须走 **Web 路由形态**（浏览器 UA + origin/referer + sec-fetch；携带 IDE 指纹头 x-device-id 等反而触发参数校验失败）；
- `usage_type=[7]` = Cloud-IDE 会话积分消耗。

**附加接口——积分包到期**（TraeTools `GetExpiredEntsAsync`）：

```
POST https://api.trae.cn/trae/api/v2/pay/expired_ents
→ {"expired_ent_list": [{"name": "…", "credits_limit": 1000, "expire_time_ms": …}]}
```

含临期与已过期包——可直接支撑本项目 ExpiryTab 的 Trae 到期日历（当前 Trae 到期数据来源若为套餐推断，此接口可给出逐包精确到期）。

### 1.2 本项目现状与差距

- [usage_history.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/usage_history.rs) **已在调用同一接口**，含增量语义（首拉 365 天全量 → 增量从上次拉取日 00:00 重拉替换）、30 天分块、多账号、缓存落盘 `data/usage_history.json`。
- **差距**：解析时只保留 `usage_time/credits_float/model_name/extra_info` 四项并**立即聚合**进 `UsageDayStat`（date → credits/sessions/models/token 合计）。`session_id`、`user_input_preview`、`cost_money_float`、`mode`、`cache_write_token` 全部丢弃。
- 前端 [TokensTab.tsx](file:///d:/code/AIWorkAssistant_main/src/pages/dashboard/TokensTab.tsx) L442 对官网源的空态文案也自证了这一点："官网源无按模型 token 明细（日粒度仅 input/output/cache 汇总）"——数据源本身有按模型粒度，是聚合层抹掉的。

### 1.3 Trae 对话明细方案（低成本高确定性）

1. **后端**：`fetch_chunk` 解析会话元素时增加会话级落盘——新缓存结构 `sessions[]`（session_id / ts / model / mode / credits / cost_money / input / output / cache_read / cache_write / preview），沿用既有增量替换语义（`refetch_from` 之后整段重建，同日多拉不叠加）。预览字段建议截断存储（如 200 字符）控制体积；365 天 × 高频使用者的会话量级在数千条，JSON 落盘可承受。
2. **前端**：积分看板新增「对话明细」面板（Trae 页官网源可用）：表格列出 时间 / 模型（+模式）/ 积分 / token 明细 / 概述预览，支持按日期区间与模型筛选。复用 `BoardPoint` 不合适（它是日粒度），建议独立 `UsageSessionRow` 类型 + 独立查询命令（如 `usage_sessions_query`，参数 start/end/model/分页）。
3. **到期日历增强**（可选）：接 `expired_ents` 逐包到期，替换/兜底 Trae ExpiryTab 推算口径。

风险：接口为逆向所得，服务端可能加风控；但本项目已稳定运行（issue #61 调度器共用），且 TraeTools 同接口互证，失效时已有 stale-on-error 惯例兜底。

---

## 2. Buddy：对话详细信息获取方案

### 2.1 官方行级计费明细（本项目已接）

[workbuddy/credits.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/workbuddy/credits.rs) L640-L716 `usage_official_fetch`：

```
POST {base}/billing/meter/get-user-request-usage     （base 按账号区域路由 ai/cn）
Authorization: Bearer <token>
X-Client-Platform: web

{"startTime":"YYYY-MM-DD 00:00:00", "endTime":"YYYY-MM-DD 23:59:59", "pageNum":1, "pageSize":3000}
```

- **行级明细**（`data.data[]`）：每行含 `requestId`（请求去重键）、`model`、credit（该请求扣积分）、日期时间字段（`usage_row_date` 解析）。
- **窗口限制：近 31 天**；分页 ≤20 页 × 3000 行。
- 本项目现状：拉取行级后**立即聚合**为日合计/模型排行（`wbOfficialAllToPoints`），行级 requestId 粒度丢弃。

### 2.2 Buddy 对话明细方案

1. **计费维度明细（官网源，低成本）**：行级落盘（requestId / 时间 / 模型 / credit），与 Trae 同款「对话明细」面板。粒度是**请求级**而非会话级（一个对话会话含多次请求），issue 的"对话概述"官网行无此字段。
2. **Token 维度（官网无解 → 本地源补）**：官网行级明细**无 token 字段**——这正是 Dashboard.tsx 禁用清单里「Buddy 官网未提供按日 token 明细接口」的根因。Token 只能来自：
   - **本地源**（已实现）：[workbuddy_stats.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/workbuddy_stats.rs) 解析 WorkBuddy 桌面端 `~/.workbuddy/projects`、CodeBuddy CLI `~/.codebuddy/projects`、CodeBuddy IDE 会话索引 `%LOCALAPPDATA%\CodeBuddyExtension\Data\`——会话文件含 usage 四分量（input/output/cache_read/cache_write）与请求记录；
   - **本地会话概述**：WorkBuddy 桌面端会话文件本身有对话内容/请求记录，可提取首条消息作为概述（与 token 统计同源，顺手可得）。
3. **组合口径**：Buddy 对话明细 = 官网行级计费（积分/模型/时间，31 天）∪ 本地会话（token/概述，本地记录覆盖范围）。两源口径不同，按本项目"明示口径不合并"惯例，明细面板应标注行来源。

---

## 3. Qoder：Token 统计与对话明细方案（无官方路径）

### 3.1 官方接口事实（本项目 R-7/R-11 抓包 + qoder2api-hub 互证）

Qoder 官方**不存在**按日/按会话消耗明细或 token 用量接口。已固化的全部用量端点：

| 端点 | 内容 | 缺口 |
|------|------|------|
| `GET {open_api}/sash/api/v2/me/usage`（Bearer + cosy-clienttype:10） | `userQuota/addOnQuota`（total/used/remaining）+ 顶层 `expiresAt` + `dedicated_resource_packages` 逐包 | 只有余额聚合，**无时间序列、无模型、无 token** |
| `GET {WEB_BASE}/api/v2/me/usages/big_model_credits` | 四族配额逐包明细（PLAN/资源包/专属/合并） | 同上；且官网域鉴权 401/openapi 503（2026-10-04 实测，负缓存 30 分钟） |

详见 [tasks/qoder_credits.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/tasks/qoder_credits.rs) L8-L45、L265-L355。

### 3.2 qoder2api-hub 的 Token 统计真相：网关本地测量

其 README 明确：token/积分统计由 **`qoder_proxy.py` 代理在转发时从上游响应/SSE 中提取**（v1.3.4 源码 + 仓库自带测试 `tests/_test_credits_mapping.py` 固化）：

**上游 usage 块实测结构**（`agent_chat_generation` SSE 帧解包后携带）：

```json
{
  "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15,
  "prompt_cache_hit_tokens": 2,
  "completion_tokens_details": { "reasoning_tokens": 3, "cached_tokens": 1 },
  "credits": 97.96,
  "original_credits": 100.5,
  "billable": true
}
```

统计链路（`_extract_usage()` 归一化 → `record_usage()` 三层落账）：

1. **归一化**：`cached_tokens` 三级兜底（顶层 `prompt_cache_hit_tokens` → `completion_tokens_details.cached_tokens` → `prompt_tokens_details.cached_tokens`）；`credit = usage.credits || usage.credit`（**复数优先**，issue #22 命门修复：旧版只读单数 credit 恒为 0 且历史无法回填）；`billable` 布尔只记 JSONL 行、不进聚合字段表（布尔求和无意义）。
2. **三层落账**：内存 `_usage` 全局累计（requests/errors/六字段+`original_credits`/`by_model` 按模型分桶/TTFT·生成·总耗时累计）；每请求一行 `usage/usage.jsonl`（时间/模型/stream/耗时/token/credit/account/key_id/realm + 派生指标 `tokens_per_sec`、`cache_hit_pct`）；`usage-summary.json` 临时文件原子重写防断电丢总账。
3. **消费**：`/usage/by-key`（按 API Key 汇总）、`/usage/timeseries`（时间序列，从 JSONL 行聚合）、dashboard.html Web 看板。

- 这是**测量值**（代理视角从上游计费/usage 字段提取），不是官方统计接口——issue 作者"桌面端不一定有接口"的判断对 Qoder 完全成立。
- 代理改造的技术门槛：COSY 签名体系（RSA-1024 包裹 AES-128 会话密钥 + MD5 五段签名 + 自定义 Base64），与本项目 QoderGateway 调研一致（`Qoder外部项目调研归档.md` §2.3/§2.5）。
- **对本项目的直接可用性**：`credits`/`original_credits`/`billable` 字段名与语义已被该仓库测试固化——本项目 `record_usage_qoder` 扩展积分记账时字段映射可直接照抄，`original_credits` 还天然支持折扣时段对账。
- **峰谷价印证 `original_credits` 对账价值**（v1.3.4 README）：官方模型目录携带 `price_factor_peak`/`price_factor_valley` 峰谷倍率与 `off_peak` 时段窗口（22:00-08:00）——上游 usage 中 `credits`（实扣）与 `original_credits`（原价）的差值即峰谷折扣额。网关记账同时落两字段，可免费还原"峰时/谷时消耗"维度，无需额外逆向。
- **提取时序基线**：Qoder 上游首字延迟实测可达 40–71 秒（xhigh 档 + 2–3 万 token 长上下文），计费/usage 块随流结束帧下发——本项目 credits 提取点若放在 `stream_forward_ex` 返回之后（流结束），时序天然正确；提取逻辑不依赖首字帧。
- **附加情报（签到链路，超出本 issue 范围、备查）**：v1.3.4 的"每日领取 100 Credits"类活动由网关直接调 `POST /sash/api/v1/me/campaigns/{id}/claim` 领取（官方幂等：已领返回 `replayed`，不会重复发放），旧 sash 签到接口仅在仍开放时兜底（能力运行时探测，404 记"本区域无此接口"6 小时后自动重探）。若本项目 Qoder 签到接口日后失效，campaigns claim 是升级路径。

### 3.3 本项目现状与缺口

- **本地源**：Qoder 客户端本地日志 token 字段恒 0（官方模型计费在服务端）→ Dashboard 禁用理由之一，无解。
- **官网源**：仅余额/逐包聚合 → 按日消耗唯一本地口径是**快照差分**（`consumed = 前快照余额 − 当日余额 + 签到奖励`，账号数变动日记 null，无模型粒度）。
- **网关源**：管道已铺但池空——[qoder_route.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/api_server/qoder_route.rs) 已有 `record_usage_qoder` 独立 qoder 桶记账（含 token 提取、请求级日志），[usage.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/api_server/usage.rs) L212 已有 `qoder_days`，前端 `GatewayDays.qoder` 已接；但 `qoder_enabled` 默认关、Qoder 上游账号未入池 → Dashboard L212/L492/L501 静态标"上游未接入"。
- **积分缺口**：网关记账只有 tokens/请求数，**不记积分扣减**（CreditsTab 网关口径 Badge："api_usage 仅记录 tokens，不记录积分扣减…以请求数为主要口径"）。

### 3.4 Qoder 方案（按优先级）

1. **点亮网关 tokens（零新逆向，配置+数据自 accumulating）**：在 API 服务页启用 Qoder 池（`qoder_enabled` + `qoder_enabled_uids`）并将 Qoder 客户端流量指向本地网关。`qoder_days` 即有 token/请求数数据，Dashboard 的 Qoder tokens-网关禁用文案可移除。**注意**：网关 token 口径只覆盖"走网关的流量"，与 Buddy 本地源同为部分口径，需沿用明示口径惯例。
2. **网关积分测量（qoder2api-hub 已验证字段）**：扩展 `record_usage_qoder` 在提取 usage 时顺带记录上游计费字段 `credits`（复数）/`original_credits`——形成"网关测量的积分消耗"新口径。这是 Qoder 积分看板唯一能拿到**模型粒度实时消耗**的路径。建议作为独立数据源或并入 credits-网关源（口径 Badge 需说明"测量值，仅网关流量"）。
3. **对话明细（Qoder 仅有网关请求级日志）**：网关请求级日志已含模型/耗时/token，补记 credits 后即成 Qoder 版"对话明细"（无对话概述——官方与网关都拿不到用户 prompt 原文？qoder2api-hub 未展示 prompt 字段，需实测上游请求体是否含；预期不可得则明细面板 Qoder 列省略概述列）。
4. **快照差分维持现状**：作为不走网关用户的兜底日消耗口径（已是 Best Effort）。
5. **不推荐**：Qoder 客户端本地数据挖掘（token 恒 0，无信息量）。

---

## 4. 看板空缺面板逐一数据获取方案

对照 [Dashboard.tsx](file:///d:/code/AIWorkAssistant_main/src/pages/dashboard/Dashboard.tsx) L487-L505 的 `sourceDisabled` 禁用清单：

| # | 空缺面板 | 根因 | 方案 | 成本 |
|---|---------|------|------|------|
| 1 | Qoder credits-官网 | 官方无按日消耗明细接口（R-7/R-11 实测固化） | 无官方路径；维持快照差分；进阶走 #2 | — |
| 2 | Qoder credits-网关 | 网关只记 tokens 不记积分扣减 | 扩展 `record_usage_qoder` 记录上游 `usage.credits` 计费字段（qoder2api-hub v1.2.19 互证字段存在）→ 网关测量积分口径 | 中 |
| 3 | Qoder tokens-本地 | 客户端本地日志 token 恒 0（服务端计费） | 无解，维持禁用 | — |
| 4 | Qoder tokens-网关 | qoder 池空（`qoder_enabled` 默认关、上游账号未入池） | 启用 Qoder 网关池即自动点亮（`qoder_days` 管道已通），移除禁用文案改动态判断（如 `qoder_enabled && uid 数>0`） | 低 |
| 5 | Qoder tokens-官网 | 官方无 token 用量接口 | 无解，维持禁用 | — |
| 6 | Buddy tokens-官网 | `get-user-request-usage` 行无 token 字段 | 无解（官网接口天花板）；本地/网关源已可用 | — |
| 7 | Trae tokens-本地 | Trae 无本地会话记录 | 维持现状（Trae 官网源 token 数据最全，本地源无必要） | — |
| 8 | **对话明细面板（三平台均无，issue #82 主诉求）** | 现有实现只落日聚合，行/会话级数据在解析时丢弃 | Trae：扩展 usage_history.rs 落会话级（§1.3）；Buddy：行级落盘（§2.2）；Qoder：网关请求级日志 + credits 字段（§3.4-3） | Trae 低 / Buddy 低 / Qoder 中 |

---

## 5. 实施建议排序

1. **Trae 对话明细**（回应 issue #82 主诉求的最直接交付）：接口已生产验证、字段齐全、仅扩展落盘 + 新面板；
2. **Qoder 网关池启用引导**：最低成本点亮 Qoder tokens（用户操作 + 动态禁用判断替代静态文案）；
3. **Buddy 行级明细落盘**：顺手（`usage_official_fetch` 已拉行级）；
4. **网关积分测量**（`record_usage_qoder` 补 credits 字段）：Qoder 积分/明细的长期解，依赖用户走网关；
5. **Trae `expired_ents` 到期日历**：独立小增强，可搭车。

共同注意点：
- 全部新数据源沿用项目惯例：**明示口径、不静默合并**（三源切换 + 口径 Badge + 诚实空态）；
- 逆向接口（Trae/Buddy）失效风险已有 stale-on-error/负缓存/冷却重试基建，新接口直接复用；
- 预览/概述类字段落盘注意截断与凭证红线（不落 JWT、不落敏感原文——首条消息预览建议默认开关可关）。

---

## 6. 代码级改动点明细（本轮深挖固化）

> 以下坐标均以当前工作区代码为准，开发时可直接定位。

### 6.1 Trae 会话级落盘（P0，改动最集中）

**数据丢弃点**：[usage_history.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/usage_history.rs) `fetch_chunk`（L181-L256）的 `for s in &arr` 循环（L212-L246）——解析出 ts/credits/model/extra 三项 token 后**就地聚合**进 `UsageDayStat`，`session_id`/`user_input_preview`/`cost_money_float`/`mode`/`cache_write_token` 从未离开循环体。

**缓存结构扩展点**：`CachedAccount`（L72-L78）现为 `{name, last_fetch_end_ts, daily: BTreeMap<date, UsageDayStat>}`。扩展方案：

```rust
/// 会话级用量明细行（fetch_chunk 采集，随 CachedAccount 落盘）
#[derive(Serialize, serde::Deserialize, Clone, Default)]
pub struct UsageSessionRow {
    pub session_id: String,
    pub ts: i64,              // usage_time（Unix 秒）
    pub model: String,        // model_name（空串归一「未知模型」，对齐现聚合口径 L226-L232）
    pub mode: String,         // "Max" 等；use_max_mode=false 时留空
    pub credits: f64,         // credits_float（兜底 amount_float，对齐 L221-L225）
    pub cost_money: f64,      // cost_money_float（可能缺省）
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub preview: String,      // user_input_preview 截断存储（建议 200 字符）
}

struct CachedAccount {
    name: String,
    last_fetch_end_ts: Option<i64>,
    daily: BTreeMap<String, UsageDayStat>,
    #[serde(default)]
    sessions: BTreeMap<String, Vec<UsageSessionRow>>,  // date → 会话行；serde default 兼容旧缓存
}
```

**增量替换语义直接复用**：`usage_history_fetch_impl` L342-L348 的增量模式（`daily.retain(|d, _| d < refetch_from)` 后整段 insert）对 `sessions` 同样适用——按日期整段重建，同日多拉不叠加。**落盘通道零新增**：`store::docs::usage_history_load/save`（[docs.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/store/docs.rs) L82/L133）序列化整个 Value 树，`sessions` 挂进 `CachedAccount` 即随存随取。

**已知的坑（必做防御）**：
1. `account_summary` L86-L103 的 date 回填防御（旧缓存映射键与字段不一致的回归，已有测试 `aggregated_day_stat_carries_date` L412 固化）——`sessions` 以 date 为映射键，序列化输出时同样要从键回填，不能信任行内字段；
2. `usage_time <= 0` 跳过（L214 已有）——会话行同样只在 ts 有效时采集，否则无日期可归；
3. 体积：365 天 × PAGE_SIZE 20 × MAX_PAGES 50/块（L29-L35）理论上限远超实际，高频使用者会话量级数千条；preview 截断 200 字符后 JSON 增量在 MB 级，可承受。

**新查询命令**（零网络，纯缓存过滤）：

```rust
#[tauri::command(async)]
pub fn usage_sessions_query(
    state: State<AppState>,
    start: String,           // YYYY-MM-DD（含）
    end: String,             // YYYY-MM-DD（含）
    model: Option<String>,   // 模型过滤（精确匹配「未知模型」需原样传）
    page: Option<u32>,
    page_size: Option<u32>,  // 默认 50，上限 200
) -> Result<UsageSessionsPage, String>
```

### 6.2 Qoder 网关积分测量（P1，管道已通只差一个字段）

本轮最关键发现：**上游 usage JSON 已在网关手里，credits 提取只差"从 Value 里再取两个数"**。

- [wb_sse.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/api_server/wb_sse.rs) L178：`let usage = v.get("usage").filter(|u| u.is_object()).cloned()`——上游 usage 块**完整保留**；`stream_forward_ex`（L234）以第 4 元组 `Option<Value>` 返回，且测试 L1184 固化"上游未下发 usage 时不得伪造"；
- [qoder_route.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/api_server/qoder_route.rs) L672-L682：目前只从 usage 取 `prompt_tokens`/`completion_tokens` 两个数喂记账，**`credits`/`original_credits` 就在这一步被丢弃**；
- 记账链路：`record_usage_qoder`（[mod.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/api_server/mod.rs) L241-L271，9 参数）→ `record_in_ttfb`（[usage.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/api_server/usage.rs) L301-L319）→ `DayStats.record`（L150-L186）；
- **`DayStats`（L108-）无积分字段**——需加 `credits: f64` + `original_credits: f64`（均 `#[serde(default)]`，旧 SQLite/JSON 数据自动补 0，与既有四桶 serde default 惯例一致）；
- 视图通道已通：`api_qoder_usage_stats`（[api_server.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/api_server.rs) L1251-L1260）→ `UsageDayView`（usage.rs L429-，同步加两字段）→ 前端 `GatewayDays.qoder`。

**改动清单**（4 处调用点 + 1 个提取函数）：

```rust
/// 上游 usage 块 → 计费字段。字段映射照抄 qoder2api-hub _extract_usage（v1.3.4）：
/// credits 复数优先（issue #22 命门：单数 credit 是旧版字段，读它恒为 0）；
/// max 语义在两字段均非负时与 Python `or` 链等价且更鲁棒
fn extract_billing(usage: Option<&Value>) -> (f64, f64) {
    let Some(u) = usage else { return (0.0, 0.0) };
    let num = |k: &str| u.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    (num("credits").max(num("credit")), num("original_credits"))
}
```

调用点：`qoder_route.rs` L514（失败路径，传 None）、L683（流式主路径）、L984、L1115（非流式/其余路径）——`record_usage_qoder` 扩参 `credits: Option<f64>, original_credits: Option<f64>`，None 不入账（与"上游未下发 usage 不得伪造"测试口径一致）。时序安全：提取点在流结束后（§3.2 时序基线），无需改流处理。

### 6.3 Qoder 网关禁用文案动态化（P1，纯前端）

- 现状：[Dashboard.tsx](file:///d:/code/AIWorkAssistant_main/src/pages/dashboard/Dashboard.tsx) L487-L505 `sourceDisabled` 是**静态字符串对象**，Qoder 网关/官网禁用文案写死；
- 数据已存在：`api_unified_models`（api_server.rs L1314-L1340）运行时已返回 `qoder_enabled` 与 `qoder_ok`（池有无可选账号），前端 unified catalog 已在接收；
- 改法：`sourceDisabled` 改为随 runtime 状态计算（`qoder_enabled && qoder_ok` 时网关源不再禁用），未启用时禁用文案换成操作引导（"在 API 服务页启用 Qoder 池并将客户端指向本地网关"）。注意 tokens/credits 两处 Qoder 网关条目共用同一判断。

### 6.4 Buddy 行级明细落盘（P2）

- 现状：`workbuddy_usage_official`（[credits.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/workbuddy/credits.rs) L545 起，kv 键 `workbuddy_usage_official_cache`，F-59 stale-on-error 已有）在 rust 侧拉行级 → 聚合 → 返回，行级 `requestId` 粒度丢弃；前端 `WbUsageOfficialAll`（[types.ts](file:///d:/code/AIWorkAssistant_main/src/types.ts) L1172-L1188）只有 `daily[]`/`models[]` 聚合字段；
- 改法：行级保留为独立缓存（新类型 `WbUsageRow { request_id, model, credit, ts }`，按账号落盘）+ 独立分页查询命令，**不做全量透传**（31 天窗口 × pageSize 3000 × 20 页上限，高用量行数可达数万）；
- 前端：明细面板 Buddy 列 = 官网行级（计费维度：时间/模型/积分）∪ 本地会话（token/概述，`workbuddy_stats.rs` 同源提取），行内标注来源，沿用"明示口径不合并"。

### 6.5 Trae 逐包到期（P2，独立小任务）

`POST /trae/api/v2/pay/expired_ents`（§1.1）接成独立命令，头部形态复用 `web_usage_post` 同款 Web 路由惯例（浏览器 UA + origin/referer，不带 IDE 指纹头）；[ExpiryTab.tsx](file:///d:/code/AIWorkAssistant_main/src/pages/dashboard/ExpiryTab.tsx) 增加逐包精确到期条目（现有 `credit_packs` 包级口径已在前端就绪，Dashboard.tsx L299-L313 已按包遍历，接入点现成）。

### 6.6 隐私与红线（P0 内嵌 + P3）

- `user_input_preview` 是用户首条消息原文——落盘前截断 + 建议加设置开关（默认开、可关，关闭时存量仅隐藏不删除）；
- JWT 红线沿袭 usage_history.rs L18（只进请求头，不进日志/返回值）——新命令无网络请求，天然满足；
- 网关 credits 记账不新增任何用户内容字段，无红线问题。

---

## 7. 开发任务清单

> 优先级依据：P0 = issue #82 主诉求（最高确定性、零新逆向）；P1 = Qoder 数据从 0 到 1（配置点亮 + 一个字段）；P2 = 顺手增强；P3 = 加固。依赖关系见"前置"列。

### P0：Trae 对话明细（issue #82 主诉求）

| # | 任务 | 主要文件 | 前置 | 验收 |
|---|------|---------|------|------|
| T1.1 | 定义 `UsageSessionRow` + `CachedAccount.sessions` 扩展（serde default） | usage_history.rs | — | 旧缓存文件加载不报错、行为不变 |
| T1.2 | `fetch_chunk` 采集会话行（含 preview 截断 200、mode/cache_write 补齐）；`fetch_account_usage`/增量替换同步处理 sessions | usage_history.rs | T1.1 | 增量重拉后当日会话不叠加；`usage_time<=0` 跳过 |
| T1.3 | 新命令 `usage_sessions_query`（区间/模型/分页，纯缓存零网络）+ 注册 invoke_handler + api.ts 绑定 | usage_history.rs、lib.rs（命令注册）、src/api.ts | T1.2 | CLI/前端可查任意日期区间，跨账号合并 |
| T1.4 | 前端明细面板：`BoardTab` 增 `'sessions'`（Dashboard.tsx L36）+ 新建 `SessionsTab.tsx`（表格：时间/模型+模式/积分/token 四分量/概述；日期区间与模型筛选；Trae 官网源专属，Buddy/Qoder 列按 §2.2/§3.4 渐进点亮） | Dashboard.tsx、SessionsTab.tsx（新）、types.ts | T1.3 | Trae 页新 Tab 展示会话明细，筛选/分页可用，空态诚实 |
| T1.5 | 回归测试：会话行字段映射（含 amount_float 兜底）、增量替换不叠加、date 从映射键回填 | usage_history.rs tests | T1.2 | `cargo test` 通过 |

### P1：Qoder 网关积分测量 + 池启用引导

| # | 任务 | 主要文件 | 前置 | 验收 |
|---|------|---------|------|------|
| T2.1 | `DayStats` + `UsageDayView` 增加 `credits`/`original_credits`（serde default） | usage.rs | — | 旧用量文件/SQLite 读取兼容 |
| T2.2 | `record_usage_qoder` 扩参（`Option<f64>` × 2），None 不入账 | api_server/mod.rs | T2.1 | 失败路径（uid="none"）不产生假积分 |
| T2.3 | `extract_billing` 提取函数（credits 复数优先）+ 4 处调用点接线 | qoder_route.rs（L514/L683/L984/L1115）、wb_sse.rs 或就近 | T2.2 | 单测：复数优先/单数兜底/双缺为 0/usage=None 为 0 |
| T2.4 | 前端 CreditsTab 网关源 Qoder 展示积分（`qoderGatewayToPoints` 适配器 + 模型粒度排行），口径 Badge「网关测量值 · 仅覆盖走网关流量」 | adapters.ts、CreditsTab.tsx | T2.3 | 启用池并走网关后，Qoder 积分页出现模型粒度数据 |
| T3.1 | `sourceDisabled` 动态化：Qoder 网关禁用改为运行时判断（`qoder_enabled && qoder_ok`），未启用时显示操作引导 | Dashboard.tsx（L487-L505） | — | 启用 Qoder 池后 tokens/credits 两处网关源自动点亮 |

### P2：Buddy 行级明细 + Trae 逐包到期

| # | 任务 | 主要文件 | 前置 | 验收 |
|---|------|---------|------|------|
| T4.1 | Buddy 行级落盘：`WbUsageRow` 类型 + 独立缓存 + 分页查询命令（不做全量透传） | workbuddy/credits.rs、store/docs.rs | — | 官网 31 天窗口内请求级记录可查，缓存含 stale-on-error |
| T4.2 | SessionsTab Buddy 列：官网行级（计费）∪ 本地会话（token/概述，`workbuddy_stats.rs` 首条消息提取），行内标注来源 | SessionsTab.tsx、workbuddy_stats.rs | T1.4、T4.1 | Buddy 页明细 Tab 两源分列/标注，不静默合并 |
| T5.1 | Trae `expired_ents` 命令（Web 路由头复用 `web_usage_post` 形态）+ ExpiryTab 逐包条目 | usage_history.rs 或新文件、ExpiryTab.tsx | — | 到期日历出现逐包精确到期（含已过期包） |

### P3：隐私加固

| # | 任务 | 主要文件 | 前置 | 验收 |
|---|------|---------|------|------|
| T6.1 | 会话概述显示开关（设置项，默认开；关闭仅隐藏不删存量） | 设置页、SessionsTab.tsx | T1.4 | 关闭后 UI 不展示 preview，缓存行为不变 |

### 建议执行顺序

```
T1.1 → T1.2 → {T1.3, T1.5} → T1.4        （P0 一条线，后端先行）
T2.1 → T2.2 → T2.3 → T2.4                （P1 积分，可与 P0 前端并行）
T3.1                                       （独立小改，随时可插）
T4.1 → T4.2；T5.1；T6.1                    （P2/P3 按需排期）
```

工作量直觉：T1 全链 ≈ 中型 PR（结构体 + 循环内采集 + 一个查询命令 + 一个 Tab）；T2 全链 ≈ 小型 PR（两字段 + 一个提取函数 + 四处接线）；T3.1 ≈ 几行；T4/T5 独立可拆。
