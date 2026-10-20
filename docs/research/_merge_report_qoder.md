# Qoder2API-Hub 能力清单（Tauri/Rust 桌面端合并参考）

只读分析对象：`E:\study\gitproject\aiwork\qoder2api-hub`（v1.4.3）。形态：Python 3.9+ 纯标准库单进程（`ThreadingHTTPServer`，默认 `127.0.0.1:8790`），把 `qoder.com.cn`(cn) / `qoder.com`(intl) 的原生服务翻译成 OpenAI Chat Completions + Responses + Anthropic Messages。零 pip 依赖（AES-128/256、RSA、GCM、DPAPI、QMC 全为内置纯实现），状态全部落在 `accounts/` 与 `usage/` 文件里，无数据库。

## 一、模块 → 职责表

| 文件 | 行数 | 职责 | 对外接口 |
|---|---|---|---|
| `qoder_proxy.py` | 7973 | 主网关：`Handler` 路由、COSY 数据面、三协议转换、工具调用回读守卫、用量记账、看板鉴权、realm 路由 | 全部 HTTP 路由（见 §2） |
| `qoder_accounts.py` | 3786 | 双区常量 `REALM_CONFIGS`、`Account`/`AccountPool`、`SessionAffinity`、`CreditsRefresher`、OAuth 设备流、PAT、刷新、护栏、活动领取、本机凭证扫描、导入导出 | 被 proxy 调用；模块级常量即协议真源 |
| `qoder_tasks.py` | 967 | 签到闭环 `run_checkin`、`run_pro_claim`、批量 `run_batch_checkin`、`run_batch_pro_claim`、`run_keepalive`、任务行合成 `fetch_tasks_view`、账本健全性 `ledger_is_unknown` | 由 proxy `/tasks*` 调用 |
| `qoder_scheduler.py` | 301 | `Scheduler`：整点排程、启动补签闸门、状态落盘、日志回流看板 | proxy `/scheduler*` |
| `qoder_settings.py` | 488 | 面板密码 PBKDF2、`PanelSessions`、多 API Key 出口绑定、模型闸门、`limits` 五种守卫阈值 | `settings.json` 唯一读写层 |
| `qoder_catalog.py` | 1900 | 双区官方模型快照（外部 JSON 优先 / 内嵌冻结副本兜底）、`MODEL_ALIASES`、`resolve_upstream_key`、独占表、官方文案 | `models_for_realm` / `display_id` / `auto_realm_for_model` |
| `qoder_sign.py` | 660 | 自定义 Base64 `qoder_encode`、纯库 AES-128/256+CBC/GCM、RSA PKCS#1v15、`dpapi_unprotect`、`qmc_decrypt`、`CosySession`/`SESSIONS` | `qoder_encode`、`SESSIONS.get(account).headers()` |
| `qoder_fingerprint.py` | 256 | `derive_id` 稳定设备指纹、`derive_machine_token/type`、`generate_request_id`、`vm_status` 虚拟化体检、`get_desktop_fingerprint`（埋点用，当前未接线） | 被 accounts/sign 调用 |
| `qoder_anthropic.py` | 598 | Anthropic Messages ⇄ chat 纯函数转换（零 import 本仓模块） | `messages_to_chat` / `chat_to_messages` / `stream_anthropic_events` |
| `qoder_agents.py` | 968 | 5 种客户端（Claude Code/Codex/OpenCode/DSH/Crush）文本级增量改写、备份、byte-exact 还原、独立 CLI | proxy `/agents*`；`python qoder_agents.py` |
| `dashboard.html` | 4848 | 单文件看板：5 页签 + 4 个子页锚点、主题、面板登录门 | `GET /` |
| `baseprompt.json` | 1（59KB） | 官方推理请求体骨架（`build_qoder_body` 深拷贝基底） | 文件 |
| `qoder_catalog_intl.json` / `_cn.json` | 835 / 607 | 双区官方模型快照（`_refresh_catalog.py` 从本机客户端 `catalog-v6` 导出） | 文件 |
| `_diag_gateway.py` / `_diag_campaign.py` / `_verify_models.py` / `_refresh_catalog.py` / `_install_umid.py` | — | 链路体检 / 活动体检 / 模型库对账（307–347 断言）/ 快照刷新 / 从官方 npm 包提取 UMID 原生组件 | CLI |
| `tests/`（29 套件 + `run_all.py`） | — | 编排器并发跑 Python+前端 JS 套件；`_test_qoder.py` 承载 689 条遗留断言 | CLI |

## 二、核心能力清单

移植价值：**A** = 直接 1:1 移植；**B** = 逻辑可移植但宿主需重写；**C** = 桌面端不需要 / 应换实现。

| 能力 | 实现要点 | 对外 HTTP | 值 |
|---|---|---|---|
| Chat Completions 主链路 | `build_qoder_body`（baseprompt 深拷贝 + `flatten_messages` 压平 + `model_config` + `business` 卡片 + `parameters.max_tokens/reasoning_effort`）→ `qoder_encode` → `CosySession.headers` → `POST CHAT_PATH` → `iter_inner_sse` 拆 `{"headers","body","statusCodeValue"}` 信封 → `aggregate_stream` 还原 chunk | `POST /v1/chat/completions`、`/chat/completions` | A |
| Responses API 双向 | `responses_to_chat` / `chat_to_response` / `stream_responses_events`；custom freeform（`apply_patch`）降级为 function 出站、入站还原 `custom_tool_call`（`_downgrade_custom_tool`、`CUSTOM_TOOL_HINT`） | `POST /v1/responses`、`/responses` | A |
| Anthropic Messages 桥 | `qoder_anthropic.messages_to_chat`；`budget_tokens` → 档位 `BUDGET_STEPS`；`x-api-key` 头回退；**不伪造 thinking signature**（`QD_ANTHROPIC_THINKING=empty` 仅实验） | `POST /v1/messages`、`/v1/messages/count_tokens`（本地 CJK 估算） | A |
| 流式帧层 + 心跳 | `_sse_begin/_sse_write/_sse_end` 用 `Transfer-Encoding: chunked` 且不发 `Connection: close`；`sse_with_heartbeat` 每 5s 补 `: ping`（`QD_SSE_HEARTBEAT`，0=关），等待上限 900s | 所有 `stream:true` 响应 | A |
| 瞬时故障双层重试 | 连接层 `TRANSIENT_MAX_RETRIES=2`（1s/2s 退避，且 intl 依次切 `api1→api2→api3`）；流内信封层 `should_retry_envelope`/`aggregate_with_envelope_retry`（仅在未向客户端写出字节前重开） | 内部 | A |
| 失败治理三族 | `note_soft_rate`(429 指数退避) / `note_failure`(硬错误熔断) / `note_unknown_failure`(降权窗口)；`note_success` 统一复位 | 内部 | A |
| 账号池轮询 | `AccountPool.pick` = 临期优先 `_weighted_pick`（nginx smooth WRR）→ `_rotate_pick`（纯轮转 + 两轮降权扫描） | 内部 | A |
| 会话亲和 | `SessionAffinity`（TTL 7200s，5000 条）+ `derive_affinity_key`/`prompt_fingerprint`；`QD_AFFINITY_*` | 内部 | A |
| 配额护栏 G1–G4 | `reserve_blocked` / `daily_limit_blocked` / `credit_limit_blocked`(free 豁免) / `model_token_limit_blocked`；`quota_blocked` 汇总 | 看板展示 + 出站前拦截 | A |
| 402 余额停放 | `note_balance_cooled` 停到「UTC+8 次日 10:00」；仅 `revive_balance_cooldown`（看到 `remain>0`）解封 | 内部 | A |
| Token 生命周期 | `refresh()` 按前缀路由：`drt-`→`/api/v1/deviceToken/refresh`，`jrt-`→`/api/v1/jobToken/refresh`，失败回落 `pt-` 重新 `jobToken/exchange`；`token_family()` | 内部 | A |
| OAuth 设备流 | `start_login` 生成 PKCE-S256 + `nonce`（cn 带横线 uuid / intl 32-hex）+ 设备授权 URL `{website}/device/selectAccounts`；`poll_login` 轮询（404/202=pending），`LOGIN_TTL_SECONDS=600` | `POST /accounts/login/start`、`GET /accounts/login/poll`、`POST /accounts/login/cancel` | A |
| PAT 导入 | `import_pat`：`pt-` → `jobToken/exchange` → 拉 `/api/v1/userinfo` 补 uid/nickname/user_type | `POST /accounts/import/pat` | A |
| 本机凭证扫描 | 桌面 `auth.v1.dat`（Chromium `v10`：`Local State` os_crypt key → `dpapi_unprotect` → `chromium_decrypt_v10` AES-256-GCM）+ CLI `~/.qoder*/.auth/user*`（`aes_cbc_decrypt`，key=`machine_id[:16]`），两步确认导入 | `POST /accounts/import/desktop`（`{}`=扫描，`{path,realm}`=导入） | B |
| 每日签到 / 活动领取 | `campaign_checkin`：`GET /sash/api/v1/me/campaigns` → 对 `claim_status=CLAIMABLE` 且 `action_type∈{"",CLAIM_BENEFIT}` 的 `POST …/{id}/claim`（幂等 `replayed`）→ 写 `campaignCodes` | `POST /accounts/checkin`、`POST /tasks/run`、`GET /tasks` | A |
| Pro 福利包 | `pro_eligibility` → `pro_claim`（`PATH_PRO_ELIGIBILITY/PATH_PRO_CLAIM`），一次性 `PRO_REWARD_CREDIT=1800` | `POST /tasks/travel` | A |
| 后台调度 | `Scheduler`：09:00/21:00 签到、22:00 全量保活、其余巡检只刷剩余寿命 <4h 的号；`mark-before-act` 启动补签闸门；日志即状态 | `GET /scheduler`、`POST /scheduler/trigger`、`POST /scheduler/toggle` | A |
| 模型目录同步 | 三源：动态 `GET /algo/api/v2/model/list?Encode=1`（COSY 签名、**GET 必须带 `qoder_encode(b"{}")` 同款 body**，否则 403）> 本机 `~/.qoder[.cn]/.models/<uid>/catalog-v6`（`qmc_decrypt`）> 双区 JSON；`fetch_models` 300s 缓存 | `GET /v1/models`（`?merge_realms=1` 并集） | A |
| 模型入口统一 | `resolve_upstream_key`（key / `「key (Name)」` / 别名 / 官方显示名 / 本地化名）+ `model_entry` 输出 60+ 字段（capabilities/limits/`reasoning_efforts`/峰谷价/`disabled_reason`） | 同上 | A |
| 工具调用历史双形态 | `structured_tool_history_enabled`（auto 仅 cn + 白名单 `qwen/glm/gm4/gm5` + id 齐备）→ `flatten_messages(structured=)`；文本化时写 `LEAK_MARKER`+JSON；`recover_leaked_tool_calls` 回读还原（`_HOLD_MAX_CHARS=32768`、`_HOLD_MAX_FRAMES=200`） | 内部 | A |
| 用量记账/分析 | 每请求一行 `usage.jsonl` + `usage-summary.json`；`perf_stats`（TTFT/gen/wall/tokens_per_sec/cache_hit 分位）、`compute_usage_analytics`、`usage_timeseries`、`usage_by_account`、`usage_by_key` | `GET /usage`、`/usage/recent`、`/usage/perf`、`/usage/analytics`、`/usage/by-account`、`/usage/by-key`、`/usage/timeseries` | A |
| 积分汇总（只读） | `credits_summary()` 只读本地登记值，**绝不触发上游**，给下游轮询 | `GET /credits/summary`（key 级鉴权） | A |
| 面板鉴权 | PBKDF2-SHA256(`PBKDF2_ROUNDS=120000`) 密码 + 内存 `PanelSessions`（`SESSION_TTL=7d`，`X-Panel-Token` 头）；API Key 由 `_key_ok` 同时认面板会话 | `POST /panel/login`、`/panel/logout`、`/panel/password`、`GET /panel/status` | C |
| 多 Key 出口绑定 + 模型闸门 | `api_keys[]` 每条 `realm`/`models`；`_model_precheck` 顺序：跨区错配 → `banned_models` → key 白名单 | `GET/POST /settings`、`/settings/save`、`/settings/reveal` | B |
| 智能体一键配置 | `qoder_agents`：文本级 YAML/TOML/JSON 增量改写、`BACKUP_KEEP=10`、`MAX_EDIT_BYTES=8MB`、只存 sha256 指纹 | `GET /agents`、`/agents/models`、`/agents/cli`（公开）、`POST /agents/apply`、`/agents/restore`（loopback） | B |
| 探活/诊断 | `/ping` 等纯文本 `pong`（不需任何凭证）；`/health` 带账号池；`/diag/vm` 虚拟化体检；`/update/check` GitHub Releases 比对 | `GET /health`、`/ping,/healthz,/livez,/readyz`、`/diag/vm`、`/update/check` | B |
| 出站安全 | `validate_public_http_url`（拒 loopback/私网，`_FAKEIP_NETS` 放过 Clash fake-IP）；`_opener_for_request` 代理**只读环境变量、每请求现读**（绕开 Windows 注册表代理固化） | 内部 | A |

## 三、数据模型

**`accounts/<uid>.json`**（一账号一文件，明文令牌，`Account.to_dict()` 即全量落盘字段）
`uid`、`nickname`、`domain`、`realm`(cn/intl)、`platform`(CLI)、`accessToken`、`refreshToken`、`personalToken`、`expiresAt`(epoch 秒)、`addedAt`、`source`(oauth/desktop-app/pat/…)、`enabled`、`lastError`、`cooldownUntil`、`plan`、`lastCheckin`(`YYYY-MM-DD HH:MM:SS`)、`userType`(默认 `personal_professional_trial`)、`organizationId`/`organizationName`、`lastQuotaAnomaly`(最近一次不可信 quota 原始摘要)、`campaignCodes{campaignId→兑换码}`、`campaignBlockedUntil{campaignId→epoch}`；
`credits` = `{remain, used, size, exceeded, usage_pct, expires_at, packages[{name,remain,used,size,(expires_at|days_left|no_expiry|package_code)}], updated_at, updated_iso}`（两个包固定为「基础额度」`userQuota` 与「赠送/签到额度」`addOnQuota`）。
**仅运行期、不落盘**：`model_cooldowns{model→epoch}`、`reserve_credits`、`daily_token_limit`、`daily_tokens_today`、`daily_credit_limit`、`daily_credits_today`、`model_daily_token_limit`、`model_daily_tokens{model→tokens}`、`free_models`、`expiring_window_days`、`balance_until`/`balance_reason`、`soft_streak`、`fails`、`degrade_count`、`breaker_until`、`degrade_until`、`_checkin_cap`/`_checkin_cap_at`、`campaign_status`、`machine_identity_source`、`machine_headers_state`。

**`accounts/scheduler/state.json`**：`enabled`、`last_run_time`、`next_run_time`、`startup_claim_date`(当日补签闸门日期)、`last_cycle_date`、`logs[≤60]`、`updated_at`。放在子目录是为了不被 `AccountPool.load()` 顶层 `*.json` 扫描误读。

**`accounts/settings.json`**：`panel_password_hash`/`panel_password_salt`/`panel_password_rounds`/`panel_password_default`、`api_keys[{id,name,key,realm(""|intl|cn),enabled,created_at,models[fnmatch]}]`、legacy `api_key`/`api_key_set`、`launcher_key`(LAN 模式 `qd-` 前缀)、`auth_disabled`、`banned_models[]`、`limits{reserve_credits|daily_token_limit|daily_credit_limit|model_daily_token_limit|expiring_window_days 各 {global,intl,cn}}`（`null`=继承 global，显式 `0`=关闭）。

**`accounts/machine_identity.json`**：`version:1` + `realm.{intl,cn}.{machineToken,machineType,machineCode,vm,vm_info{isVm,brand,percentage,vmTypeCode},source("runtime-info"),cached_at}`。

**`usage/usage.jsonl`**（每请求/每错误一行）：`at`、`iso`、`model`、`stream`、`elapsed_ms`、`ttft_ms`、`gen_ms`、`prompt_tokens`、`completion_tokens`、`reasoning_tokens`、`cached_tokens`、`total_tokens`、`credit`、`original_credits`、`billable`、`account`、`realm`、`realm_reason`、`reasoning_effort`、`reasoning_effort_requested`、`key_id`、`tokens_per_sec`、`cache_hit_pct`、`fp*`（`prompt_fingerprint`）、错误行 `error:true`/`status`/`message[:400]`。
**`usage/usage-summary.json`**：`requests`、`errors`、`USAGE_FIELDS` 六项累加、`by_model{model→{requests,…}}`、`ttft_ms_sum/samples`、`gen_ms_sum/samples`、`wall_ms_sum/samples`、`started`。

## 四、关键算法细节

**1. 冷却 / 配额守卫（`Account.ready()` 短路顺序，纯内存无网络）**
`enabled`且`access_token` → `cooldown_until>now` → `model_cooldowns[model]>now` → `balance_until>now`(402 停放) → `breaker_until>now`(熔断) → `reserve_blocked()` → `daily_limit_blocked()` → `credit_limit_blocked(model)` → `model_token_limit_blocked(model)` → 最后才判 `expiresAt`（剩余 `>240s` 直接用；`>0` 先 refresh 再用；`<=0` 只 refresh）。
退避公式：软限流 `soft_backoff(n)=min(600*2^(n-1),7200)`；熔断 `breaker_backoff(n)=min(1800*2^(n-3),21600)`，阈值 `BREAKER_THRESHOLD=3`；降权 `degrade_backoff(n)=min(600*2^(n-5),7200)`，阈值 `DEGRADE_THRESHOLD=5`。`note_error(msg, cooldown=60, single_account)` 在单账号池把冷却压到 **3s** 并返回真值。`open_upstream` 遇到池内无号时先 `_short_error_cooldown_wait`：`cooldown_until` 与「预算内（`ERROR_COOLDOWN_WAIT_MAX=10s`）的 `model_cooldowns`」一起收集取最小值，若 >0 则 `sleep(wait+0.25)` 后重新选号——**等它续上**而不是报 429（429 只在真频控 >10s 时出现）。所有惩罚状态写走 `_QUOTA_LOCK`，落盘走 `_SAVE_LOCK`。

**2. realm（CN/INTL）作用域**
常量真源 `REALM_CONFIGS`：cn = `openapi.qoder.com.cn` / `gateway.qoder.com.cn` / `client_id 1c5e33e1-…` / `redirect qoder-work-cn://` / `ua QoderWork/1.1.64` / `home .qoder-cn` / `app_dir com.qodercn.app.stable`；intl = `openapi.qoder.sh` / `api1.qoder.sh`(备用 api2/api3) / `client_id e883ade2-…` / `redirect qoder://aicoding.aicoding-agent/login-success` / `ua Qoder/1.1.64` / `home .qoder` / `app_dir com.qoder.app.stable`。
请求定区优先级：`_request_realm` = `?realm=` 显式 > 该 API Key 绑定的 `realm` > `CURRENT_REALM`(持久化在 `settings.json`)。`fallback_realm_by_accounts` 只在未显式指定时生效，且**模型归属优先于账号可用性**。响应头 `X-Qoder-Realm` / `X-Qoder-Realm-Reason`(`model_missing`/`no_account`) 回传实际出口；`realm_scope("all"|"")→None` 供看板不过滤。

**3. 积分与倍率映射**
`fetch_credits()`：`remain = userQuota.remaining + addOnQuota.remaining`、`used = 两者 used 之和`、`size = 两者 total 之和`、`exceeded = isQuotaExceeded`、`usage_pct = totalUsagePercentage`、`expires_at = normalize_epoch(q.expiresAt)`；**退化守卫**：`userQuota`与`addOnQuota`双缺 → 或 `exceeded=true 且 remain=0 且 size=0` → 记 `lastQuotaAnomaly` 且**不覆盖**旧快照（真耗尽的特征是 `size>0 且 remain=0`，会放行）。
倍率：`price_factor` 原样透传当前值；`price_factor_peak = promotion.before_promotion_price_factor`（退化 `original_price_factor`）；**`price_factor_valley = round(peak × promotion.discount_factor, 4)`**（缺折扣才回落当前 `price_factor`）——不按抓取时刻猜。`promotion` 分支**优先于** `is_free`，所以 `is_free=true` 但价 0.20x 的 Qwen3.8-Max 不会被标成免费。
低谷判定 `off_peak_active_now(start,end,tz)`：`HH:MM→分钟`，固定按 `UTC+8`（`tz="UTC±N"` 可覆盖），`s<e` 区间判断、`s>e` 跨午夜 `cur>=s or cur<e`，`s==e` 恒真；看板前端另按本地时间复算做双保险。

**4. 过期时间计算**
`normalize_epoch(v)`：`>1e11` 视为毫秒先除 1000，异常返回 0。`_device_expiry(data)`：`expires_in`(**毫秒**，`+ /1000`) → `expires_at`(RFC3339 前 19 字符 strptime) → 默认 `now+30d`。凭证族寿命：`dt-`≈30 天 / `jt-` 24h、`drt-`≈1 年 / `jrt-` 48h、`pt-` 长期兜底。`ready()` 的 240s 阈值、`run_keepalive(threshold_seconds=4*3600)` 的 4 小时预刷。`_EXPIRY_NEVER_EPOCH = 253402214400`（CN 号「永不过期」占位，`soonest_expiring_days` 会跳过它，且不映射到包级——避免把按月重置算成临期）。`next_checkin_window()` 固定 `CHECKIN_WINDOW_HOUR_UTC8=10`：`now8 < 今日10:00` 取今日，否则取明日，文案 `"MM-DD HH:MM（UTC+8）"`。

**5. 请求签名（COSY）**
`qoder_encode(plain)`：先标准 base64 得 `std`，`a=len//3`，三段轮转为 `std[n-a:] + std[a:n-a] + std[:a]`，再按 `QODER_CUSTOM_ALPHABET`(`_doRTgHZBKcGVjlvpC,@aFSx#DPuNJme&i*MzLOEn)sUrthbf%Y^w.(kIQyXqWA!`) 逐字符映射，`=` → `$`；`qoder_decode` 是其逆（`R3+R2+R1`）。
会话：`temp_key = uuid4().hex[:16]`（16 个 ASCII 字节，名义 AES-128、有效熵 ~64bit，**不可单方面改**，服务端用同一串字节解）；`cosy_key = b64(RSA_PKCS1v15_encrypt(temp_key))`（服务端 1024 位公钥 `SERVER_PUB_PEM`）；`info = b64(AES-CBC(identity_json_sorted_compact, key=iv=temp_key))`，identity 为 9 键 `{name,aid,uid,yx_uid,organization_id,organization_name,user_type,security_oauth_token,refresh_token}`。
`bearer(body, raw_url)`：`path` 去掉 `/algo` 前缀；`payload = b64(sorted_compact{cosyVersion:"1.1.64", ideVersion:"", info, requestId:uuid4, version:"v1"})`；`date = unix 秒`；**`sig = md5(payload + "\n" + cosyKey + "\n" + date + "\n" + body + "\n" + path)`**；`Authorization = "Bearer COSY.{payload}.{sig}"`。
固定头：`cosy-data-policy: AGREE`、`cosy-clienttype: 5`（桌面端活动平台是 `10`）、`cosy-date`、`cosy-user`、`cosy-key`、`cosy-clientip: 169.254.198.161`、`cosy-version`、`cosy-machineid/machinetoken/machinetype`、`login-version: v2`、`user-agent: Go-http-client/2.0`，可选 `x-model-key`/`x-model-source: system`。会话缓存在 `SESSIONS`（key=uid，token 变化即重建）。

**6. 设备指纹生成**
`derive_id(uid, salt) = md5(f"{salt}:{uid or 'anonymous'}").hexdigest()[:36]`（恒为 32 字符，切片是防御性恒等）。盐表：`machine`(machineId/均用于 `cosy-machineid`、`X-Machine-ID`)、`session`、`req`(请求 ID 前缀)、`machinetype`(取前 18 且去横线)、`machinecode`。`derive_machine_token(uid) = urlsafe_b64(sha512(f"machinetoken:{uid}"))[:43]`。`generate_request_id = derive_id(uid,"req") + "-" + (time_ns % 1e6 补 6 位)`。
真机器身份（活动链路专用）：`native_machine_identity` 调官方 `runtime-info.exe prod --account-stdin`（stdin `{"account": uid}`），内存 TTL `NATIVE_IDENTITY_TTL=1800` + 落盘 `machine_identity.json`；首次无缓存做 `IDENTITY_VOTE_ROUNDS=3` 多数表决（分歧补 `IDENTITY_VOTE_EXTEND_ROUNDS=2`）；换身份只能「清种子 `$HOME/.config/.locale_cfg` + 重调」。身份是**机器级**的（`runtime-info` 对所有账号返回同一份），因此按 realm 缓存即正确。

**7. 模型目录同步**
优先级：动态 `/algo/api/v2/model/list` > 本机 `catalog-v6`（QMC v1 = HKDF-SHA256(uid)+AES-256-GCM，`qmc_decrypt(blob, uid)`）> 双区 JSON。`merge_catalog(primary, realm)`：静态快照打底补元数据，**动态条目只覆盖非 `None` 字段**（上游对 `context_config`/`thinking_config`/`description` 返回 null，不能抹掉真值），**清单集合以 primary 为准**（动态 15 条就不外塞静态独有项），顺序按 `CN_UI_ORDER`/`INTL_UI_ORDER`。独占表 `INTL_EXCLUSIVE={efficient,cmodel,smodel,ultimate,performance}`、`CN_EXCLUSIVE={q37fmodel,gm51model}`（含前缀版）。`auto_realm_for_model` 只在「该模型仅一区存在」时换区，双区同名（如 `mmodel`）不猜。`reasoning_effort` 归一化 `normalize_reasoning_effort(effort, meta)`：顺序 `low<medium<high<xhigh<max`，命中原样透传，未命中取「最近合法档」（同距偏向该模型 `is_default` 档：`dfmodel` 的 `medium→high`、`xhigh→max`；`qfmodel` 的 `high→medium`、`max→xhigh`）；有 `thinking_config` 无档位表则只留 `none`；完全无 `thinking_config`（如 `auto`）原样透传。

**8. 活动 / 埋点**
`campaigns()` 必须**同时**满足两层才拿得到活动：① 桌面端头 `User-Agent: Qoder` + `cosy-clienttype: 10` + `cosy-version: 0.4.3`（缺则静默返回空列表）；② 真实机器身份——`derived` 分支**一个 `cosy-machine*` 头都不发**（issue #10 实测：全套派生六头会被判非官方客户端，把 `CLAIMABLE` 的「每日领取 100 Credits」整条过滤；六头任一个单独出现反而可见）。列表缓存 `CAMPAIGNS_TTL=20s`，旧 sash 接口能力探测 `CHECKIN_PROBE_TTL=6h`（404/405/410 记「本区域无此接口」）。
领取状态机（`claim_campaign` 返回 + `_CAMPAIGN_FAILURE_CN`）：`CLAIMED`→`already`（**归类先于 `action_type` 过滤**，官方领完会把 `actionType` 降级为 `VIEW_DETAILS`）；`ok+replayed`→`already`；`ok`→`claimed`(+`earned`、`redemption_code`→`campaignCodes`)；`SAME_PERSON_ALREADY_CLAIMED`→`blocked` 且写 `campaignBlockedUntil[cid]=now+6h`（按「人」去重，多账号共用一张券）；`REDEMPTION_CODE_OUT_OF_STOCK`→`pending`（次日 10:00 重试）；`ACHIEVEMENT_NOT_COMPLETED`→`locked`。相邻 claim 间隔 `CLAIM_GAP_DEFAULT=1.0s`（`gap=None` 才用默认，`0` 合法）。钻埋点侧：`get_desktop_fingerprint(uid,…)` 给出官方事件字段（`timezone:"Asia/Shanghai"`、`product:"SaaS"`、`ideName/ideType:"Qoder"`、`ideVersion/extVersion:"1.1.64"`、`releaseDate`、`commit`、`cpuCores`、`memorySize`、`reportDelay:2000`、`machineId/sessionId`），**但当前全仓无调用点**——是预留，不是已接线链路。

## 五、桌面端（Tauri/Rust）可复用点与不可移植点

**直接可搬（算法/常量/状态机）**
- `qoder_sign` 全链：自定义 B64 字母表、`RSA_PKCS1v15(temp_key)`、`AES-CBC(key==iv)`、`md5(payload\ncosyKey\ndate\nbody\npath)`、`path` 去 `/algo`。Rust 用 `md-5`/`rsa`/`aes`/`cbc` crate 即可，**不要**移植手写 S-box（那是为「零 pip」付的代价）。
- `derive_id`/`derive_machine_token`/`generate_request_id` 与全部盐字符串——迁移必须**逐字节保持**，否则已入池账号的 `machineId` 全变（破坏性变更，代码注释里明确禁止）。
- `REALM_CONFIGS` 全字段（openapi/gateway/fallback/client_id/redirect_uri/ua/nonce 形态/home_dir/app_dir/`send_client_id`/`send_redirect_uri`）。
- `Account` 的状态机与常量：三族退避公式、`ready()` 判定顺序、G1–G4 四条护栏、`_EXPIRY_NEVER_EPOCH`、`next_checkin_window`（UTC+8 10:00）、`CHECKIN_PROBE_TTL`、`CLAIM_GAP_DEFAULT`。
- `resolve_upstream_key` 的解析优先级链、`MODEL_ALIASES` 表、`merge_catalog` 的「`None` 不覆盖 + primary 决定集合」、`model_entry` 的峰谷价推导、`normalize_reasoning_effort` 的最近档 + 默认档偏向。
- `off_peak_active_now` 的跨午夜判定与「promotion 优先于 is_free」的分支顺序。
- `qoder_anthropic.py` 与 `responses_to_chat`/`chat_to_response`/`stream_responses_events`：纯函数、无全局状态，可直接对译为 Rust；`structured_tool_history_enabled` 的 auto 门槛（cn + `qwen/glm/gm4/gm5` + id 齐备，否则 fail-safe 回退文本化）与 `LEAK_MARKER` 回读守卫（32KB / 200 帧上限）建议整体搬，这是长尾 bug 最集中的地方。
- 双层重试与 SSE 心跳策略、`_short_error_cooldown_wait`「≤10s 就等」的取舍、chunked + keep-alive 的帧层写法。

**需要重写宿主（逻辑保留）**
- HTTP 栈：`BaseHTTPRequestHandler` + `ThreadingHTTPServer` → `axum`/`hyper` + 状态注入。现在是**大量模块级可变单例**（`POOL`、`CURRENT_REALM`、`USAGE_DIR/LOG/SUMMARY`、`PANEL`、`_usage`、`SESSIONS`、`_models_cache`、`_INNER_ERR_STATS`、`_STRUCTURED_LOG_ONCE`），Rust 侧必须收敛进一个 `AppState`（`Arc`），否则测试与多窗口会互相污染。
- 存储：`accounts/*.json` 明文 + `usage.jsonl` 追加 + `usage-summary.json` 全量重写 → 桌面端应改为 SQLite（WAL）+ 系统钥匙串（令牌），并把 `usage-summary.json` 的「每次写全量」换成增量聚合表。
- 配置：20+ 个 `QD_*` 环境变量（`QD_PROXY_DEFAULT_REALM`、`QD_AUTO_REALM`、`QD_SSE_HEARTBEAT`、`QD_STRUCTURED_TOOL_HISTORY`、`QD_TOOL_RESULT_KEEP`、`QD_MACHINE_IDENTITY_CACHE/_TTL/_RESET/_VOTE`、`QD_NATIVE_IDENTITY`、`QD_UMID_DIR`、`QD_ANTHROPIC_THINKING`、`QD_AFFINITY_*`、`QD_MAX_PAYLOAD_BYTES`、`QD_DESKTOP_VERSION`、`QD_PROXY_USAGE_DIR`）→ 桌面端要落成 TOML 配置 + 设置页，且**改值即时生效**（现在靠重启）。
- 看板：`dashboard.html` 4848 行单文件（5 主标签 `gateway/analytics/logs/agents/settings`，25 个 `section` 锚点，`postJSON/getJSON` + `X-Panel-Token` + `localStorage`）→ 应改为前端框架 + Tauri IPC。**不可复用**：DOM 断言类测试（`tests/_test_dashboard_*.js` 通过 `dash_extract.js` 抓函数体跑 Node）与整页 HTML 字符串拼接。
- 面板密码 / LAN 模式 / 端口占用探测 / 控制台关闭处理 / `start-*.bat` / `allow-firewall.*` / Docker 相关：桌面端场景不适用（本机单用户），但这些开关背后的**多 Key 出口绑定与模型闸门**（`api_keys[].realm` + `.models` + `banned_models`）值得保留成「多个下游客户端凭据」的配置项。

**不可移植 / 桌面端需换实现**
- `runtime-info.exe` 原生桥：Windows 依赖官方安装目录（`_read_ini state.ini` 定位 + `resources/umid/runtime-info.exe`），Linux 需 `_install_umid.py` 从 `@qoder-ai/qodercli` npm 包提取并装 gcompat/libstdc++。桌面端**能更自然地定位本机官方客户端**，可保留调用，但必须处理「未安装/组件不可执行」→ 回退 `derived` 并如实标注 `machine_headers_state="omitted"`（现在就是这样，不能发假机器头）。
- 本机凭证扫描的加密层：`os_crypt` + `dpapi_unprotect`(DPAPI) + Chromium `v10` 为 **Windows 专属**；CLI 侧 `AES-128-CBC(key=machine_id[:16])` 跨平台可用。macOS 需换 Keychain 实现。**只读探测 + 两步确认**的产品行为要保留（`scan_desktop_credentials` 不返回任何明文令牌）。
- `qmc_decrypt`（QMC v1）依赖官方客户端 `catalog-v6` 的密钥派生约定：可移植，但一旦官方换格式就会整段失效——桌面端更适合直接调官方客户端自己的接口，把这条降为兜底。
- 零第三方依赖的纯 Python 密码学（AES/GCM/RSA 手写实现、`_INSTALL_UMID`、`dpapi`）：纯为「Docker alpine 无 pip」存在，**不要搬**，Rust crate 直接替代。
- 出站代理「只读环境变量、忽略 Windows 注册表」（`_opener_for_request`）：这是 CLI/容器场景的正确解，桌面端应改为 UI 配置 + 每请求现读，别复用环境变量语义。
- `/agents*` 的 loopback-only 写配置与「从网关 `curl` 取 CLI」：桌面端进程本身就在本机，`/agents/cli` 这条分发通道无意义；改写配置要换成带权限提示的原生操作。
- `CreditsRefresher`（每 1800s 只刷一个账号，失败停放 6h）与 `apply_quota_limits`/`apply_settings` 的「与守卫同批推送 `free_models`」约束：逻辑可留，但它是为「24×7 常驻服务」设计的节流；桌面端按需触发更合适。

**迁移时必须保留的三条不变量**（代码里反复标注的红线）：① `derive_id` 输出逐字节不变；② `temp_key` 的长度/编码不能改（服务端用同串解 `info`，无降级路径）；③ 无真机身份时**一律不发** `cosy-machine*` 六头，而不是发派生假头。
