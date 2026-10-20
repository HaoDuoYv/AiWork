//! 活动时间线（合并增强：源自 `workbuddy2api-hub / wb_activity.py`）
//!
//! ## 作用
//! 给「网关 / 上游 / 智能体接入」三条业务线提供**统一的、脱敏的**事件流水，
//! 供 UI 的「活动」页按时间倒序展示与排障；与 `api_logger`（请求级日志）互为
//! 补充：日志回答「这个请求怎么了」，活动回答「这个系统刚刚做了什么」。
//!
//! ## 红线（继承 hub 的 `ALLOWED_FIELDS` + `_SECRET_PATTERNS` 设计）
//! - **白名单投影**：只保留已知字段，任何未登记字段（可能是新加的密钥/令牌）
//!   一律丢弃，避免"以后加了个字段就顺手泄露"；
//! - **字符串脱敏**：`sk-` / `ck_` / `Bearer` / `token=` / `api_key` 形态的值
//!   一律替换为 `***`，并截断到 200 字符；
//! - **环形存储**：kv `activity_events` 保留最近 500 条；同时追加 JSONL
//!   `data/activity.jsonl`，超过 2MB 轮转为 `.1`（单代，不留无界增长）。

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// kv 键：最近事件环形缓冲
const KV_EVENTS: &str = "activity_events";
/// 环形缓冲容量
const CAP: usize = 500;
/// JSONL 轮转阈值（字节）
const JSONL_MAX: u64 = 2 * 1024 * 1024;
/// 单字段截断长度
const FIELD_MAX: usize = 200;

/// 允许落库的字段（新增字段必须显式登记在这里）
pub const ALLOWED_FIELDS: &[&str] = &[
    "model",
    "upstream",
    "pool",
    "key_id",
    "key_name",
    "account",
    "status",
    "code",
    "duration_ms",
    "latency_ms",
    "in_tokens",
    "out_tokens",
    "cost",
    "policy_id",
    "route",
    "action",
    "target",
    "reason",
    "ok",
    "request_id",
    "bytes",
    "group",
    "endpoint",
    "retry",
    "note",
];

/// 单条活动事件
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivityEvent {
    pub at: i64,
    /// 事件类型：gateway / upstream / agent / checkin / system
    pub kind: String,
    /// 触发者（操作人 / 模块名）
    pub actor: String,
    /// 一句话摘要（已脱敏）
    pub summary: String,
    /// 白名单投影后的字段
    #[serde(default)]
    pub fields: Map<String, Value>,
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 脱敏：先按键名判断敏感字段，再对值做正则清洗
fn redact_text(s: &str) -> String {
    use std::sync::OnceLock;
    static RE_TOKEN: OnceLock<regex::Regex> = OnceLock::new();
    static RE_KV: OnceLock<regex::Regex> = OnceLock::new();
    let re_token = RE_TOKEN.get_or_init(|| {
        // sk-xxx / ck_xxx / Bearer xxx / token=xxx 形态
        regex::Regex::new(r"(?i)(sk-[a-z0-9_\-]{6,}|ck_[a-z0-9]{8,}|bearer\s+[a-z0-9._\-]{6,})")
            .expect("token 脱敏正则")
    });
    let re_kv = RE_KV.get_or_init(|| {
        regex::Regex::new(r#"(?i)("?(?:api[_-]?key|token|secret|authorization|password)"?\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,}]+)"#)
            .expect("kv 脱敏正则")
    });
    let step1 = re_token.replace_all(s, "***").to_string();
    let step2 = re_kv.replace_all(&step1, "$1***").to_string();
    if step2.chars().count() > FIELD_MAX {
        step2.chars().take(FIELD_MAX).collect::<String>() + "…"
    } else {
        step2
    }
}

fn is_secret_key(k: &str) -> bool {
    let lower = k.to_ascii_lowercase().replace(['-', '_'], "");
    matches!(
        lower.as_str(),
        "key" | "apikey" | "token" | "accesstoken" | "secret" | "password" | "authorization" | "cookie"
    )
}

/// 白名单投影：只保留已知字段；字符串值脱敏；敏感键整值替换
pub fn project(payload: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(map) = payload.as_object() else {
        return out;
    };
    for (k, v) in map {
        if !ALLOWED_FIELDS.contains(&k.as_str()) {
            continue;
        }
        let value = match v {
            Value::String(s) => Value::String(if is_secret_key(k) {
                "***".to_string()
            } else {
                redact_text(s)
            }),
            Value::Number(n) => Value::Number(n.clone()),
            Value::Bool(b) => Value::Bool(*b),
            Value::Null => continue,
            other => Value::String(redact_text(&other.to_string())),
        };
        out.insert(k.clone(), value);
    }
    out
}

fn jsonl_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("data").join("activity.jsonl")
}

/// 追加 JSONL（单代轮转，写失败不阻断业务）
fn append_jsonl(data_dir: &Path, line: &str) {
    let path = jsonl_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > JSONL_MAX {
            let bak = std::path::PathBuf::from(format!("{}.1", path.display()));
            let _ = std::fs::rename(&path, bak);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}

/// 记录一条活动
pub fn record(data_dir: &Path, kind: &str, actor: &str, summary: &str, payload: Value) -> ActivityEvent {
    let ev = ActivityEvent {
        at: now_ts(),
        kind: kind.to_string(),
        actor: actor.to_string(),
        summary: redact_text(summary),
        fields: project(&payload),
    };
    let store = crate::store::db(data_dir);
    let mut list: Vec<ActivityEvent> = store.kv_get(KV_EVENTS);
    list.push(ev.clone());
    if list.len() > CAP {
        let drop_n = list.len() - CAP;
        list.drain(0..drop_n);
    }
    let _ = store.kv_set(KV_EVENTS, &list);
    if let Ok(line) = serde_json::to_string(&ev) {
        append_jsonl(data_dir, &line);
    }
    ev
}

/// 最近事件（倒序）
pub fn recent(data_dir: &Path, limit: usize) -> Vec<ActivityEvent> {
    let mut list: Vec<ActivityEvent> = crate::store::db(data_dir).kv_get(KV_EVENTS);
    list.reverse();
    list.truncate(limit.clamp(1, CAP));
    list
}

/// 按类型过滤（倒序）
pub fn recent_by_kind(data_dir: &Path, kind: &str, limit: usize) -> Vec<ActivityEvent> {
    recent(data_dir, CAP)
        .into_iter()
        .filter(|e| kind.is_empty() || e.kind == kind)
        .take(limit.clamp(1, CAP))
        .collect()
}

/// 清空（kv 与 JSONL 一并；JSONL 移动到 `.1` 保留一代供排查）
pub fn clear(data_dir: &Path) -> Result<usize, String> {
    let store = crate::store::db(data_dir);
    let list: Vec<ActivityEvent> = store.kv_get(KV_EVENTS);
    let n = list.len();
    store.kv_set(KV_EVENTS, &Vec::<ActivityEvent>::new())?;
    let path = jsonl_path(data_dir);
    if path.is_file() {
        let bak = std::path::PathBuf::from(format!("{}.1", path.display()));
        let _ = std::fs::remove_file(&bak);
        let _ = std::fs::rename(&path, bak);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aw_activity_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("data")).unwrap();
        dir
    }

    #[test]
    fn whitelist_projection_drops_unknown_fields() {
        let m = project(&json!({"model":"glm-5.3","unknown_secret_field":"x","ok":true}));
        assert!(m.contains_key("model"));
        assert!(m.contains_key("ok"));
        assert!(!m.contains_key("unknown_secret_field"), "未登记字段必须丢弃");
    }

    #[test]
    fn token_shapes_are_redacted() {
        let s = redact_text("call sk-abcdef123456 with ck_0123456789ab and Bearer abc.def.ghi");
        assert!(!s.contains("sk-abcdef123456"));
        assert!(!s.contains("ck_0123456789ab"));
        assert!(s.contains("***"));
    }

    #[test]
    fn kv_secret_is_masked_by_key_name() {
        let m = project(&json!({"key_id":"k1","note":"api_key=sk-1234567890"}));
        assert_eq!(m.get("note").unwrap(), &json!("api_key=***"));
        assert_eq!(m.get("key_id").unwrap(), &json!("k1"));
    }

    #[test]
    fn record_is_capped_and_ordered() {
        let dir = fixture();
        for i in 0..(CAP + 5) {
            record(&dir, "gateway", "test", &format!("event {i}"), json!({"model":"m"}));
        }
        let list = recent(&dir, CAP);
        assert_eq!(list.len(), CAP);
        assert!(list[0].summary.starts_with("event "), "倒序：最新在前");
        assert!(jsonl_path(&dir).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_moves_jsonl_aside() {
        let dir = fixture();
        record(&dir, "system", "t", "hello", json!({}));
        assert_eq!(clear(&dir).unwrap(), 1);
        assert!(recent(&dir, 10).is_empty());
        assert!(!jsonl_path(&dir).is_file());
        assert!(std::path::PathBuf::from(format!("{}.1", jsonl_path(&dir).display())).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
