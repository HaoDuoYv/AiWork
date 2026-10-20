# Qoder 双区升级为「双池」（方案 C）实施设计

## 实施进度（2026-10-18）

| 步骤 | 状态 | 落点 |
|---|---|---|
| 1. 双区纯函数（`pool_tag` / `realm_of_pool` / `normalize_model_id` / `uid_with_pool` / `split_pooled_uid` / `resolve_pool_tag`） | ✅ + 8 单测 | `tasks/qoder_realm.rs` |
| 2. `ApiSharedState` 双池实例 + `qoder_pool_for(tag)` | ✅ | `api_server/mod.rs`、`dispatch.rs`、`routes.rs`、`commands/api_server.rs` |
| 3. 按账号 `realm` 分区同步 | ✅ | `apply_pool_snapshot`（`is_intl(uid)` 分流两池） |
| 4. `dispatch` 三级定区 + 设置项 | ✅ | `dispatch.rs`、`Settings.qoder_default_realm` / `qoder_intl_enabled` |
| 5. `qoder_route` 同区解析/选号/记账/对冲 | ✅ | 41 处池访问改用 `qoder_pool_for`；目录按区域 `resolve`；`entry_is_global_only` 对国际版放行；`race_qoder_first_byte` / `QoderHedgeLease` 带 `pool_tag` |
| 6. Key 绑区贯通 | ✅ + 单测 | `parse_bind_pool` / `pool_realm_of` / `resolve_pool_tag`（前缀 > 绑区 > 默认）；Key UI 加「Qoder CN / Qoder INTL」 |
| 7. `/v1/models` 区域可见性 | ✅ | Global 专属模型追加 `qoder-intl/<id>`（`realm`/`source_pool`/`alias_of`） |
| 8. UI 设置与徽标 | ✅ | `QoderSettings`「区域与资源池」、`QoderAccounts` realm 徽标、`types.ts`、`QoderAccountView.realm` |
| 9. 验证与打包 | ✅ | `cargo test` 全量通过、`tsc` 0 error、NSIS/MSI/便携版重建 |

### 仍未覆盖（下一代）

- **真机双区验证**：需在同时登录两区的机器上核对 `api.log` 的 `route=` 与 `/v1/models` 的 `qoder-intl/…`；
- ~~Qoder 签到/积分/PAT 换令的区域接线~~ **已完成**：`checkin::urls_for(realm)`、`claim_one` 走 `urls.campaigns`、`credits::fetch_usage_balance(realm)`、`fetch_account` 的 usage 与逐包域名、`exchange_job_token(realm)` 全部按账号区域取值；
- ~~IDE 扫描双区导入~~ **已完成（2026-10-18 收口）**：删除国际版跳过分支后仍有一版「命中即停、必须扫两次」的过渡实现；现已改为**一次扫描遍历全部候选目录**逐区导入，账号身份键统一为 `(realm, uid)`（`qoder_realm::same_identity`），存量 `realm` 空值首次识别补全，扫描结果回传 `accounts[]` 逐条区域/通道/新增或更新；PAT 导入另加 userinfo 双探自动定区（CN → INTL）。
- **签到兜底盲发（blind_round）**：仍固定国内版基址（该函数只接 token，未带 realm）；主 claim 路径已按区，仅「主列表整批过期 → 盲发兜底」这条少见分支对国际版账号会打错域；
- **WorkBuddy 区域**：按决策保持现状（单池 + 按域分流），不进入本方案。

---

> 目标：**区域 = 池维度**，跨区绝不混轮；「智能调配」只发生在同区之内。
> 前置已完成：`tasks/qoder_realm.rs`（双区单一事实源）、`QoderAccount.realm` / `QoderCreds.realm`、
> 对话上游 / openapi / 目录同步已按 `creds.realm` 取端点（`cargo check` + `cargo test qoder` 通过）。


## 1. 不变量（先立规矩，再改代码）

1. 一个账号只属于一个区（`QoderAccount.realm`，空 = cn）；账号不跨区复用。
2. 一个请求只在一个区内完成：选号、对冲、轮换、重试、冷却都不得跨区。
3. 模型名解析必须与目标区一致：`upstreamKey`/倍率/上下文取该区目录。
4. 区域是显式选择（模型命名或 Key 绑定），**不是**按负载/价格动态漂移。
5. 存量零迁移：无 `realm` 的账号与无池前缀的 Key 行为与今日一致。

## 2. 池命名与数据结构

| 层 | 今日 | 升级后 |
|---|---|---|
| 池标签 | `trae` / `buddy` / `qoder` / `custom` | 新增 `qoder_cn` / `qoder_intl`；`qoder` 保留为「两区并集」的兼容别名 |
| `ApiSharedState` | `qoder_pool: ApiPool` | `qoder_cn_pool` + `qoder_intl_pool`（各自冷却/在途/粘性） |
| Key 绑定 | `bind_pool: ""/trae/buddy/qoder` | 追加 `qoder_cn` / `qoder_intl`（`qoder` = 跟随模型区域） |
| 模型 ID | `glm-5.3`（两区同名） | 规范化 `qoder/<model>`（cn）与 `qoder-intl/<model>`（intl）；裸名走「按区并集 + 显式区域优先」兼容 |
| 冷却键 | `model` | `realm:model`（防同名跨区污染） |
| 粘性 | 会话 → 账号 | 会话 → (池标签, 账号)，池标签含区域 |

## 3. 请求路径改造（按执行顺序）

1. **`dispatch`**：模型 → 池判定改为「先定区域」：
   - 模型带 `qoder-intl/` 前缀 → `qoder_intl`；带 `qoder/` → `qoder_cn`；
   - 裸名：两区目录都命中时按 Key 的 `bind_pool` 决定；未绑定则按 `settings.qoder_default_realm`（默认 cn）；
2. **`qoder_route`**：`resolve(model, region_of_pool)`（不再恒 CN）；删除 `entry_is_global_only` 硬拦，改为「该区目录无此模型 → 本池跳过，换同区账号/返回明确 404」；
3. **选号 `pool.rs`**：入池即按池标签隔离（现有 `ApiPool` 天然按池实例隔离，只需两份实例 + 各自的 uid 前缀 `qoder_cn:` / `qoder_intl:`）；
4. **对冲/轮换**：候选集限定同池实例（`creds2` 必为同区账号），`upstreamKey` 用同区目录重解析；
5. **冷却**：`model_cooldowns` 键 `format!("{pool}:{model}")`；
6. **客户端约束**：`api_keys::PoolConstraint` 增加两区；`allowed_accounts` 前缀跟随池标签。

## 4. `/v1/models` 输出

- 每个 Qoder 条目输出 `id`（规范化名）+ `realm`（`cn|intl`）+ `rate/context_length/efforts`；
- 同区同名去重（以该区目录为准），**跨区同名保留两条**（id 前缀不同），避免"看得见但打不到"；
- 可选别名：裸名映射到 `settings.qoder_default_realm` 那条，并在 `display` 里标注「（国内版）/（国际版）」。

## 5. 设置与 UI

| 位置 | 新增 |
|---|---|
| Qoder 环境配置 | 「默认区域」（cn/intl）、「启用国际版池」开关（关闭时 `qoder_intl` 池不参调） |
| Qoder 账号页 | 每行 `realm` 徽标；导入 PAT / IDE 扫描时显示命中区域（扫描已双候选，导入侧按目录区落库） |
| 资源调度 · Key | `bind_pool` 下拉追加「Qoder 国内 / Qoder 国际」 |
| 枢纽增强 · 上游组 | 组条目展示所属区域（custom 上游不受影响） |

## 6. 迁移

- 存量 `qoder` 账号：`realm` 空 → 归入 `qoder_cn` 池（日志注明「存量账号默认国内版」）；
- 存量 Key `bind_pool="qoder"` → 语义变更为「跟随模型区域」（仍等价于旧行为：只有 cn 池有账号时只会命中 cn）；
- 存量快照/导出文件：`realm` 缺省即 cn，导入不报错；
- 目录缓存：`QoderRegion` 已有 CN/Global 两份，`qoder_cn` 池用 `Cn`、`qoder_intl` 用 `Global`。

## 7. 实施顺序（8 步，每步可独立编译 + 单测）

1. `qoder_realm`：加 `pool_tag(realm)` / `realm_of_pool(tag)` / `normalize_model_id()`（纯函数 + 单测）；
2. `state/api_server`：`qoder_pool` → `qoder_cn_pool` + `qoder_intl_pool`（构造/注册/关闭/健康快照同步改）；
3. `pool.rs`：uid 前缀 `qoder_cn:`/`qoder_intl:`；进池/入册按前缀分派；
4. `dispatch`：模型 → 池（步骤 3.1 的规则）+ `qoder_default_realm` 设置项；
5. `qoder_route`：按池取 region 解析目录、去硬拦、同区轮换/对冲；
6. `api_keys`：`bind_pool` 两区 + `PoolConstraint` 贯通；
7. `/v1/models`：`realm` 字段 + 规范化 id + 裸名别名；
8. UI：Qoder 设置默认区域/启用开关、账号页 realm 徽标、Key 绑定下拉。

## 8. 验收

- 单测：`normalize_model_id`、池标签映射、`realm:model` 冷却隔离、Key 绑区后不跨区（构造两区假账号断言选号集合）；
- 集成：CN Key 打 `qoder-intl/*` → 400 明确文案；国际版池关闭时 → 404 且提示开关；
- 真机：两区账号各一，同一 Key 连续请求分别命中对应区（`api.log` 的 `route=` 显示 `qoder_cn` / `qoder_intl`）；
- 冒烟：`/v1/models` 两区同名模型各一条，`realm` 正确；
- 回滚：`qoder_intl` 关闭 = 行为退化为方案 B（单池 cn），无需数据回滚。

## 9. 风险

| 风险 | 处置 |
|---|---|
| `ApiSharedState` 字段改名波及大量构造/测试 | 保留 `qoder_pool` 作为 cn 的别名方法，先内部双实例，再逐步替换调用点 |
| uid 前缀改动影响 Key 白名单/组织索引 | `org_index` 与 `api_keys` 的 uid 归一函数集中到 `split_pool_tagged`，扩展而非重写 |
| 客户端缓存了裸模型名 | 裸名默认映射 `qoder_default_realm`，并在 `/v1/models` 同时给规范化 id，避免一次性断裂 |
| 双池使「临时无可用账号」概率上升 | 两区池分别参与跨池回退；池级健康徽标可见 |
