//! 模型能力目录：四级查找（合并增强：源自 `workbuddy2api-hub / wb_modelsdev.py`）
//!
//! ## 为什么需要
//! 自定义模型 / 上游组里的模型往往没有 `context_length` / `max_tokens`，看板与
//! 长上下文降档都拿不到依据。hub 的做法是「四级查找」，本模块照搬该顺序：
//!
//! 1. **remote**：`https://models.dev/api.json`（缓存 24h，失败不阻断）；
//! 2. **家族兜底**：按模型名前缀推断上下文窗口（本文件 `FAMILY_CONTEXT`）；
//! 3. **本地目录**：调用方传入的既有值（`model.json` / 自定义模型条目）；
//! 4. **默认**：未知上下文按 1M 处理（hub 同口径），未知输出 **省略不编造**。
//!
//! ## 红线
//! - 网络失败/解析失败一律回落下一级，**绝不因目录拉取失败阻断请求**；
//! - 不编造 `max_output`：查不到就返回 `None`（前端显示 `-` 而不是假数据）。

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// kv 键：远程目录缓存
const KV_CACHE: &str = "modelsdev_cache";
/// 缓存有效期（秒）
const TTL: i64 = 24 * 3600;
/// 远程地址
const REMOTE_URL: &str = "https://models.dev/api.json";
/// 缓存条目上限（防异常数据撑爆 kv）
const MAX_ENTRIES: usize = 4000;
/// 未知上下文的默认值（与 hub 一致：1M）
pub const DEFAULT_CONTEXT: u64 = 1_000_000;

/// 单模型能力
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelCapability {
    pub model: String,
    pub context_length: u64,
    /// 未知 → None（**不编造**）
    pub max_output: Option<u64>,
    /// remote | family | local | default
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CacheEntry {
    at: i64,
    map: HashMap<String, (u64, Option<u64>)>,
}

/// 家族兜底表（前缀小写匹配）。数值取各家族公开的常见上下文窗口；
/// 命中多个前缀时取**最长前缀**，避免 `gpt-4` 抢走 `gpt-4.1`。
const FAMILY_CONTEXT: &[(&str, u64)] = &[
    ("claude", 200_000),
    ("gpt-4.1", 1_000_000),
    ("gpt-4o", 128_000),
    ("gpt-4", 128_000),
    ("gpt-5", 400_000),
    ("o3", 200_000),
    ("o4", 200_000),
    ("gemini", 1_000_000),
    ("qwen", 131_072),
    ("kimi", 262_144),
    ("moonshot", 262_144),
    ("deepseek", 131_072),
    ("glm", 128_000),
    ("llama", 131_072),
    ("mistral", 131_072),
    ("mixtral", 32_768),
    ("hunyuan", 131_072),
    ("ernie", 128_000),
    ("doubao", 262_144),
    ("minimax", 200_000),
    ("step", 131_072),
    ("yi", 200_000),
    ("grok", 131_072),
    ("command", 128_000),
    ("nova", 300_000),
];

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 归一化模型名：取最后一段（`openai/gpt-4o` → `gpt-4o`），小写、去版本后缀空格
fn norm(model: &str) -> String {
    let base = model.trim().rsplit('/').next().unwrap_or(model);
    base.trim().to_ascii_lowercase()
}

/// 家族兜底
fn family_lookup(model: &str) -> Option<u64> {
    let m = norm(model);
    FAMILY_CONTEXT
        .iter()
        .filter(|(prefix, _)| m.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, ctx)| *ctx)
}

fn load_cache(data_dir: &Path) -> CacheEntry {
    crate::store::db(data_dir).kv_get(KV_CACHE)
}

/// 远程目录同步（阻塞；调用方放 spawn_blocking）。
/// 返回写入条目数；任何失败都以 `Err` 返回，调用方降级即可。
pub fn sync_remote(data_dir: &Path) -> Result<usize, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(8))
        .timeout_read(std::time::Duration::from_secs(20))
        .build();
    let text = match agent.get(REMOTE_URL).call() {
        Ok(resp) => resp.into_string().map_err(|e| format!("读取目录失败: {e}"))?,
        Err(e) => return Err(format!("拉取 {REMOTE_URL} 失败: {e}")),
    };
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("目录 JSON 解析失败: {e}"))?;
    let mut map: HashMap<String, (u64, Option<u64>)> = HashMap::new();
    // models.dev 形态：{ provider: { models: { id: { limit: { context, output } } } } }
    if let Some(providers) = v.as_object() {
        for (_pid, pv) in providers {
            let Some(models) = pv.get("models").and_then(|m| m.as_object()) else {
                continue;
            };
            for (id, mv) in models {
                let ctx = mv
                    .pointer("/limit/context")
                    .and_then(|c| c.as_u64())
                    .filter(|c| *c > 0);
                let out = mv.pointer("/limit/output").and_then(|c| c.as_u64());
                if let Some(ctx) = ctx {
                    if map.len() < MAX_ENTRIES {
                        map.insert(norm(id), (ctx, out));
                    }
                }
            }
        }
    }
    if map.is_empty() {
        return Err("目录内没有可用条目（结构可能已变化）".to_string());
    }
    let n = map.len();
    crate::store::db(data_dir).kv_set(
        KV_CACHE,
        &CacheEntry {
            at: now_ts(),
            map,
        },
    )?;
    Ok(n)
}

/// 四级查找
pub fn lookup(data_dir: &Path, model: &str, local: Option<(u64, u64)>) -> ModelCapability {
    let mut cap = ModelCapability {
        model: model.to_string(),
        ..Default::default()
    };
    // 1) remote（缓存未过期）
    let cache = load_cache(data_dir);
    if !cache.map.is_empty() && now_ts() - cache.at <= TTL {
        if let Some((ctx, out)) = cache.map.get(&norm(model)) {
            cap.context_length = *ctx;
            cap.max_output = *out;
            cap.source = "remote".to_string();
            return cap;
        }
    }
    // 2) 家族兜底
    if let Some(ctx) = family_lookup(model) {
        cap.context_length = ctx;
        cap.source = "family".to_string();
        // 家族表不含输出上限 → 留 None（不编造）
        if let Some((_lctx, lout)) = local {
            if lout > 0 {
                cap.max_output = Some(lout);
            }
        }
        return cap;
    }
    // 3) 本地目录
    if let Some((lctx, lout)) = local {
        if lctx > 0 {
            cap.context_length = lctx;
            cap.max_output = if lout > 0 { Some(lout) } else { None };
            cap.source = "local".to_string();
            return cap;
        }
    }
    // 4) 默认
    cap.context_length = DEFAULT_CONTEXT;
    cap.source = "default".to_string();
    cap
}

/// 缓存状态（前端展示「上次同步 / 条目数」）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CatalogStatus {
    pub entries: usize,
    pub synced_at: i64,
    pub fresh: bool,
    pub remote_url: String,
    pub ttl_secs: i64,
}

pub fn status(data_dir: &Path) -> CatalogStatus {
    let cache = load_cache(data_dir);
    CatalogStatus {
        entries: cache.map.len(),
        synced_at: cache.at,
        fresh: !cache.map.is_empty() && now_ts() - cache.at <= TTL,
        remote_url: REMOTE_URL.to_string(),
        ttl_secs: TTL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aw_catalog_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn family_longest_prefix_wins() {
        assert_eq!(family_lookup("gpt-4.1-mini"), Some(1_000_000));
        assert_eq!(family_lookup("gpt-4o"), Some(128_000));
        assert_eq!(family_lookup("claude-sonnet-4"), Some(200_000));
        assert_eq!(family_lookup("unknown-model"), None);
    }

    #[test]
    fn remote_cache_hit_preferred() {
        let dir = fixture();
        let mut map = HashMap::new();
        map.insert("gpt-4o".to_string(), (999u64, Some(111u64)));
        crate::store::db(&dir)
            .kv_set(KV_CACHE, &CacheEntry { at: now_ts(), map })
            .unwrap();
        let cap = lookup(&dir, "openai/gpt-4o", None);
        assert_eq!(cap.source, "remote");
        assert_eq!(cap.context_length, 999);
        assert_eq!(cap.max_output, Some(111));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn falls_back_through_levels() {
        let dir = fixture();
        // 家族兜底
        let cap = lookup(&dir, "deepseek-v4", None);
        assert_eq!(cap.source, "family");
        assert_eq!(cap.context_length, 131_072);
        assert!(cap.max_output.is_none(), "家族表不编造输出上限");
        // 本地
        let cap = lookup(&dir, "totally-unknown", Some((64_000, 8_000)));
        assert_eq!(cap.source, "local");
        assert_eq!(cap.max_output, Some(8_000));
        // 默认
        let cap = lookup(&dir, "totally-unknown", None);
        assert_eq!(cap.source, "default");
        assert_eq!(cap.context_length, DEFAULT_CONTEXT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expired_cache_is_ignored() {
        let dir = fixture();
        let mut map = HashMap::new();
        map.insert("m".to_string(), (5u64, None));
        crate::store::db(&dir)
            .kv_set(KV_CACHE, &CacheEntry { at: now_ts() - TTL - 10, map })
            .unwrap();
        let cap = lookup(&dir, "m", None);
        assert_eq!(cap.source, "default", "过期缓存不得命中");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
