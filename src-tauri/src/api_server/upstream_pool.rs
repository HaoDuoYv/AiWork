//! 上游组：多端点负载均衡 + 三族退避熔断（合并增强）
//!
//! ## 来源
//! - `workbuddy2api-hub / wb_upstream_pool.py`：加权平滑轮询（nginx SWRR）、
//!   按出口/端点维度的失败阶梯、抖动重试；
//! - `qoder2api-hub / qoder_accounts.py`：soft / breaker / degrade 三族退避
//!   与 `note_success` 全清（但不解 402 停放）的成熟参数。
//!
//! ## 与既有模块的关系
//! `custom_models.rs` 是「一个模型名 → 一个上游」的单源语义；本模块把它扩展为
//! 「一个模型名 → 一组上游」，调度语义保持向后兼容：
//! - 组不存在 → 完全走 `custom_models` 单源（行为不变）；
//! - 组存在 → 按 `strategy` 在**健康端点**间选一个，失败换下一个（预流式回退）。
//!
//! ## 红线（自 hub 报告继承）
//! - 运行时计数（fails / breaker_until / soft_until / cw…）**只存内存不落盘**：
//!   落盘会在重启后把早已自愈的冷却当作生效状态（hub 同名 bug 复盘）；
//! - 组配置（端点/权重/开关）落 kv `upstream_groups`，与模型定义解耦；
//! - 401/403 视为端点凭据错误，直接熔断到 `BREAKER_MAX`（不参与 soft 阶梯）。
//!
//! ## 三族退避（参数与两个 hub 一致）
//! | 族 | 触发 | 公式 | 上限 |
//! |---|---|---|---|
//! | soft | 429 / 首次 5xx | `min(600·2^(n-1), 7200)` | 2h |
//! | breaker | 连续失败达 3 次 | `min(1800·2^(n-3), 21600)` | 6h |
//! | degrade | 连续失败达 5 次 | `min(600·2^(n-5), 7200)` | 2h |
//!
//! `FAIL_WINDOW=600s`：窗口内没有新失败则计数从头算，避免偶发失败累积成永久冷却。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::unified_catalog::canonical_id;

/// kv 键：上游组配置
const KV_GROUPS: &str = "upstream_groups";
/// 连续失败达该值进入 breaker 族
pub const BREAKER_THRESHOLD: u32 = 3;
/// 连续失败达该值进入 degrade 族
pub const DEGRADE_THRESHOLD: u32 = 5;
const SOFT_BASE: i64 = 600;
const SOFT_MAX: i64 = 7200;
const BREAKER_BASE: i64 = 1800;
const BREAKER_MAX: i64 = 21600;
const DEGRADE_BASE: i64 = 600;
const DEGRADE_MAX: i64 = 7200;
/// 失败计数滚动窗口（秒）
pub const FAIL_WINDOW: i64 = 600;
/// 余额停放：402 后停到「次日 UTC+8 10:00」
const BALANCE_HOUR_CST: i64 = 10;

/// 单个上游端点（组内成员）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpstreamEndpoint {
    /// 稳定 id（组内唯一；保存时空值自动生成 `ue-<12hex>`）
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    /// OpenAI 兼容 base（可含 /v1）
    #[serde(default)]
    pub base_url: String,
    /// 端点自带 Key（空则回退模型条目的 Key）
    #[serde(default)]
    pub api_key: String,
    /// 权重（0/1 视作 1；>1 参与平滑加权）
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub note: String,
}

fn default_weight() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

/// 模型级上游组
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamGroup {
    /// 关联模型名（canonical 匹配，与 custom_models.name 对齐）
    pub model: String,
    /// 调度策略：swrr（平滑加权，默认）| priority（按列表顺序取首个健康）
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default)]
    pub endpoints: Vec<UpstreamEndpoint>,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_strategy() -> String {
    "swrr".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpstreamGroupsFile {
    #[serde(default)]
    pub groups: Vec<UpstreamGroup>,
    #[serde(default)]
    pub updated_at: i64,
}

/// 端点运行时状态（**不落盘**）
#[derive(Debug, Clone, Default)]
struct Rt {
    fails: u32,
    last_fail_at: i64,
    soft_streak: u32,
    soft_until: i64,
    breaker_until: i64,
    degrade_count: u32,
    degrade_until: i64,
    /// 余额停放（402）：到点前不参与选号
    balance_until: i64,
    /// SWRR 当前权重
    cw: i64,
    /// 最近一次错误摘要（展示用）
    last_error: String,
}

fn rt_store() -> &'static Mutex<HashMap<String, Rt>> {
    static RT: OnceLock<Mutex<HashMap<String, Rt>>> = OnceLock::new();
    RT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn rt_key(model: &str, endpoint_id: &str) -> String {
    format!("{}|{}", canonical_id(model), endpoint_id)
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 失败类别（决定走哪一族退避）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// 传输失败 / 连接超时 / 5xx
    Transport,
    /// 429 限流
    RateLimited,
    /// 401 / 403 凭据错误（直接长熔断）
    Auth,
    /// 402 余额不足（停放至次日 10:00 CST）
    Balance,
}

impl FailKind {
    /// 从上游状态码推断（非 HTTP 错误传 None → Transport）
    pub fn from_status(status: Option<u16>) -> Self {
        match status {
            Some(401) | Some(403) => FailKind::Auth,
            Some(402) => FailKind::Balance,
            Some(429) => FailKind::RateLimited,
            _ => FailKind::Transport,
        }
    }
}

// ==================== 组配置读写 ====================

/// 读取全部上游组
pub fn load_groups(data_dir: &Path) -> Vec<UpstreamGroup> {
    let file: UpstreamGroupsFile = crate::store::db(data_dir).kv_get(KV_GROUPS);
    file.groups
}

/// 按模型名（canonical）取组
pub fn find_group(data_dir: &Path, model: &str) -> Option<UpstreamGroup> {
    let canonical = canonical_id(model);
    load_groups(data_dir)
        .into_iter()
        .find(|g| canonical_id(&g.model) == canonical)
}

/// 保存/覆盖一个上游组（按 model canonical upsert；端点校验 + id 补全）
pub fn save_group(data_dir: &Path, mut group: UpstreamGroup) -> Result<Vec<UpstreamGroup>, String> {
    group.model = group.model.trim().to_string();
    if group.model.is_empty() {
        return Err("模型名不能为空".into());
    }
    let strategy = group.strategy.trim().to_ascii_lowercase();
    group.strategy = if strategy == "priority" {
        "priority".to_string()
    } else {
        "swrr".to_string()
    };
    let mut seen: Vec<String> = Vec::new();
    let mut eps: Vec<UpstreamEndpoint> = Vec::new();
    for (i, mut e) in group.endpoints.into_iter().enumerate() {
        e.base_url = e.base_url.trim().trim_end_matches('/').to_string();
        e.label = e.label.trim().to_string();
        if e.base_url.is_empty() {
            return Err(format!("第 {} 个端点缺少 base_url", i + 1));
        }
        if !(e.base_url.starts_with("http://") || e.base_url.starts_with("https://")) {
            return Err(format!("端点 {} 的 base_url 必须以 http:// 或 https:// 开头", e.base_url));
        }
        if e.id.trim().is_empty() {
            e.id = format!("ue-{:012x}", uuid::Uuid::new_v4().as_u128() as u64 & 0xffff_ffff_ffff);
        }
        if seen.contains(&e.id) {
            return Err(format!("端点 id 重复：{}", e.id));
        }
        seen.push(e.id.clone());
        if e.weight == 0 {
            e.weight = 1;
        }
        if e.label.is_empty() {
            e.label = e.base_url.clone();
        }
        eps.push(e);
    }
    if eps.is_empty() {
        return Err("上游组至少需要一个端点".into());
    }
    group.endpoints = eps;
    group.updated_at = now_ts();

    let mut file: UpstreamGroupsFile = crate::store::db(data_dir).kv_get(KV_GROUPS);
    let canonical = canonical_id(&group.model);
    match file
        .groups
        .iter_mut()
        .find(|g| canonical_id(&g.model) == canonical)
    {
        Some(slot) => {
            // 旧端点 id 全消失 → 清掉对应运行时状态，避免新端点继承旧冷却
            let old_ids: Vec<String> = slot.endpoints.iter().map(|e| e.id.clone()).collect();
            let new_ids: Vec<String> = group.endpoints.iter().map(|e| e.id.clone()).collect();
            *slot = group.clone();
            drop_rt_missing(&slot.model, &old_ids, &new_ids);
        }
        None => file.groups.push(group.clone()),
    }
    file.updated_at = now_ts();
    let store = crate::store::db(data_dir);
    store.kv_set(KV_GROUPS, &file)?;
    super::config_cache::invalidate(data_dir, "upstream_groups");
    Ok(file.groups)
}

fn drop_rt_missing(model: &str, old_ids: &[String], new_ids: &[String]) {
    let mut rt = rt_store().lock().unwrap_or_else(|e| e.into_inner());
    for id in old_ids {
        if !new_ids.contains(id) {
            rt.remove(&rt_key(model, id));
        }
    }
}

/// 删除上游组；返回是否确有删除
pub fn remove_group(data_dir: &Path, model: &str) -> Result<bool, String> {
    let canonical = canonical_id(model);
    let mut file: UpstreamGroupsFile = crate::store::db(data_dir).kv_get(KV_GROUPS);
    let before = file.groups.len();
    let removed_models: Vec<String> = file
        .groups
        .iter()
        .filter(|g| canonical_id(&g.model) == canonical)
        .map(|g| g.model.clone())
        .collect();
    file.groups.retain(|g| canonical_id(&g.model) != canonical);
    let removed = file.groups.len() != before;
    if removed {
        file.updated_at = now_ts();
        crate::store::db(data_dir).kv_set(KV_GROUPS, &file)?;
        super::config_cache::invalidate(data_dir, "upstream_groups");
        let mut rt = rt_store().lock().unwrap_or_else(|e| e.into_inner());
        for m in removed_models {
            rt.retain(|k, _| !k.starts_with(&format!("{}|", canonical_id(&m))));
        }
    }
    Ok(removed)
}

// ==================== 端点状态 / 选择 ====================

/// 端点当前是否可用（不计权重）
fn available(rt: &Rt, now: i64) -> bool {
    rt.balance_until <= now && rt.soft_until <= now && rt.breaker_until <= now && rt.degrade_until <= now
}

/// 端点健康视图（前端展示）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EndpointHealth {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub enabled: bool,
    pub weight: u32,
    pub healthy: bool,
    /// cooling_until（取各族最大者；0 = 无冷却）
    pub cooling_until: i64,
    pub cooling_reason: String,
    pub fails: u32,
    pub breaker_until: i64,
    pub soft_until: i64,
    pub degrade_until: i64,
    pub balance_until: i64,
    pub last_error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GroupView {
    pub model: String,
    pub strategy: String,
    pub endpoints: Vec<EndpointHealth>,
    /// 组内健康端点数
    pub healthy_count: usize,
    pub updated_at: i64,
}

/// 组视图（含运行时健康；不落盘）
pub fn snapshot(data_dir: &Path) -> Vec<GroupView> {
    let now = now_ts();
    let rt = rt_store().lock().unwrap_or_else(|e| e.into_inner());
    load_groups(data_dir)
        .into_iter()
        .map(|g| {
            let mut endpoints: Vec<EndpointHealth> = Vec::new();
            let mut healthy = 0usize;
            for e in &g.endpoints {
                let st = rt.get(&rt_key(&g.model, &e.id)).cloned().unwrap_or_default();
                let cooling = st
                    .balance_until
                    .max(st.soft_until)
                    .max(st.breaker_until)
                    .max(st.degrade_until);
                let reason = if st.balance_until > now {
                    "余额不足停放"
                } else if st.breaker_until > now {
                    "连续失败熔断"
                } else if st.degrade_until > now {
                    "持续失败降级"
                } else if st.soft_until > now {
                    "限流退避"
                } else {
                    ""
                };
                let ok = e.enabled && available(&st, now);
                if ok {
                    healthy += 1;
                }
                endpoints.push(EndpointHealth {
                    id: e.id.clone(),
                    label: e.label.clone(),
                    base_url: e.base_url.clone(),
                    enabled: e.enabled,
                    weight: e.weight,
                    healthy: ok,
                    cooling_until: if cooling > now { cooling } else { 0 },
                    cooling_reason: reason.to_string(),
                    fails: st.fails,
                    breaker_until: st.breaker_until,
                    soft_until: st.soft_until,
                    degrade_until: st.degrade_until,
                    balance_until: st.balance_until,
                    last_error: st.last_error.clone(),
                });
            }
            GroupView {
                model: g.model.clone(),
                strategy: g.strategy.clone(),
                healthy_count: healthy,
                endpoints,
                updated_at: g.updated_at,
            }
        })
        .collect()
}

/// 候选端点（已启用且不在冷却期），按策略排序
pub fn candidates(data_dir: &Path, model: &str) -> Vec<UpstreamEndpoint> {
    let Some(group) = find_group(data_dir, model) else {
        return Vec::new();
    };
    let now = now_ts();
    let rt = rt_store().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<UpstreamEndpoint> = group
        .endpoints
        .iter()
        .filter(|e| e.enabled)
        .filter(|e| {
            rt.get(&rt_key(&group.model, &e.id))
                .map(|st| available(st, now))
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    if group.strategy == "priority" {
        // 保持用户列表顺序（priority 语义）
        out.sort_by_key(|e| {
            group
                .endpoints
                .iter()
                .position(|x| x.id == e.id)
                .unwrap_or(usize::MAX)
        });
    }
    out
}

/// 平滑加权轮询选一个端点（nginx SWRR）。
/// 无健康端点 → None（调用方回落单源或报错）；`strategy=priority` 直接取首个。
pub fn pick(data_dir: &Path, model: &str) -> Option<UpstreamEndpoint> {
    let group = find_group(data_dir, model)?;
    let list = candidates(data_dir, model);
    if list.is_empty() {
        return None;
    }
    if group.strategy == "priority" || list.len() == 1 {
        return list.first().cloned();
    }
    let mut rt = rt_store().lock().unwrap_or_else(|e| e.into_inner());
    let total: i64 = list.iter().map(|e| i64::from(e.weight.max(1))).sum();
    let mut best: Option<(String, i64)> = None;
    for e in &list {
        let key = rt_key(&group.model, &e.id);
        let st = rt.entry(key.clone()).or_default();
        st.cw += i64::from(e.weight.max(1));
        if best.as_ref().map(|(_, cw)| st.cw > *cw).unwrap_or(true) {
            best = Some((key, st.cw));
        }
    }
    let (best_key, _) = best?;
    if let Some(st) = rt.get_mut(&best_key) {
        st.cw -= total;
    }
    let id = best_key.rsplit('|').next().unwrap_or_default().to_string();
    list.into_iter().find(|e| e.id == id)
}

/// 成功：清零计数与三族冷却（**不解除 402 余额停放**——余额要等账单周期）
pub fn record_success(model: &str, endpoint_id: &str) {
    let mut rt = rt_store().lock().unwrap_or_else(|e| e.into_inner());
    let st = rt.entry(rt_key(model, endpoint_id)).or_default();
    st.fails = 0;
    st.soft_streak = 0;
    st.degrade_count = 0;
    st.soft_until = 0;
    st.breaker_until = 0;
    st.degrade_until = 0;
    st.last_error.clear();
}

/// 失败：按族推进退避；返回本次冷却截止时间戳（0 = 未冷却）
pub fn record_failure(model: &str, endpoint_id: &str, kind: FailKind, detail: &str) -> i64 {
    let now = now_ts();
    let mut rt = rt_store().lock().unwrap_or_else(|e| e.into_inner());
    let st = rt.entry(rt_key(model, endpoint_id)).or_default();
    // 滚动窗口：窗口外的新失败视为新一轮
    if st.last_fail_at == 0 || now - st.last_fail_at > FAIL_WINDOW {
        st.fails = 0;
        st.soft_streak = 0;
        st.degrade_count = 0;
    }
    st.fails = st.fails.saturating_add(1);
    st.last_fail_at = now;
    if !detail.is_empty() {
        st.last_error = detail.chars().take(200).collect();
    }
    match kind {
        FailKind::Auth => {
            // 凭据错误：直接压到最长熔断，避免反复打爆上游
            st.breaker_until = now + BREAKER_MAX;
        }
        FailKind::Balance => {
            st.balance_until = next_cst_ten(now);
        }
        FailKind::RateLimited | FailKind::Transport => {
            st.soft_streak = st.soft_streak.saturating_add(1);
            let soft = (SOFT_BASE
                * pow2(st.soft_streak.saturating_sub(1)))
            .min(SOFT_MAX);
            st.soft_until = now + soft;
            if st.fails >= BREAKER_THRESHOLD {
                let breaker = (BREAKER_BASE
                    * pow2(st.fails.saturating_sub(BREAKER_THRESHOLD)))
                .min(BREAKER_MAX);
                st.breaker_until = now + breaker;
            }
            if st.fails >= DEGRADE_THRESHOLD {
                st.degrade_count = st.degrade_count.saturating_add(1);
                let degrade = (DEGRADE_BASE
                    * pow2(st.degrade_count.saturating_sub(1)))
                .min(DEGRADE_MAX);
                st.degrade_until = now + degrade;
            }
        }
    }
    st.balance_until
        .max(st.soft_until)
        .max(st.breaker_until)
        .max(st.degrade_until)
        .max(0)
}

/// 2^n（饱和到 2^20，防溢出；配合上限 clamp 足够）
fn pow2(n: u32) -> i64 {
    let n = n.min(20);
    1i64 << n
}

/// 下一个 UTC+8 10:00 的时间戳（已过 10:00 则顺延到明天）
fn next_cst_ten(now: i64) -> i64 {
    let cst = now + 8 * 3600;
    let day_start = cst - cst.rem_euclid(86_400);
    let mut target = day_start + BALANCE_HOUR_CST * 3600;
    if target <= cst {
        target += 86_400;
    }
    target - 8 * 3600
}

/// 清空所有运行时状态（测试 / 网关重启 / 用户手动「重置健康状态」）
pub fn reset_runtime() {
    rt_store().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aw_upool_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn group(model: &str, eps: Vec<(&str, u32)>) -> UpstreamGroup {
        UpstreamGroup {
            model: model.to_string(),
            strategy: "swrr".to_string(),
            endpoints: eps
                .into_iter()
                .enumerate()
                .map(|(i, (url, w))| UpstreamEndpoint {
                    id: format!("ep{i}"),
                    label: format!("ep{i}"),
                    base_url: url.to_string(),
                    api_key: String::new(),
                    weight: w,
                    enabled: true,
                    note: String::new(),
                })
                .collect(),
            updated_at: 0,
        }
    }

    #[test]
    fn swrr_distributes_by_weight() {
        let dir = fixture();
        save_group(&dir, group("m1", vec![("https://a", 3), ("https://b", 1)])).unwrap();
        let mut counts = HashMap::new();
        for _ in 0..8 {
            let e = pick(&dir, "m1").unwrap();
            *counts.entry(e.id.clone()).or_insert(0) += 1;
        }
        assert_eq!(counts.get("ep0"), Some(&6), "权重 3:1 → 8 次应 6:2");
        assert_eq!(counts.get("ep1"), Some(&2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn breaker_opens_after_three_failures() {
        let dir = fixture();
        save_group(&dir, group("m2", vec![("https://a", 1), ("https://b", 1)])).unwrap();
        for _ in 0..BREAKER_THRESHOLD {
            record_failure("m2", "ep0", FailKind::Transport, "boom");
        }
        let cands = candidates(&dir, "m2");
        assert_eq!(cands.len(), 1, "熔断端点数应退出候选");
        assert_eq!(cands[0].id, "ep1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn success_clears_ladder_but_not_balance_parking() {
        let dir = fixture();
        save_group(&dir, group("m3", vec![("https://a", 1)])).unwrap();
        record_failure("m3", "ep0", FailKind::RateLimited, "429");
        let until = record_failure("m3", "ep0", FailKind::Balance, "402");
        assert!(until > now_ts(), "402 应停放");
        record_success("m3", "ep0");
        // 停放仍在：candidates 为空
        assert!(pick(&dir, "m3").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_failure_jumps_to_max_breaker() {
        let dir = fixture();
        save_group(&dir, group("m4", vec![("https://a", 1), ("https://b", 1)])).unwrap();
        record_failure("m4", "ep0", FailKind::Auth, "401");
        let snap = snapshot(&dir);
        let ep = snap[0].endpoints.iter().find(|e| e.id == "ep0").unwrap();
        assert!(!ep.healthy);
        assert!(ep.breaker_until - now_ts() > BREAKER_MAX - 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn priority_strategy_keeps_list_order() {
        let dir = fixture();
        let mut g = group("m5", vec![("https://a", 1), ("https://b", 9)]);
        g.strategy = "priority".to_string();
        save_group(&dir, g).unwrap();
        for _ in 0..3 {
            assert_eq!(pick(&dir, "m5").unwrap().id, "ep0");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_validates_and_dedupes() {
        let dir = fixture();
        // 空模型名
        assert!(save_group(&dir, group("", vec![("https://a", 1)])).is_err());
        // 非 http
        assert!(save_group(&dir, group("m6", vec![("ftp://a", 1)])).is_err());
        // 缺端点
        assert!(save_group(&dir, group("m6", vec![])).is_err());
        // 正常保存 → 覆盖同 canonical
        save_group(&dir, group("Model-X", vec![("https://a", 1)])).unwrap();
        save_group(&dir, group("model-x", vec![("https://b", 2), ("https://c", 1)])).unwrap();
        let groups = load_groups(&dir);
        assert_eq!(groups.len(), 1, "同名 canonical 必须覆盖而非追加");
        assert_eq!(groups[0].endpoints.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn next_cst_ten_is_future_and_aligned() {
        let now = 1_700_000_000; // 2023-11-14T22:13:20Z
        let t = next_cst_ten(now);
        assert!(t > now);
        let cst = t + 8 * 3600;
        assert_eq!(cst.rem_euclid(86_400), 10 * 3600, "必须落在 UTC+8 10:00");
        let _ = now;
    }

    #[test]
    fn fail_window_resets_counter() {
        let dir = fixture();
        save_group(&dir, group("m7", vec![("https://a", 1)])).unwrap();
        record_failure("m7", "ep0", FailKind::Transport, "x");
        // 手工把 last_fail_at 拨到窗口外，模拟时间流逝
        {
            let mut rt = rt_store().lock().unwrap();
            if let Some(st) = rt.get_mut(&rt_key("m7", "ep0")) {
                st.last_fail_at = now_ts() - FAIL_WINDOW - 5;
            }
        }
        record_failure("m7", "ep0", FailKind::Transport, "x");
        let snap = snapshot(&dir);
        assert_eq!(snap[0].endpoints[0].fails, 1, "窗口外失败应重新计数");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
