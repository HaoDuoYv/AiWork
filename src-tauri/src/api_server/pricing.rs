//! 计价与成本核算（合并增强：源自 `workbuddy2api-hub / wb_pricing.py`）
//!
//! ## 为什么需要
//! 原项目只统计「请求数 / token 数」（`api_server/usage.rs`），看不到**花掉多少钱**；
//! 自定义模型与上游组尤其如此。本模块补齐「策略 → 记账 → 汇总」三件套：
//!
//! 1. **策略（policy）**：按模型维护单价，支持分档（bands）；
//! 2. **记账（record）**：每请求按 token 用量算成本，写入环形记录；
//! 3. **汇总（summary）**：按日 / 按模型聚合成看板数据。
//!
//! ## 关键设计（与 hub 一致）
//! - **内容寻址 id**：`policy_id = sha256(model|currency|flat_in|flat_out|bands|usd_cny)`
//!   取前 12 位。调价即产生新 id，历史记录只存 id 引用 → 改价不会篡改历史账；
//! - **分档命中「覆盖用量的最小上限档」**：bands 按 `upto_input` 升序，`upto_input = 0`
//!   表示无上限兜底档（必须放最后）；常规请求落最小可覆盖档，超限才落到兜底档；
//! - **未定价可见**：`unpriced` 明确列出「有请求但没有策略」的模型，避免静默 0 成本。
//!
//! ## 落盘
//! kv `cost_policies`（策略表）、`cost_records`（明细，cap 5000）、
//! `cost_daily`（按日×模型聚合）。均为 kv 文档，不新增行表（`schema.rs` 白名单外）。

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const KV_POLICIES: &str = "cost_policies";
const KV_RECORDS: &str = "cost_records";
const KV_DAILY: &str = "cost_daily";
/// 明细保留条数（环形）
const RECORD_CAP: usize = 5000;
/// 按日聚合保留天数
const DAILY_CAP: usize = 400;

/// 分档单价（每 100 万 token）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CostBand {
    /// 输入 token 上限（0 = 无上限，必须排在最后）
    #[serde(default)]
    pub upto_input: u64,
    /// 输入单价（每 1M token）
    #[serde(default)]
    pub price_in: f64,
    /// 输出单价（每 1M token）
    #[serde(default)]
    pub price_out: f64,
}

/// 计价策略
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CostPolicy {
    /// 内容寻址 id（保存时重算，前端传入值被忽略）
    #[serde(default)]
    pub id: String,
    /// 模型名（canonical 匹配）
    #[serde(default)]
    pub model: String,
    /// 币种（默认 CNY）
    #[serde(default = "default_currency")]
    pub currency: String,
    /// 无档位时的输入单价（每 1M token）
    #[serde(default)]
    pub flat_in: f64,
    /// 无档位时的输出单价（每 1M token）
    #[serde(default)]
    pub flat_out: f64,
    /// 分档（升序；空 = 用 flat_*）
    #[serde(default)]
    pub bands: Vec<CostBand>,
    /// USD→CNY 折算率（策略里固化，避免汇率漂移改写历史）
    #[serde(default = "default_usd_cny")]
    pub usd_cny: f64,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub created_at: i64,
}

fn default_currency() -> String {
    "CNY".to_string()
}

fn default_usd_cny() -> f64 {
    7.2
}

impl Default for CostPolicy {
    fn default() -> Self {
        Self {
            id: String::new(),
            model: String::new(),
            currency: default_currency(),
            flat_in: 0.0,
            flat_out: 0.0,
            bands: Vec::new(),
            usd_cny: default_usd_cny(),
            note: String::new(),
            created_at: 0,
        }
    }
}

/// 单条成本明细
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostRecord {
    pub at: i64,
    pub model: String,
    pub policy_id: String,
    pub in_tokens: u64,
    pub out_tokens: u64,
    /// 本请求成本（policy.currency 单位）
    pub cost: f64,
    #[serde(default)]
    pub key_id: String,
}

/// 按日×模型聚合
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostDaily {
    pub date: String,
    pub model: String,
    pub requests: u64,
    pub in_tokens: u64,
    pub out_tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostSummary {
    /// 选定时段总成本
    pub total_cost: f64,
    pub total_requests: u64,
    pub total_in_tokens: u64,
    pub total_out_tokens: u64,
    pub currency: String,
    pub days: Vec<CostDaily>,
    /// 有请求但无策略的模型（静默 0 成本需显式暴露）
    pub unpriced: Vec<String>,
    /// 策略条数
    pub policy_count: usize,
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 内容寻址：策略内容的稳定指纹（f64 用定点 4 位缩放，避免浮点文本抖动）
pub fn policy_id_for(p: &CostPolicy) -> String {
    let mut bands: Vec<CostBand> = p.bands.clone();
    bands.sort_by_key(|b| b.upto_input);
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}",
        p.model.trim().to_ascii_lowercase(),
        p.currency.trim().to_ascii_uppercase(),
        fix4(p.flat_in),
        fix4(p.flat_out),
        bands
            .iter()
            .map(|b| format!("{}:{}:{}", b.upto_input, fix4(b.price_in), fix4(b.price_out)))
            .collect::<Vec<_>>()
            .join(","),
        fix4(p.usd_cny)
    );
    let mut h = Sha256::new();
    h.update(canonical.as_bytes());
    let hex = format!("{:x}", h.finalize());
    hex.chars().take(12).collect()
}

fn fix4(v: f64) -> String {
    format!("{:.4}", if v.is_finite() { v } else { 0.0 })
}

/// 读取全部策略
pub fn load_policies(data_dir: &Path) -> Vec<CostPolicy> {
    crate::store::db(data_dir).kv_get(KV_POLICIES)
}

/// 保存策略（按 id 内容寻址去重；同模型允许多条不同价位的策略，取最新创建者）
pub fn upsert_policy(data_dir: &Path, mut policy: CostPolicy) -> Result<CostPolicy, String> {
    policy.model = policy.model.trim().to_string();
    if policy.model.is_empty() {
        return Err("模型名不能为空".into());
    }
    if !policy.flat_in.is_finite() || !policy.flat_out.is_finite() {
        return Err("单价必须是有限数值".into());
    }
    if policy.flat_in < 0.0 || policy.flat_out < 0.0 {
        return Err("单价不能为负".into());
    }
    for b in &policy.bands {
        if !b.price_in.is_finite() || !b.price_out.is_finite() || b.price_in < 0.0 || b.price_out < 0.0 {
            return Err("分档单价必须是非负有限数值".into());
        }
    }
    policy.bands.sort_by_key(|b| b.upto_input);
    if let Some(pos) = policy.bands.iter().position(|b| b.upto_input == 0) {
        if pos != policy.bands.len() - 1 {
            return Err("无上限档（upto_input=0）必须排在最后".into());
        }
    }
    if !policy.usd_cny.is_finite() || policy.usd_cny <= 0.0 {
        policy.usd_cny = default_usd_cny();
    }
    policy.currency = if policy.currency.trim().is_empty() {
        default_currency()
    } else {
        policy.currency.trim().to_ascii_uppercase()
    };
    policy.id = policy_id_for(&policy);
    let mut list = load_policies(data_dir);
    if let Some(slot) = list.iter().find(|p| p.id == policy.id) {
        // 同内容 → 幂等更新备注/创建时间不动
        let mut merged = slot.clone();
        merged.note = policy.note.clone();
        let out = merged.clone();
        if let Some(pos) = list.iter().position(|p| p.id == out.id) {
            list[pos] = out.clone();
        }
        crate::store::db(data_dir).kv_set(KV_POLICIES, &list)?;
        return Ok(out);
    }
    policy.created_at = now_ts();
    let out = policy.clone();
    list.push(policy);
    // 同模型只保留最近 8 条价位历史，防无限增长
    let same: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, p)| p.model.eq_ignore_ascii_case(&out.model))
        .map(|(i, _)| i)
        .collect();
    if same.len() > 8 {
        let mut drop_idx: Vec<usize> = same;
        drop_idx.sort_by_key(|i| list[*i].created_at);
        let drop_n = drop_idx.len() - 8;
        for i in drop_idx.into_iter().take(drop_n).rev() {
            list.remove(i);
        }
    }
    crate::store::db(data_dir).kv_set(KV_POLICIES, &list)?;
    Ok(out)
}

/// 删除策略（按 id）
pub fn remove_policy(data_dir: &Path, id: &str) -> Result<bool, String> {
    let mut list = load_policies(data_dir);
    let before = list.len();
    list.retain(|p| p.id != id);
    let removed = list.len() != before;
    if removed {
        crate::store::db(data_dir).kv_set(KV_POLICIES, &list)?;
    }
    Ok(removed)
}

/// 取某模型当前生效策略（同模型多条 → created_at 最新者）
pub fn policy_for(data_dir: &Path, model: &str) -> Option<CostPolicy> {
    let canonical = model.trim().to_ascii_lowercase();
    let mut hits: Vec<CostPolicy> = load_policies(data_dir)
        .into_iter()
        .filter(|p| p.model.trim().to_ascii_lowercase() == canonical)
        .collect();
    hits.sort_by_key(|p| p.created_at);
    hits.pop()
}

/// 单价（每 1M token）：返回 (输入价, 输出价)
pub fn unit_price(p: &CostPolicy, in_tokens: u64) -> (f64, f64) {
    if p.bands.is_empty() {
        return (p.flat_in, p.flat_out);
    }
    // 命中规则：取**覆盖当前用量的最小上限档**（bands 已按 upto_input 升序）；
    // `upto_input = 0` 是无上限兜底档（必须排最后），只有前面所有档都不覆盖时才用它。
    // 反例警戒：若实现成「最后一个 <= 用量的档」，常规小请求会一路落到最贵档。
    for b in &p.bands {
        if b.upto_input > 0 && in_tokens <= b.upto_input {
            return (b.price_in, b.price_out);
        }
    }
    if let Some(b) = p.bands.iter().find(|b| b.upto_input == 0) {
        return (b.price_in, b.price_out);
    }
    (p.flat_in, p.flat_out)
}

/// 单请求成本
pub fn price(p: &CostPolicy, in_tokens: u64, out_tokens: u64) -> f64 {
    let (pin, pout) = unit_price(p, in_tokens);
    let cost = (in_tokens as f64 / 1_000_000.0) * pin + (out_tokens as f64 / 1_000_000.0) * pout;
    if cost.is_finite() && cost >= 0.0 {
        cost
    } else {
        0.0
    }
}

/// 记账：无策略时返回 None（调用方据此把模型列为「未定价」）
pub fn record(
    data_dir: &Path,
    model: &str,
    key_id: &str,
    in_tokens: u64,
    out_tokens: u64,
) -> Option<CostRecord> {
    if in_tokens == 0 && out_tokens == 0 {
        return None;
    }
    let policy = policy_for(data_dir, model)?;
    let rec = CostRecord {
        at: now_ts(),
        model: model.to_string(),
        policy_id: policy.id.clone(),
        in_tokens,
        out_tokens,
        cost: price(&policy, in_tokens, out_tokens),
        key_id: key_id.to_string(),
    };
    let store = crate::store::db(data_dir);
    let mut records: Vec<CostRecord> = store.kv_get(KV_RECORDS);
    records.push(rec.clone());
    if records.len() > RECORD_CAP {
        let drop_n = records.len() - RECORD_CAP;
        records.drain(0..drop_n);
    }
    let _ = store.kv_set(KV_RECORDS, &records);

    // 按日聚合（date + model 唯一）
    let date = today();
    let mut daily: Vec<CostDaily> = store.kv_get(KV_DAILY);
    match daily
        .iter_mut()
        .find(|d| d.date == date && d.model.eq_ignore_ascii_case(&rec.model))
    {
        Some(d) => {
            d.requests += 1;
            d.in_tokens = d.in_tokens.saturating_add(in_tokens);
            d.out_tokens = d.out_tokens.saturating_add(out_tokens);
            d.cost += rec.cost;
        }
        None => daily.push(CostDaily {
            date: date.clone(),
            model: rec.model.clone(),
            requests: 1,
            in_tokens,
            out_tokens,
            cost: rec.cost,
        }),
    }
    if daily.len() > DAILY_CAP {
        daily.sort_by(|a, b| a.date.cmp(&b.date));
        let drop_n = daily.len() - DAILY_CAP;
        daily.drain(0..drop_n);
    }
    let _ = store.kv_set(KV_DAILY, &daily);
    Some(rec)
}

/// 汇总（最近 `days` 天；0 = 全量）
pub fn summary(data_dir: &Path, days: u32) -> CostSummary {
    let daily: Vec<CostDaily> = crate::store::db(data_dir).kv_get(KV_DAILY);
    let cutoff = if days == 0 {
        String::new()
    } else {
        let d = chrono::Local::now() - chrono::Duration::days(i64::from(days) - 1);
        d.format("%Y-%m-%d").to_string()
    };
    let mut out = CostSummary {
        currency: default_currency(),
        policy_count: load_policies(data_dir).len(),
        ..Default::default()
    };
    for d in daily.iter().filter(|d| cutoff.is_empty() || d.date >= cutoff) {
        out.total_cost += d.cost;
        out.total_requests = out.total_requests.saturating_add(d.requests);
        out.total_in_tokens = out.total_in_tokens.saturating_add(d.in_tokens);
        out.total_out_tokens = out.total_out_tokens.saturating_add(d.out_tokens);
        out.days.push(d.clone());
    }
    out.days.sort_by(|a, b| a.date.cmp(&b.date).then(a.model.cmp(&b.model)));

    // 未定价：明细里出现但当前无策略的模型
    let records: Vec<CostRecord> = crate::store::db(data_dir).kv_get(KV_RECORDS);
    let mut unpriced: Vec<String> = Vec::new();
    for r in &records {
        let missing = policy_for(data_dir, &r.model).is_none();
        if missing && !unpriced.iter().any(|m| m.eq_ignore_ascii_case(&r.model)) {
            unpriced.push(r.model.clone());
        }
    }
    out.unpriced = unpriced;
    out
}

/// 明细（最近 `limit` 条，倒序）
pub fn recent_records(data_dir: &Path, limit: usize) -> Vec<CostRecord> {
    let mut records: Vec<CostRecord> = crate::store::db(data_dir).kv_get(KV_RECORDS);
    records.reverse();
    records.truncate(limit.min(RECORD_CAP));
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aw_pricing_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pol(model: &str, fin: f64, fout: f64) -> CostPolicy {
        CostPolicy {
            model: model.to_string(),
            flat_in: fin,
            flat_out: fout,
            ..Default::default()
        }
    }

    #[test]
    fn policy_id_is_content_addressed() {
        let a = pol("GLM-5.3", 2.0, 8.0);
        let b = pol("glm-5.3", 2.0, 8.0);
        assert_eq!(policy_id_for(&a), policy_id_for(&b), "大小写归一");
        let mut c = b.clone();
        c.flat_out = 9.0;
        assert_ne!(policy_id_for(&a), policy_id_for(&c), "调价必须换 id");
        assert_eq!(policy_id_for(&a).len(), 12);
    }

    #[test]
    fn band_last_hit_wins() {
        let mut p = pol("m", 1.0, 2.0);
        p.bands = vec![
            CostBand { upto_input: 32_000, price_in: 1.0, price_out: 2.0 },
            CostBand { upto_input: 128_000, price_in: 2.0, price_out: 4.0 },
            CostBand { upto_input: 0, price_in: 3.0, price_out: 6.0 },
        ];
        assert_eq!(unit_price(&p, 1_000), (1.0, 2.0));
        assert_eq!(unit_price(&p, 100_000), (2.0, 4.0));
        assert_eq!(unit_price(&p, 1_000_000), (3.0, 6.0), "超上限档命中无上限档");
        // 成本：1M 输入 + 0.5M 输出 @ 3/6
        let cost = price(&p, 1_000_000, 500_000);
        assert!((cost - 6.0).abs() < 1e-9, "3*1 + 6*0.5 = 6，实际 {cost}");
    }

    #[test]
    fn record_and_summary_aggregate() {
        let dir = fixture();
        upsert_policy(&dir, pol("glm-5.3", 1.0, 2.0)).unwrap();
        let r = record(&dir, "glm-5.3", "k1", 1_000_000, 500_000).expect("应记账");
        assert!((r.cost - 2.0).abs() < 1e-9, "1*1 + 0.5*2 = 2，实际 {}", r.cost);
        let s = summary(&dir, 30);
        assert_eq!(s.total_requests, 1);
        assert_eq!(s.days.len(), 1);
        assert!((s.total_cost - 2.0).abs() < 1e-9);
        assert!(s.unpriced.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpriced_models_are_reported() {
        let dir = fixture();
        // 无策略 → 不记账
        assert!(record(&dir, "unknown-model", "k", 100, 100).is_none());
        // 造一条明细再删策略：应被列入 unpriced
        upsert_policy(&dir, pol("m-x", 1.0, 1.0)).unwrap();
        assert!(record(&dir, "m-x", "k", 1000, 1000).is_some());
        let list = load_policies(&dir);
        remove_policy(&dir, &list[0].id).unwrap();
        let s = summary(&dir, 30);
        assert_eq!(s.unpriced, vec!["m-x".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_unbounded_band_not_last() {
        let dir = fixture();
        let mut p = pol("m", 1.0, 1.0);
        p.bands = vec![
            CostBand { upto_input: 0, price_in: 1.0, price_out: 1.0 },
            CostBand { upto_input: 100, price_in: 2.0, price_out: 2.0 },
        ];
        assert!(upsert_policy(&dir, p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn price_history_keeps_latest_per_model() {
        let dir = fixture();
        upsert_policy(&dir, pol("m", 1.0, 1.0)).unwrap();
        upsert_policy(&dir, pol("m", 2.0, 2.0)).unwrap();
        let list = load_policies(&dir);
        assert_eq!(list.len(), 2, "不同价位共 2 条策略");
        // policy_for 取 created_at 最新（同秒内按插入顺序后写者胜）
        let cur = policy_for(&dir, "m").unwrap();
        assert_eq!(cur.flat_in, 2.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
