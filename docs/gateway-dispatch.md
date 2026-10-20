# API 网关资源调配（Dispatch）说明

> 面向「这条请求为什么落在这个账号上」的排障与调参。实现入口：
> `src-tauri/src/api_server/{dispatch,pool,routes,custom_route,qoder_route,wb_route}.rs`。

## 1. 资源池（四类，彼此独立开关）

| 池 | 上游 | 计费口径 | 入口 |
|---|---|---|---|
| `trae` | `llm_utils_chat`（SOLO） | 积分（余额/到期） | `dispatch.rs` |
| `buddy` | WB `copilot /v2/chat/completions` | 积分 + 官方用量 | `wb_route.rs` |
| `qoder` | `agent_chat_generation`（COSY 签名） | 积分包（逐包到期） | `qoder_route.rs` |
| `custom` | 任意 OpenAI 兼容上游（可扩成「上游组」多端点） | 计价策略（`pricing`） | `custom_route.rs` |

## 2. 一条请求的判定顺序

1. **模型命中自定义模型**（`custom_models`，canonical 匹配 + enabled）→ 直达 custom 池；
   配了**上游组**（`upstream_pool`）时按 SWRR 选端点、失败按族退避换端，成功回写健康分。
2. **模型名 → 池**：先查统一目录（`unified_catalog`：trae 官方目录 + buddy 目录 + qoder 双区目录 + 自定义条目），
   再按 `wb_model_route` 的系列/别名规则映射（含 `-thinking`、`flash`、后台任务、长上下文降档）；
   Qoder 条目按账号区域解析（CN = `gateway.qoder.com.cn`，INTL = `api3.qoder.sh`）。
3. **池内选号**（`pool.rs`）：过滤 → 排序 → 粘性 → 试跑 → 失败轮换。
4. **协议转换**：OpenAI / Anthropic / Responses 三形态（`sse.rs`、`wb_sse.rs`），流式与非流式同源。

## 3. 选号：过滤（硬条件）

按顺序剔除：

1. `disabled` / 凭证为空 / `needs_relogin`；
2. **冷却中**：软冷却（错误后 30s 起）、积分耗尽硬冷却（至次日 04:00）、
   上游组端点熔断（soft 600s→7200s / breaker 1800s→21600s / degrade 600s→7200s / 401 直压 6h / 402 停放至次日 UTC+8 10:00）；
3. **模型级冷却**（`model_cooldowns`：10s→20s→40s 渐进）；
4. **积分/配额门槛**：余额 ≤ 保留阈值、积分包已过期、免费档（倍率 0）豁免积分维度；
5. **Key 约束**（`api_keys`）：`bind_pool` 优先覆盖、`allowed_accounts` 白名单、
   `allowed_groups`/`allowed_tags` 实时展开（交集）、`schedule_mode=dedicated` 只打指定账号、
   `daily_limit` 当日配额（跨天自动重置）。

## 4. 选号：排序（软策略）

- **smart（默认）**：按「临期/低余额优先消耗」——先到期先花、余额少的先用，
  避免积分过期浪费；同档位再看模型倍率（低倍率先行）。
- **priority**：按账号列表顺序取首个健康账号（人工指定主力号）。
- 每池独立设置（trae/buddy/qoder 各自开关与参数），也可用 `dispatch_presets` 一键套用预设。

## 5. 会话粘性 / 对冲 / 轮换

| 机制 | 行为 |
|---|---|
| 粘性（`wb_sticky`、`pool_sticky`） | 同会话（按首条 user 内容指纹）在 TTL 内固定同池同账号，复用上游 KV cache；每池独立 TTL（默认 300s） |
| 慢请求对冲 | 首字节超过阈值（默认 8s，`*_hedge_threshold_ms`）且池内有其他健康账号时，向第二账号并发发一份，先到先用 |
| 失败轮换 | 401/403 → 换号并刷新凭证；429 → 软冷却 + 换号；5xx/空完成 → 换号重试（最多 2 轮，30s/90s） |
| 跨池回退 | 目标池全不可用时按 `fallback_*` 尝试其他池（可开关），回退原因记账 |
| 后台/长上下文降档 | 标题/摘要类短请求、输入 ≥100k token 的请求自动换低倍率 flash 档 |

## 6. 记账与可观测

- 每请求写 `api_usage`（按池/模型/Key/日）+ `api_logger`（含 Key 名与上游账号名）；
- custom 池额外记 **成本**（`pricing`：内容寻址策略 + 分档单价 → 按日×模型聚合，未定价模型显式列示）；
- 池健康、Key 用量、模型倍率在「资源调度 / 枢纽增强」页可见；
- 所有冷却/熔断状态**只存内存**（重启不复活），组配置与策略落 SQLite。

## 7. 常见「为什么打不到某个账号」

| 现象 | 排查点 |
|---|---|
| 新账号一直不被选中 | 是否 `disabled` / 凭证为空 / 积分低于保留阈值 / Key 白名单未包含它 |
| 只在某个 Key 上失败 | 该 Key 的 `bind_pool`、`allowed_*`、`daily_limit` |
| 请求固定落同一个号 | 会话粘性 TTL 内（属预期）；或 policy=priority |
| 一会儿换号一会儿不换 | 粘性到期 / 对冲触发 / 失败轮换；看 `api.log` 的 `route=` 与 `fallback` 记录 |
| 国际版 Qoder 模型 404 | 账号 `realm` 与模型所属区域不匹配（当前 CN/INTL 双区目录已支持，端点按账号区域分流） |
