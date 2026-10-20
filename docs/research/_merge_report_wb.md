# wb-proxy v1.7.12 能力清单（供 Tauri/Rust 桌面端合并参考）

只读分析对象：`workbuddy2api-hub-v1.7.12/wb-proxy`（Python 单进程 API 网关；`python/` 便携运行时已跳过）。
行数为实际总行数（含空行）。所有结论均来自源码/数据文件实证，未运行进程。

---

## 一、模块 → 职责表

| 文件 | 行数 | 职责 | 对外接口 |
|---|---|---|---|
| `wb_proxy.py` | 20185 | 主程序：`BaseHTTPRequestHandler` 单进程路由、API Key/面板会话鉴权、CORS、请求体上限、上游调度与重试、SSE 流式转译、usage 记账、聚合缓存、内嵌看板服务 | 全部 HTTP 路由（见二） |
| `wb_accounts.py` | 3878 | `Account`（凭证+熔断+限流状态机+headers/checkin/daily_chat/report_activity/credits）、`AccountPool`（选号/权重/分页/亲和）、`SessionAffinity`、`CreditsRefresher`、OAuth 登录、桌面凭据导入、导入导出、护栏落地 | 被 `wb_proxy` 直接调用；无独立路由 |
| `wb_pricing.py` | 2557 | OpenRouter 等价花费：内嵌快照、bands 取档、`policy_id` 内容哈希、策略表/时间轴、`PriceRefresher` 线程、未定价缺口报告、人工覆盖 | `GET /pricing`、`POST /pricing/refresh`、`POST /pricing/mapping` |
| `wb_settings.py` | 2056 | `accounts/settings.json` 全部读写与严格校验：面板密码 PBKDF2、多 API Key、limits、upstream/usage/prompt、proxy slots、`PanelSessions` | `GET /settings`、`POST /settings/save`、`GET /settings/reveal`、`POST /panel/*`、`GET/POST /proxy/slots*` |
| `wb_catalog.py` | 1732 | **纯数据**：国际版 `STATIC_INTL_MODELS` / 国内版 `STATIC_CN_MODELS` 静态目录快照（`id/name/credits/maxInputTokens/maxOutputTokens/supports*/reasoning/vendor/tags`） | 无函数，仅常量 |
| `wb_agents.py` | 1474 | 六个客户端（claude-code/codex/opencode/dsh/crush/zcode）配置文本级 upsert、字节级备份/还原、`CLIENTS` 表、可独立运行 CLI | `GET /agents`、`GET /agents/available`、`GET /agents/cli`、`POST /agents/apply`、`POST /agents/restore` |
| `wb_webtools.py` | 1100 | 本地代跑 `web_search` / `web_fetch`：5 引擎轮换、限流冷却、TTL 缓存、SSRF/DNS-rebinding 守卫 | 被 Responses 路径内部调用 |
| `wb_atrest.py` | 916 | 桌面端 `$wbEncrypted` 信封解密（纯 Python AES-256-GCM + 定制 AAD）、keyblob 自检、Windows 进程内存三档扫描回收密钥 | 被 `POST /accounts/import/desktop` 调用 |
| `wb_tasks.py` | 883 | 国内版成长任务：`TASK_SPECS` 19 项、接取/事件上报/领奖、连登管家、夜猫、猫猫旅行 | `GET /tasks`、`POST /tasks/run`、`POST /tasks/travel`、`POST /tasks/streak-bonus` |
| `wb_upstream_pool.py` | 722 | 自建 keep-alive 连接池（绕过 urllib `Connection: close`）、探活、共享 SSLContext、`egress_key()` | 被 `wb_accounts.urlopen` 调用 |
| `wb_logstore.py` | 416 | 可选 SQLite 运行日志持久化（单线程队列 + 三轴保留上限） | `GET /logs`、`GET /logs/export`、`POST /logs/clear` |
| `wb_activity.py` | 387 | 签到/每日活跃结构化历史 JSONL（白名单投影 + 脱敏 + 压实） | `GET /activity/history` |
| `wb_updates.py` | 358 | 自更新**只发现不安装**：GitHub releases 比较版本 | `GET /updates`、`POST /updates/check` |
| `wb_webagent.py` | 283 | ACP over streamable HTTP：SSE + JSON-RPC 驱动网页沙箱会话 | 被国际版每日活跃调用 |
| `wb_modelsdev.py` | 246 | context/output 四级查找（remote > `CONTEXT_FALLBACK` > `model.json` > models.dev 异步） | 被 `model_entry()` 调用 |
| `wb_identity.py` | 231 | 三套出站身分头（workbuddy/vscode/cli）、`uuid7()`、trace 头族 | 被 `Account.headers()` 调用 |
| `wb_scheduler.py` | 230 | 整点排程线程（09/21 签到+旅行、22 保活、01 夜猫） | `GET /scheduler`、`POST /scheduler/trigger`、`POST /scheduler/toggle` |
| `wb_prompt.py` | 128 | system prompt 三模式（passthrough/custom/append）+ 403 降级闸 `DegradeGate` | 被 `build_upstream_body` 调用 |
| `wb_probes.py` | 121 | 输出上限探测（读 `output_probes.json`、`probe_model()`） | 面板脚本调用；结果并入 `/v1/models` |
| `wb_ipintel.py` | 108 | 出口 IP 归属/机房-住宅分类（ip-api.com） | 被 `/proxy/discover`、`/proxy/slots/test` 调用 |
| `wb_egress.py` | 87 | 出口级瞬时失败退避（秒级，与账号级分钟级分离） | 被 `open_upstream` 调用 |
| `wb_fingerprint.py` | 81 | `derive_id()`：uid+salt → 稳定 UUID 形状 machineId/sessionId | 被 identity/billing 头调用 |
| `dashboard.html` | 14683 | 单文件看板（6 主 Tab + 现场生成侧栏导航） | 静态页 `/`、`/dashboard`、`/ui` |

---

## 二、核心能力清单

| 能力 | 实现要点（算法/常量/函数） | HTTP 接口 | 移植价值 |
|---|---|---|---|
| Chat Completions 兼容 | `do_POST` → `_dispatch_chat_post` → `open_upstream`；客户端 body 由 `build_upstream_body()` 规整：`normalize_roles/sanitize_messages/backfill_reasoning_content/merge_adjacent_tool_calls/repack_tool_result_blocks/cleanup_orphan_tool_calls/translate_max_completion_tokens/normalize_tool_choice/normalize_tools/normalize_stop`；强制 `stream=True` + `stream_options.include_usage`；DeepSeek 家族自动补 `thinking:{type:enabled}` 与 `reasoning_effort`（默认 `high`）；`gpt-*` 的 `max_tokens<16` 夹到 16 | `POST /v1/chat/completions`、`/chat/completions`、`/v1/completions`、`/completions` | ★★★ 必移植（协议核心） |
| Responses API | `_handle_responses()`，namespace 工具、`input`→messages 展开、`web_search_call`/`url_citation` 事件、`_` 前缀内部标记在出站前剥离 | `POST /v1/responses`、`/responses` | ★★★ 若桌面端要接 Codex 类客户端 |
| Anthropic Messages 适配 | `_handle_messages()`，默认**关闭**（`WB_ENABLE_ANTHROPIC_MESSAGES`）；关闭时返回 Anthropic 形状 400；`count_tokens` 为 CJK 感知估算 | `POST /v1/messages`、`/messages`、`/v1/messages/count_tokens` | ★ 收益低（缓存命中率仅~11%） |
| `/v1/models` 目录对齐 | `merge_catalog()` + `model_entry()`：静态表∪上游实时目录，`INTL_UI_ORDER`(17)/`CN_UI_ORDER`(14) 排序；过滤 `NON_CHAT_MODELS`(`lite`+虚拟别名)/`NON_CHAT_PREFIXES`(`codewise-`,`completion-`)/`NON_CHAT_SUFFIXES`(`-image-alpha`,`-image-alpha-edit`,`-taco-completion`)；多方言能力字段（`capabilities/abilities/architecture/input_modalities/reasoning_efforts/output_clamp`） | `GET /v1/models`、`/models` | ★★★ 客户端能力宣告全靠它 |
| 多账号池调度 | `AccountPool.pick_for_session()` → `pick()` 四级偏好：手动 `weight>1` 平滑加权 → 临期积分 → 剩余用量优先 → `_rotate_pick` 轮询；`_smooth_weighted_pick()` 为 nginx SWRR；`_page_slice()` 按 uid 排名分页 | 内部 | ★★★ 桌面端多号必需 |
| 会话亲和 | `SessionAffinity(ttl=7200, max_entries=5000, history=3)`：每会话保留 3 个最近账号的 MRU 列表，失败用 `demote()` 而非 `unbind()`；`AFFINITY_MAX_MSGS=400` 以上转页内轮转 | 内部 | ★★★ prompt cache 保命点 |
| 账号级失败治理 | `SOFT_RATE_BASE=600/MAX=7200`、`SOFT_RATE_UNVERIFIED_BASE=60/MAX=600`、`BREAKER_THRESHOLD=3`、`BREAKER_COOLDOWN=1800→21600`、`DEGRADE_THRESHOLD=5`、`DEGRADE_COOLDOWN=600→7200`、`FAIL_WINDOW=600s`（`WB_FAIL_WINDOW`）；`_exponential_backoff(count,base,cap,offset)` 封顶 2^20 步 | 内部 | ★★★ |
| 出口级退避 | `wb_egress`：`EGRESS_BACKOFF_BASE=1.0`、`MAX=15.0`、`STREAK_RESET=60`、jitter 0.75~1.25；仅对 502/503/504 与传输失败记账，`egress_wait` 每请求每出口只记一次，`EGRESS_SLEEP_MAX=5.0` | 内部（开关 `egress_backoff_enabled`） | ★★★ |
| keep-alive 连接池 | `_KeepAlivePool`：键 `(scheme,host,port,proxy)`；`WB_UPSTREAM_POOL_SIZE=2`、`WB_UPSTREAM_POOL_IDLE=90`；`_connection_looks_alive()` 非阻塞试读 1 字节；`_PooledResponse._release()` 在 `_close_conn()` 时归还；stale 连接仅重试一次 | 内部（`WB_UPSTREAM_KEEPALIVE=0` 关闭） | ★★ Rust 有 reqwest/hyper 池，仅借语义 |
| 双区域出口 | `REALM_CONFIGS{intl,cn}`；`detect_model_realm()/exclusive_realm()`；`CURRENT_REALM` 落盘 `active_realm.json` | `GET/POST /realm` | ★★★ |
| 多 Key + 模型白名单 | `wb_settings.api_keys()`：`id/name/key/realm/models(fnmatch 通配)/enabled/deletedAt/concurrency`；`key_allows_model()`；`_acquire_key_slot()` 每 Key 并发上限 | `GET/POST /settings`（reveal 单条） | ★★★ |
| 四条限额护栏 | `LIMIT_KEYS=("reserve_credits","daily_token_limit","daily_credit_limit","model_daily_token_limit")`，scope `global/intl/cn`，本地 0 点解封；`apply_daily_guards()/apply_daily_token_limit()/apply_daily_credit_limit()/apply_model_daily_token_limit()`；全池达额 → 429 + `Retry-After` | `POST /settings/save` | ★★★ |
| 剩余用量估算 | `note_limit_event()` 在 429 带重置墙钟时落 `limit-events.jsonl`；`_pair_buffer` 时间戳+累计和双数组、`_range_sum` 两次二分 O(log n)；预算 = 撞线窗口用量按账号平均再跨账号平均；`WB_REMAINING_TTL=15s` + `ETag`/304 | `GET /usage/remaining` | ★★★ 极省算力 |
| 积分获取历史 | `credit_grants()` 摊平 `credits.packages[]`；状态机 `_grant_status()`：`expired > used-up > in_usage > available`；来源推断 + 关联本机签到动作（前 2 小时内最近一次尝试） | `GET /accounts/credits/grants` | ★★ |
| OpenRouter 等价花费 | `compute_row()`：`(uncached*miss + cached*hit + out*output)/unit*factor`；`band_for()` 取**最后一个**命中档（长度阈值/UTC 时段）；`_pick()` 缺 hit 价回落 miss 价；策略 `policy_id=sha1(model,flat,bands,usd_cny)[:12]` | `GET /pricing`、`POST /pricing/refresh`、`POST /pricing/mapping` | ★★★ |
| 本地网络工具 | `install_tool_defs()` 替换同名声明；`_search_fanout()` 引擎链 `duckduckgo→bing→wikipedia→stackoverflow→news`；`COOLDOWN_BLOCKED=600`、`COOLDOWN_ERROR=60`、`MIN_INTERVAL=0.5`、`SEARCH_BUDGET=25`、`CACHE_TTL=600`；`MAX_RESULTS=10`、`MAX_FETCH_CHARS=100000`、`MAX_BODY_BYTES=4MB`；`MAX_WEB_ROUNDS`（默认 3，上限 8） | 内部（`local_web_tools` 开关） | ★★（Rust 侧需自建抓取） |
| 一键智能体配置 | `CLIENTS` 6 项；`plan()` 同时驱动 dry-run 与写入；`_atomic_write()`(tmp+`os.replace`, mode 0600)、`BACKUP_KEEP=10`、`MAX_CONFIG_BYTES=8MB`、YAML/TOML/.env 文本级 upsert | `/agents*` | ★★（桌面端本身可直写） |
| 设备指纹 + 出站身分 | `derive_id(uid,salt)`=md5(`salt:uid`)，第 13 位固定 `4`、第 17 位固定 `a` → UUID 形状；`session_id()` 用 salt `"session"`，machine 用 `"machine"`；`generate_request_id()` 每条随机 32hex；`build_identity_headers()` 三套头 + `trace_headers()` | 内部 | ★★★ |
| 桌面凭据解密导入 | `derive_key_id(key)=sha256(key).hex[:16]`；AAD 逐字节构造；`unwrap_keyblob()` 双自检；`hunt_key()` 三档（literal→secret 正则→window stride 8→stride 1）；`CHUNK=4MB`、`OVERLAP=31`、`SLICE=64MB`、workers≤24 | `POST /accounts/import/desktop`（`recoverKey`/`keyText`/`path`） | ★★★（Rust 只需 crate `aes-gcm`） |
| 国内自动化 | `wb_tasks.TASK_SPECS`(19)/`DESKTOP_ONLY_TASKS`(4 不可伪造)/`NIGHT_TASK_CODES`/`EXPERT_ID_POOL`(18)/`TEAM_ID_POOL`(8)；`run_growth_tasks()`、`run_streak_bonus()`、`run_night_growth()`、`do_cat_travel()`；事件间隔 ≥1.0s | `/tasks*` | ★★ |
| 国际每日活跃 | 双通道：桌面 `daily_chat()` + 网页 `daily_chat_web()` → `_daily_chat_web_once()` → `wb_webagent.run_turn()`（`session/set_model` 钉 `DAILY_CHAT_MODEL=deepseek-v4.1-flash`）；`WEB_TURN_ATTEMPTS=3`、`WB_WEB_TURN_TIMEOUT=120` | `POST /accounts/daily-chat`、`/accounts/daily-chat-web` | ★★ |
| 调度器 | `Scheduler(checkin_hours=[9,21], travel_hours=[9,21], keepalive_hours=[22], cat_hours=[1])`；60 秒粒度命中 `cur_min==0`；`_run_lock` 防重叠；每账号随机错峰 2~5s；保活阈值 `exp-now<7200` | `/scheduler*` | ★★★ |
| 运行日志 | `LOG_BUFFER=deque(maxlen=2000)` + 关键词自动分级/归类；`LogStore` SQLite 可选 | `/logs*` | ★★ |
| 更新发现 | `RELEASES_URL`（API）+ `LATEST_RELEASE_URL`（302 回退）、`SCHEDULED_INTERVAL_SECONDS=24h`、`FIRST_DELAY_SECONDS=30`、`POLL_SECONDS=300`、`VERSION_RE=^v?(\d+)\.(\d+)\.(\d+)$` | `/updates*` | ★★ |
| 面板会话 | `PBKDF2_ROUNDS=120000`、`SESSION_TTL=7d`、`X-Panel-Token` 头（禁用 query 传 token）、默认密码 `admin`、`PAGE_KEEP_VISIBLE="settings"` | `/panel/login|logout|password`、`/panel/status` | ★★ |
| 并发与体积护栏 | `MAX_PAYLOAD_BYTES=16MB`、`MAX_CONCURRENT_CHAT=32`（`auto` = 就绪账号数，下限 32）、`CHAT_SLOT_WAIT_SECONDS=30` | 全局 | ★★★ |

---

## 三、数据模型

**`accounts/<uid>.json`**（单账号凭证；`Account.to_dict()`）
`uid, nickname, domain, realm(intl|cn), platform, product(workbuddy|vscode|cli), enterpriseId, accessToken, refreshToken, expiresAt, addedAt, source, proxySlot, proxy, enabled, weight(1..100), note(≤200), disabledModels[](小写精确匹配), credits{remain,used,size,used_percent,remain_percent,is_paid_user,checkin{today_checked_in,streak_days,daily_credit,today_credit,active},earliest_expiring{name,package_code,remain,cycle_end_time,days_left},packages[]{name,package_code,product_name,sub_product_name,grant_reason,resource_id,deal_name,create_time,remain,used,size,unit,in_usage,auto_renew,cycle_start_time,cycle_end_time,expire_time,no_expiry,days_left,is_expired,status},updated_at,updated_iso}, lastCheckin, lastDailyChat, lastActivityReport, modelCaps{}`
运行时态（`VOLATILE_FIELDS`/`RUNTIME_ONLY_FIELDS` 不落盘）：`last_error, last_error_detail, cooldown_until, model_cooldowns{model:until}, model_cap_started, soft_streak, unscoped_streak, fails, strikes`。

**`accounts/settings.json`**（`wb_settings` 键全集）
`panel_password`(PBKDF2 摘要+盐)、`api_key`、`api_keys[]{id,name,key,realm,models,enabled,deletedAt,concurrency}`、`auth_disabled`、`limits{reserve_credits|daily_token_limit|daily_credit_limit|model_daily_token_limit → {global,intl,cn}}`、`expiring_window_days`(默认 7)、`credits_refresh_hours`(12)、`pricing_enabled / pricing_refresh_minutes(5) / pricing_variant_inherit`、`local_web_tools`、`daily_chat_web / daily_chat_web_retry / daily_chat_web_attempts(3) / daily_chat_web_timeout(120) / daily_chat_model`、`enable_anthropic_messages`、`free_model_credit_fallback`、`auto_switch_product`、`remaining_priority_enabled`、`egress_backoff_enabled`、`update_check_enabled / update_check{last_attempt,last_success,latest_version}`、`log_db_enabled / log_db_max_rows|max_days|max_bytes`、`usage{hour_keep_hours:48,day_keep_days:32}`、`upstream{header_timeout_seconds:120,idle_timeout_seconds:300,device_token,device_token_file}`、`prompt{mode:passthrough,file}`、`proxy_slots[]{id,name,url,country,country_code,ip_type,isp,asn,enabled,...}`、`ui_language(zh|zh-Hant|en)`、`accounts_collapsed / accounts_separate_tab / key_before_hidden / hidden_pages / section_max_height(640)`。

**`accounts/active_realm.json`**：`{realm, updated_at, updated_iso}`
**`accounts/integration-state.json`**：`{<client_id>:{applied_at, base_url, model, files[]{path, existed, backup, sha256, bytes}}}`
**`accounts/model.json`**：`{bare_model_id: [context, output]}`（models.dev 索引缓存，100KB 级）
**`accounts/output_probes.json`**：`{model:{model,requested_max_tokens,observed_completion_tokens,finish_reason,clamped,at}}`
**`catalog/product-config-<realm>.json`**：上游 `GET /v3/config` 原始响应 `{code,msg,requestId,data:{agents[]{commands,description,instructions,modelTags,models[],name,tags,tools[]}}}`
**`pricing/pricing.json`**：`{meta{generated_at,base_currency,usd_cny:7.1,sources,note}, models{id:{display,source,currency,unit:1000000,or_id,flat{input_cache_hit,input_cache_miss,output},bands[{flat,min_prompt_tokens|days+start+end}]}}}`

**`usage/usage.jsonl`**（一行一请求，成功/失败共用文件，靠 `error` 区分）
成功：`at, iso, model, stream, outcome(completed|client_aborted|upstream_aborted|failed), elapsed_ms, ttft_ms, gen_ms, prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens, total_tokens, credit, msgs_sha, n_msgs, system_sha, prefix_sha, account, realm, key(settings id，非密钥), cost_policy(策略 id), reasoning_effort, tokens_per_sec, cache_hit_pct[, requested_model][, usage_missing]`
失败：`at, iso, model, error:true, outcome, status, message(≤200), elapsed_ms[, stream][, ttft_ms][, gen_ms][, account][, realm][, key] + usage 字段（流中断时）`

**`usage/activity_history.jsonl`**：`ts(本地带偏移 ISO), uid, nickname, realm, task(checkin|daily_chat), trigger(scheduler|manual|account_add|account_import|unknown), ok(bool), message(≤200，脱敏)`；`ALLOWED_FIELDS` 白名单投影
**`usage/pricing-policies.jsonl`**：`id, model, or_id, currency, unit, usd_cny, flat{}, bands|null, via(direct|variant|override), inherited_from, override_from, first_seen`
**`usage/pricing-timeline.jsonl`**：`{at, policies{model:policy_id}}`（每次真的变价才追加）
**`usage/limit-events.jsonl`**：`{uid|account, model, reset_at, at, realm, ...}`（撞线样本，剩余估算的预算来源）
**`usage/usage-aggregate-cache-by_account.json`**：`{schema:5, kind:"by_account", written_at, entries[{offset, key[3], tail(hex), buckets{uid:{account,requests,prompt_tokens,completion_tokens,reasoning_tokens,cached_tokens,total_tokens,models{}}}}]}`（增量聚合缓存）
**`usage/logs.sqlite3`**：`logs(id, ts, time, level, tag, msg)` + `idx_logs_level`、`idx_logs_tag`
**`accounts/agent-backups/<client>/`**：`<yyyymmdd-HHMMSS>-<原文件名>`，每文件保留 10 份

---

## 四、关键算法细节

### 1. 上游池负载均衡与熔断
- 选号顺序（`AccountPool.pick`）：① `_pick_weighted_first`（仅 `weight>1`，`_smooth_weighted_pick(...,"_manual_weights")`）② `_pick_expiring_first`（`in_expiring_window()`，权重 `max(1, round(window - days)+1)`）③ `_pick_remaining_first`（`self._remaining_weights`，键 `(uid, model)`）④ `_rotate_pick`（全局 `_cursor` / 页级 `_page_cursors`）。
- SWRR：每轮 `state[key] += weight`，取最大者，中选者 `-= 总权重`；全等权重退化为纯轮转。状态在 `self._lock` 内读写，但 `ready()` 判断故意放在锁外（`ready()` 可能触发 token 刷新走网络，不能串行化整个区域）。
- 页面：`_page_slice(snapshot, (index,size))`，rank = 该区域账号按 uid 排序的名次，`rank % size == index % size`；`size>=len(ranks)` 时整池。
- 会话亲和：`pick_for_session` 先按 `affinity.recent(session_key)` 找可用暖号并 `bind`；`page is not None` 时不绑（保留轮转）。
- 熔断阶梯：`fails` 连续（`FAIL_WINDOW=600s` 内）达 `BREAKER_THRESHOLD=3` → `breaker_backoff` = 1800s 起每次翻倍、封顶 21600s；`unknown` 失败走 `DEGRADE_THRESHOLD=5` / 600s→7200s。成功即 `note_success()` 清零。
- 429 三分支（`open_upstream`）：`rate_limit_is_account_level(detail, reset_at)` → 账号级 `note_soft_rate()`（600→7200）→ 换号；否则按模型级 `note_error(model=..., until=reset_at)`；无重置时钟且形状可识别 → `note_unscoped_rate()`（60→600，只 parking 该模型）；`rate_limit_is_cap(detail)`（上游 code 6004）才 `note_model_cap()` 并落盘。
- 403 三分支：`restriction_is_account_level()`（code 11140）→ 冷却 `RESTRICTED_403_COOLDOWN=7200`；否则 `DegradeGate.trigger()` 换极简提示词重试一次；再不行透传。
- 402 → `note_balance_cooled()` 停到次日 04:00（`next_local_4am()`）；401 → 60s；5xx → 记账号或（502/503/504 且开关开）记出口；本机断网 `looks_like_local_network()` 不记账、原地重试。
- `max_attempts = max(2, total) + 1 + MAX_PRODUCT_SWITCHES(4)`；`tried` 集合 + `charged` 集合（一次请求对同一账号最多记一笔）；`transient_hits` 触发整池重轮并 `sleep(min(1.5*hits,3.0))`。

### 2. at-rest 加密（`wb_atrest`）
- `key = sha256(atRestSecretKey_b64_string)`（对**字符串**做 sha256，不是对它解码的 32 字节）；`keyId = sha256(key).hex[:16]`（= `derive_key_id`）。
- AAD 逐字节：`b"WB-AAD\0" | 0x01 | LP(FRAMING_MAGIC[framing]) | LP("sym-v1") | u32(suite=1) | LP(keyId) | framingCode | 0x00 | 0x00`，LP = u32be 长度 + utf8；`FRAMING_MAGIC{file:"WBEF1",field:"WBEV1",record:"WBER1",stream:"WBES1"}`，`FRAMING_CODE{1,2,3,4}`。
- 信封：`{"$wbEncrypted":1,"envelope":base64(JSON{suite,keyId,nonce,authTag,ciphertext})}`，AES-256-GCM。
- 纯 Python AES-256（`_AES256`，列主序状态）+ GHASH（`_gf_mul`，`_REDUCTION=0xE1000...`）；`_j0()` 对 12 字节 nonce 直接 `nonce||00000001`；`_gctr` 只递增低 32 位。
- 密钥来源：磁盘 `keyblob`（`<配置目录>/keyblob`，`CONFIG_DIR_CANDIDATES=(".workbuddy-ai",".workbuddy")`，可用 `WORKBUDDY_CONFIG_DIR`/`CODEBUDDY_CONFIG_DIR` 覆盖）里的 `slots[]{type:"static-v1", protectorKeyId, wrapped}`；`unwrap_keyblob()` 校验 `protectorKeyId==derive_key_id(key)` 且解出的 32 字节主密钥的 `derive_key_id` 等于 `blob.keyId`。
- 恢复路径：Windows `hunt_key()` 只读 `OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ)`，只扫 `MEM_COMMIT & MEM_PRIVATE & READABLE & !PAGE_GUARD` 区域，按字节数均分给 ≤24 个 `subprocess` worker（`--scan-worker`，因绿色包无 multiprocessing），四档命中即止：`MODE_LITERAL`（`atRestSecretKey` 字面量 + UTF-16 变体）→ `MODE_SECRET`（`_SECRET_PATTERNS` 43 字符 base64 + 边界断言）→ `MODE_WINDOW` stride 8 → stride 1；每个候选仍需 `unwrap_keyblob()` 复核。macOS/Linux 走 `adopt_key()`（手工粘贴 64hex 或 44 字符 base64）。密钥只存进程内存 `_KEY_CACHE`，`cached_key()` 每次用 keyblob 复核。

### 3. egress / ipintel 评分
- 出口身份 = `wb_upstream_pool.egress_key(url, proxy)` → `(scheme, host, port, proxy_raw)`，且「配了代理但命中 `urllib.request.proxy_bypass(host)`」折叠成直连 `""`（与连接池键同口径，故直连账号天然共用一个计数器）。
- 评分/退避模型（无 ML，纯阶梯）：`_state[egress_key] = [streak, last_hit_epoch]`；`now - last_hit > EGRESS_STREAK_RESET(60s)` 重置 streak；`transient_backoff(streak) = min(1.0 * 2**min(streak-1,20), 15.0)`；实际退避乘 `uniform(0.75,1.25)`；`note_success()` 直接 `pop`。
- `wb_ipintel` 不是评分，是二分类 + 元数据：`GEO_ENDPOINT = http://ip-api.com/json/<ip>?lang=zh-CN&fields=status,message,country,countryCode,isp,org,as,hosting,proxy,mobile`，`LOOKUP_TIMEOUT=8`；`classify()` 只看 `hosting === True/False` → `datacenter`/`residential`，其余留空（不猜）；`FIELDS=("country","country_code","ip_type","isp","asn")`；`slot_name()` 生成「美国 住宅」式默认槽名。失败返回 `empty()` 而非抛错。

### 4. 指纹生成（`wb_fingerprint`）
- `derive_id(uid, salt)`：`seed=f"{salt}:{uid or 'anonymous'}"` → `md5(seed).hexdigest()` → 第 13 位强制 `"4"`、第 17 位强制 `"a"` → 8-4-4-4-12 拼成 UUIDv4 形状。幂等（同账号恒定），跨账号无关联。
- salt 取值：`"machine"`（machineId）、`"session"`（sessionId）。`generate_request_id()` 每条 `uuid.uuid4().hex`（32hex，故意不掺 uid，避免成为「网关特征」）。
- `get_desktop_fingerprint()` 用于事件上报：`timezone:"Asia/Shanghai", reportDelay:2000, product:"SaaS", releaseDate:1789036585355, commit:"5f9692923c93033111c51ad7b003eb80204a9b75", ideName/ideType:"WorkBuddy", ideVersion:"5.8.4", extName:"workbuddy-desktop", os/arch/osVersion:"win32/x64/10.0.26220", cpuCores:20, memorySize:24, timestamp, presentAt`。
- 出站 ID 族另在 `wb_identity`：`uuid7()`（RFC 9562，48 位毫秒 + ver 7 + variant 10）；`X-Conversation-ID` 带连字符，`X-Request-ID`/`X-Conversation-Message-ID`/`-Root-Request-ID` 为 32hex；`trace_headers()` 给 `traceparent/b3/X-B3-*`。

### 5. logstore 索引与轮转（`wb_logstore`）
- 单后台线程 + 单连接，全部操作走 `queue.Queue`；`append` 是纯 put（永不阻塞请求路径），读路径 `_submit(..., wait=True)` 走 `threading.Event`（超时 8s）。
- 批量：`_BATCH_MAX=256` 条/事务，一次 fsync。
- 索引：`idx_logs_level`、`idx_logs_tag`；检索用 `instr(LOWER(msg),?)`/`instr(LOWER(tag),?)` 而非 LIKE（无通配注入），`since_id` 走 `id > ?` 升序增量拉取，无 `since_id` 时 `ORDER BY id DESC LIMIT ?` 后 `reverse()`。
- 三轴独立保留（空 = 该轴不限）：`_TRIM_INTERVAL=5.0`（rows/days，`DELETE ... id NOT IN (SELECT id ... ORDER BY id DESC LIMIT n)`）、`_TRIM_BYTES_INTERVAL=30.0`（bytes，按 `size/count` 估算要删多少、`ORDER BY id ASC LIMIT drop` + `VACUUM`，目标 `cap*0.85`，最多 4 轮）、`_IDLE_TICK=5.0`（空闲也裁剪）。
- 参数：`PRAGMA auto_vacuum=INCREMENTAL`、`journal_mode=DELETE`（**故意不用 WAL**，否则 cap 收缩不到 sidecar）、`synchronous=NORMAL`；`_MSG_MAX=64KB`；`max_id()` 用独立连接在启动时恢复 `_LOG_COUNTER`。

### 6. pricing 计算与 timeline/policies
- 单价单位：`unit=1000000`（每百万 token），`meta.usd_cny=7.1`，结果存 CNY，展示 USD 时除回。
- `compute_row`：`cached=min(cached_tokens, prompt_tokens)`，`uncached=prompt-cached`，`cny=(uncached*miss + cached*hit + output*out_rate)/unit*factor`；`output` 用 `completion_tokens`（已含 reasoning，不重复计）；`_pick(rate,"input_cache_hit","input_cache_miss")` 保证不缺 hit 价时按 miss 价算（不低估）。`known=False` 时 `cny=0` 并在缺口报告里列出，绝不猜。
- 取档：`_utc_parts(epoch)` → `(weekday, HHMM)`；`_band_applies()` 支持两类互斥条件（`min_prompt_tokens` 或 `days+start+end`）；`band_for()` 遍历所有 band，**取最后一个命中**（源列表按阈值升序 → 等价于取最紧档）；无命中回 `flat`。
- 策略与时间轴：`policy_id(model,flat,bands,usd_cny)=sha1(json(sorted))[:12]`（汇率进哈希，汇率变即新策略）；`pricing-policies.jsonl` 按 id 去重（`INSERT` 语义 = 内容寻址，同价不重写）；`pricing-timeline.jsonl` 仅在实际变化时 append `{at, policies{}}`；请求行只记 `cost_policy` 引用，调价不改历史数字。补算：`policy_ref_at()` 对「早于该模型第一个策略」的行按最近策略补，面板标 `*`。
- 维护：`prune_policies(protect=set(assignment.values()))` 只在 `added>0` 时跑（只有新价才会让旧价变孤儿），保护「每模型现行策略 + 刚抓的一批 + 每模型最后一行」。
- 匹配链：面板手填 `runtime_overrides()`（`pricing-overrides.json`）→ 源码 `OVERRIDES` → 归一化同名唯一命中 → 变体后缀继承（`VARIANT_SUFFIXES`，只有 `-lkeap/-taiji/-volc/-sg` 参与剥离，`-f/-dev/-x` 不剥；仍要求唯一命中）。
- 刷新：`PriceRefresher(interval_minutes=DEFAULT_REFRESH_MINUTES=5)`，`run_once()` 内重读总开关；`live_ids()` 把「内置目录∪实时目录」的新名字纳入；`remember_live_index()` 只在一轮真的成功后发布索引，供请求路径 `ensure_policy()` 用**内存**索引按需定价（请求路径零网络）。
- 出厂快照：`_JSON` 内嵌（`pricing/pricing.json` 或 `WB_PRICING_FILE` 覆盖，`load_pricing()` 按 `mtime` 缓存）。

### 7. models.dev 目录同步（`wb_modelsdev`）
- 四级（每栏位独立）：`remote>0` → `CONTEXT_FALLBACK`（内建表，27 项，如 `deepseek-v4.1-flash:(1000000,384000)`、`hy3:(192000,64000)`、`auto:(168000,0)`）→ 本地 `accounts/model.json` → `DEFAULT_CONTEXT=1000000`。未知 output → `None`（省略字段，不编造）。
- 同步：`MODELSDEV_URL="https://models.dev/api.json"`、`MODELSDEV_TIMEOUT=5`、`MODELSDEV_FETCH_COOLDOWN=300`（进程级节流）、`MODELSDEV_NEGATIVE_TTL=24h`、`MODELSDEV_MAX_BODY=32MB`、`MODELSDEV_VALUE_MAX=1e9`；`refresh_async()` 守护线程，失败静默（只 log 一行），成功后 `save_index()`（tmp+`os.replace`）。
- 聚合算法 `build_index(doc)`：按 `bare_id`（取 `/` 尾段小写）投票；`VENDOR_SOURCES=("zai","moonshotai","moonshotai-cn","openai","google","deepseek","minimax")` 官方厂优先；否则取出现次数最多的 `(context,output)`，并列取较小值；`context<=0 and output<=0` 或超上限丢弃。
- 调用点：`model_entry()` → `wb_modelsdev.lookup(mid, meta.maxInputTokens, meta.maxOutputTokens)`；`/v1/models` 每次顺带 `refresh_async()` 温缓存。

### 8. probes 健康探测（`wb_probes`）
- 目的不是存活探测，而是反推**上游真实输出钳制值**：送一次 `max_tokens=1000000` 的「写 1..200000」请求，若 `finish_reason=="length"` 则本次 `completion_tokens` 即上游钳制值。
- 常量：`PROBE_FILENAME="output_probes.json"`、`CACHE_TTL=60`、`PROBE_PROMPT`（固定英文长输出指令）、`timeout=900`、`_cache={at,dir,data}` 模块级缓存。
- 落盘 `save_probe()` 用 tmp+`os.replace`；`clamp_for(dir, model)` 给 `/v1/models` 加 `output_clamp` / `max_output_tokens_clamped`（**只标注，不覆盖规格**）。`dry_run=True` 只回请求预览，不发网络。

### 9. webagent / webtools 工具执行
- `wb_webagent`（ACP over streamable HTTP）：`AcpChannel.open()` 发 `GET <link>` + `Accept: text/event-stream`，从响应头取 `Acp-Connection-Id`；后台线程把 `data:` 行解析成 JSON-RPC 塞 `queue`，统计 `session/update` 与 `agent_message_chunk`；`request()` 用 `POST <link>` + `Acp-Connection-Id` 发 JSON-RPC；`run_turn()` 序列固定 `initialize(id=1, protocolVersion=1, clientCapabilities{fs:{readTextFile:false,writeTextFile:false},terminal:false})` → `session/load(id=2, cwd="/workspace", mcpServers=[])` →（可选）`session/set_model(id=3,{sessionId,modelId})` → `session/prompt(id=4, prompt=[{type:"text",text}])`；再轮询 `poll_status()` 到 `completed`（`failed|error` 直接停），`wait_seconds` 默认 120、`poll_interval=3`；代理走 `_proxy_parts()` 显式塞 `Proxy-Authorization`（裸 `http.client.set_tunnel` 不会自动加，缺了必 407）。
- `wb_webtools`：`client_wants_web(tools)` 认 `SEARCH_DECL_TYPES=("web_search","web_search_preview","web_search_preview_2025_03_11")`/`FETCH_DECL_TYPES=("web_fetch",)`；`install_tool_defs()` 先删同名再插唯一一份；`query_args()` 同时接受 `query/queries/q` 并合并；`_search_fanout()` 按 `_chain_for(query)`（`_has_cjk` 影响顺序）逐引擎试，`_page_looks_blocked()` 用 `GENERIC_BLOCKED` 关键词 + HTTP 202/403/429 判定，命中则 `_mark_cooling(name, COOLDOWN_BLOCKED|COOLDOWN_ERROR)`；`_TTLCache` + `_single_flight(key)` 去重；`_throttle()` 保证 `MIN_INTERVAL`；`_guard_url(url, resolve=True)` 逐字面 + DNS 解析后全部必须是公网地址（`ipaddress`，挡 DNS rebinding），`_SafeRedirectHandler` 校验跳转目标；`fetch()` 只收文本型（`_is_textual_content_type`）、gzip、`MAX_FETCH_CHARS` 截断；全部失败回 `_UNAVAILABLE_PREFIX` 并要模型不要重试同一句（**绝不把失败伪装成「查无结果」**，也**不合成 wrapup**）。

### 10. updates 自更新（`wb_updates`）
- 只**发现**，不下载/不解包/不重启。主路径 `RELEASES_URL="https://api.github.com/repos/ardeyouxipianyi/workbuddy2api-hub/releases"`；失败（403/连接被掐）自动回退 `LATEST_RELEASE_URL=".../releases/latest"`，跟随 302 后用 `TAG_RE=/releases/tag/([^/?#]+)` 从最终 URL 取 tag（不解析 HTML）。
- 取舍：`pick_latest()` 跳过 `draft`/`prerelease`、tag 非 `^v?\d+\.\d+\.\d+$` 一律跳过，取版本号最大者而非列表第一个；`safe_url()` 只放行 `https://github.com/` 前缀且截 300 字符；`describe_failure()` 只输出 `HTTP n` / 异常类型，绝不带 URL 与响应体；`MAX_BODY=2MB`；`GITHUB_TOKEN` 仅从环境变量读、不落盘。
- 调度：`SCHEDULED_INTERVAL_SECONDS=24h`、`FIRST_DELAY_SECONDS=30`（先让端口与账号池就绪）、`POLL_SECONDS=300`；`check(manual=True)` 绕过开关与窗口；`due()` 只是 `now-last_attempt>=24h`；状态只落 `last_attempt/last_success/latest_version` 三个字段。

### 11. scheduler 任务编排（`wb_scheduler`）
- 结构：`Scheduler(pool)`，四张小时表 `checkin_hours=[9,21]`、`travel_hours=[9,21]`、`keepalive_hours=[22]`、`cat_hours=[1]`，`all_hours` 为并集排序。
- 循环：启动 `sleep(10)` 后先跑一次「启动初次初始化巡检」；主循环每 `30s` 醒一次，`cur_min==0 and cur_hour in all_hours` 命中即跑，跑完 `sleep(65)` 避免同分钟重复触发；`_calc_next_fire()` 只用于面板展示下次时刻。
- 单账号顺序（每账号之间 `random.uniform(2.0,5.0)` 错峰，步间 `sleep(1.0)`）：① Token 保活 `exp and (exp-time.time())<7200` → `acc.refresh()`；② `realm=="cn"`：`can_checkin()→checkin(trigger="scheduler")`、`can_report_activity()→report_activity()`、`do_cat_travel()`、`run_streak_bonus()`、`tm_hour in cat_hours → run_night_growth()`；③ `realm=="intl"`：`can_daily_chat()→daily_chat(trigger="scheduler")`。
- 并发保护：`_run_lock`（`acquire(blocking=False)`，重叠则跳过并记日志）；`trigger_now()` 起线程；`status()` 回 `enabled/mode/last_run_time/next_run_time/logs[-20:]`；日志双写：本地 `logs` 环形 60 条 + `wb_proxy.add_log_entry(tag="tasks"|"scheduler")`。
- 与请求路径隔离：四条护栏只在请求路径生效，定时任务不受限额影响（README 明示）。

---

## 五、桌面端可复用点与不可移植点

### 值得移植（附理由）
1. **`/v1/models` 目录对齐 + `model_entry()` 多方言字段**：纯数据映射，Rust 侧一个 `serde` 结构体即可，直接决定客户端能力宣告与上下文窗口；`wb_catalog.py` 是现成 JSON 快照。
2. **`wb_modelsdev.lookup` 四级查找 + `CONTEXT_FALLBACK` 表**：无状态纯函数，未知 context 保守取 1M、未知 output 省略，规则清晰无坑。
3. **账号池调度 + 会话亲和 + 熔断阶梯**：`SOFT_RATE/BREAKER/DEGRADE` 三阶梯与 `FAIL_WINDOW` 是有实证依据的（issue #271 记录了「一次抖动关 13 个号」的教训）；亲和 MRU(3) 直击 prompt cache 命中率。
4. **出口级退避与 `egress_key()`**：把瞬时失败记在出口而非账号，是低成本高收益的修正；`std::time` + `Mutex<HashMap>` 即可复刻。
5. **限额护栏 + 剩余用量估算（时间戳+累计和双数组 + O(log n) 区间和 + 15s TTL + ETag）**：算法与性能数据都有实测（2 万行 ~0.7ms），是桌面端最容易「实现歪」的一块。
6. **pricing 计算 + `policy_id` 内容寻址策略/时间轴**：请求只存引用、调价不改历史，是正确的审计模型；`band_for` 取最后命中档的规则必须照抄（否则会算便宜）。
7. **`wb_activity` 白名单投影 + 脱敏**：`ALLOWED_FIELDS` + `_SECRET_PATTERNS` + 截断，是「日志即产物」场景的正确姿势。
8. **`wb_atrest` 的 KDF/AAD 才能对接现状**：`sha256(b64_string)` 与逐字节 AAD 是唯一能解开现有 `.info` 的实现；Rust 用 `aes-gcm` + `sha2` 直接照抄常量。Windows 内存三档扫描可保留为可选 feature（`windows` crate，只读）。
9. **`wb_identity` 三套身分头 + `uuid7()` + trace 头族**：必须与官方客户端同形，属于协议契约。
10. **`wb_prompt` 三模式 + `DegradeGate`（次日 00:00 CST 重置、不续期）**：403 指纹误杀的实用补丁。
11. **`wb_egress`/`wb_logstore` 的「不阻塞请求路径」原则**：队列 + 后台单线程，Rust 侧对应 `tokio::mpsc` + 单 task。
12. **`wb_updates` 的两跳回退 + `safe_url()` + 失败只输出状态码**：安全边界清楚。

### 不可移植 / 应重写
1. **`wb_proxy.py`（20k 行 `BaseHTTPRequestHandler` 单文件）**：路由靠一长串 `if path == ...`，鉴权/序列化/SSE 混在 handler 里。Rust 必须换成 `axum`/`tauri::ipc` 分层，硬搬只会把技术债搬过去。
2. **`wb_upstream_pool` 整个连接池**：存在的唯一理由是 urllib 写死 `Connection: close`。Rust 的 `reqwest`/`hyper` 自带 keep-alive + 连接池 + TLS 会话复用，只需保留 `PoolBypass` 语义（3xx 交回上层跟随、socks 走另一条路）。
3. **`dashboard.html`（14683 行单文件）**：内联 CSS/JS、`switchMainTab()` + 现场生成侧栏、大量 `postJSON/getJSON`。桌面端是原生 UI，只能取「信息架构」（6 Tab、侧栏锚点、`section_max_height=640` 折叠、`hidden_pages`/`PAGE_KEEP_VISIBLE` 规则），不能取代码。
4. **`wb_accounts` 的桌面凭据扫描/导入链路**：`scan_desktop_credentials()`、`desktop_auth_dirs()` 依赖 WorkBuddy 桌面端在**本机**的安装布局与 `.info` 文件；桌面端内嵌支持后应改走自身已持有的凭据，不要再扫别人。
5. **`wb_agents`（写别的客户端配置）**：一键改 `~/.claude`、`~/.codex`、`~/.dsh` 等，是「网关在另一台机器」场景的补丁；桌面端自身就是目标客户端，这套逻辑（含字节级备份/还原、`_resolve_home` 注入）价值大幅下降。若保留，只移植 `_atomic_write` 与 `_target_mode`（0600）两条安全约定。
6. **`wb_atrest._run_scan_workers` 的 `subprocess` + `--scan-worker` 方案**：为「绿色包无 multiprocessing」而设；Rust 应用线程/`rayon` 即可，不必保留进程模型（但 `SLICE=64MB`/`OVERLAP=31` 的切分思路值得留）。
7. **`wb_tasks` 的国内版成长任务/连登/夜猫/猫猫**：`TASK_SPECS`、`EXPERT_ID_POOL`(18 个硬编码 `ex_*` id)、`TEAM_ID_POOL`(8 个团队 id)、`DESKTOP_ONLY_TASKS` 都是**2026-09 实测值**，属于上游"脏"契约，随时会腐化；且 4 个任务被明示「只能真实点击」。桌面端若已有 WorkBuddy 支持，应走原生能力而不是伪造事件。
8. **`wb_scheduler` 的整点排程 + `sleep(65)` 去重**：长驻进程 + 本地时钟假设。桌面端应换成系统计划任务/`tokio` interval + 持久化 `last_run`，并注意**用户不常开电脑**这一根本差异（漏签补签逻辑要重做）。
9. **`wb_logstore` SQLite 三轴裁剪**：桌面端可用同一套 SQL，但 `VACUUM` 在用户盘上代价高；建议改 `rusqlite` + 按 `ts` 分区或直接 `tracing` 滚动文件。
10. **`wb_webtools` 的 5 引擎 HTML 抓取**：解析 DuckDuckGo/Bing 的 HTML 结构，属易碎实现；`WB_WEB_TOOLS_PROXY` 与 SSRF 守卫（`_guard_url` 解析后必须是公网）应保留，但抓取层建议换成可替换的 provider。
11. **`wb_pricing` 的内嵌 2556 行快照**：出厂价快照随版本走，Rust 侧应改为外部 `pricing.json` 资源文件，否则每次调价都要发版（这也正是项目自己用 `WB_PRICING_FILE` 解决的）。
12. **`PanelSessions`/`PBKDF2` 面板密码体系**：桌面端没有"看板密码"概念，应改为 OS 级凭据（Keychain/DPAPI）。

### 需要特别注意的边界
- **Anthropic Messages 默认关闭**：`_handle_messages` 在关闭时返回 400，理由是 prompt cache 命中率仅约 11%（issue #367）。桌面端若做 Anthropic 兼容，必须自己解决 `cache_control` 语义，不能假设上游支持。
- **跨区域混轮未实现**：同名模型（如 `deepseek-v4.1-flash`）不跨 intl/cn 混合轮询，两区是独立出口（出站指纹取舍）。
- **`CURRENT_REALM` 是进程级全局**：`active_realm.json` 落盘，`detect_model_realm()` 用硬编码 intl-only/cn-only 名单 + 前缀猜测（`gpt-`, `gemini-`, `minimax-`, `deepseek-v4-pro`），名单会腐化。
- **`/accounts/export` 与 `VOLATILE_FIELDS`/`RUNTIME_ONLY_FIELDS`**：导出默认含密钥，运行时态字段被剥离；合并时若把 `cooldownUntil/lastError` 当持久状态会重犯本项目已修过的 bug（重启后面板显示早已恢复的 429）。
