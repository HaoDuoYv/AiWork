//! 枢纽增强包：命令层（上游组 / 成本 / 活动 / 出口探测 / 模型目录 / 信封）
//!
//! 本模块只做「薄壳」：参数校验、状态读写、活动记账，业务算法全部落在
//! `api_server::{upstream_pool, pricing, activity, modelsdev, atrest}`。
//! 这样网关热路径（`custom_route`）与 UI 命令共用同一份实现，不会出现
//! "页面显示的策略和实际生效的策略不是一套"。
//!
//! 命名前缀统一 `hub_`，便于在 `main.rs::generate_handler!` 里成组识别。

use crate::api_server::{activity, atrest, modelsdev, pricing, upstream_pool};
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ==================== 上游组 ====================

/// 上游组列表（含运行时健康；运行态不落盘）
#[tauri::command]
pub fn hub_upstream_groups(state: tauri::State<'_, AppState>) -> Vec<upstream_pool::GroupView> {
    upstream_pool::snapshot(&state.data_dir)
}

/// 保存/覆盖一个上游组
#[tauri::command]
pub fn hub_upstream_group_save(
    state: tauri::State<'_, AppState>,
    group: upstream_pool::UpstreamGroup,
) -> Result<Vec<upstream_pool::GroupView>, String> {
    let model = group.model.clone();
    let n = group.endpoints.len();
    upstream_pool::save_group(&state.data_dir, group)?;
    activity::record(
        &state.data_dir,
        "upstream",
        "hub",
        &format!("保存上游组：{model}（{n} 个端点）"),
        serde_json::json!({"model": model, "route": "group_save"}),
    );
    Ok(upstream_pool::snapshot(&state.data_dir))
}

/// 删除上游组
#[tauri::command]
pub fn hub_upstream_group_remove(
    state: tauri::State<'_, AppState>,
    model: String,
) -> Result<bool, String> {
    let removed = upstream_pool::remove_group(&state.data_dir, &model)?;
    if removed {
        activity::record(
            &state.data_dir,
            "upstream",
            "hub",
            &format!("删除上游组：{model}"),
            serde_json::json!({"model": model, "route": "group_remove"}),
        );
    }
    Ok(removed)
}

/// 重置全部运行时健康状态（熔断/退避清零；402 停放一并解除）
#[tauri::command]
pub fn hub_upstream_reset_health(state: tauri::State<'_, AppState>) -> usize {
    let n = upstream_pool::snapshot(&state.data_dir)
        .iter()
        .map(|g| g.endpoints.len())
        .sum();
    upstream_pool::reset_runtime();
    activity::record(
        &state.data_dir,
        "upstream",
        "hub",
        "重置上游健康状态",
        serde_json::json!({"endpoint": n}),
    );
    n
}

/// 上游连通性探测（探活当前选中端点，失败自动按族退避记账）
#[tauri::command]
pub async fn hub_upstream_test(
    state: tauri::State<'_, AppState>,
    model: String,
) -> Result<ProbeOutcome, String> {
    let data_dir = state.data_dir.clone();
    let endpoint = upstream_pool::pick(&data_dir, &model);
    let Some(ep) = endpoint else {
        return Err(format!(
            "模型 {model} 没有可用端点（未配置上游组或全部冷却中）"
        ));
    };
    let model_clone = model.clone();
    let base = ep.base_url.clone();
    let key = ep.api_key.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        probe_endpoint_blocking(&base, &key, &model_clone)
    })
    .await
    .map_err(|e| format!("探测任务失败: {e}"))?;
    if outcome.ok {
        upstream_pool::record_success(&model, &ep.id);
    } else {
        let kind = upstream_pool::FailKind::from_status(outcome.status);
        upstream_pool::record_failure(&model, &ep.id, kind, &outcome.detail);
    }
    activity::record(
        &data_dir,
        "upstream",
        "hub",
        &format!("探测上游 {model} → {}", if outcome.ok { "通过" } else { "失败" }),
        serde_json::json!({
            "model": model,
            "endpoint": ep.label,
            "status": outcome.status.unwrap_or(0),
            "latency_ms": outcome.latency_ms,
            "ok": outcome.ok,
        }),
    );
    Ok(outcome)
}

/// 探测结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ProbeOutcome {
    pub ok: bool,
    /// HTTP 状态码（传输层失败为 None）
    pub status: Option<u16>,
    pub latency_ms: i64,
    /// 成功时的正文预览 / 失败原因
    pub detail: String,
    pub base_url: String,
}

fn probe_endpoint_blocking(base_url: &str, api_key: &str, model: &str) -> ProbeOutcome {
    let url = crate::api_server::custom_models::chat_url(base_url);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(6))
        .timeout_read(std::time::Duration::from_secs(15))
        .timeout_write(std::time::Duration::from_secs(10))
        .build();
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 8,
        "stream": false,
    })
    .to_string();
    let started = now_ms();
    let mut req = agent.post(&url).set("content-type", "application/json");
    if !api_key.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {api_key}"));
    }
    let mut out = ProbeOutcome {
        base_url: base_url.to_string(),
        ..Default::default()
    };
    match req.send_string(&body) {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            let preview: String = text.chars().take(160).collect();
            let parsed_ok = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .map(|v| v.get("choices").is_some() || v.get("error").is_none())
                .unwrap_or(false);
            out.status = Some(status);
            out.ok = (200..300).contains(&status) && parsed_ok;
            out.detail = if out.ok {
                format!("连通成功：{preview}")
            } else {
                format!("上游返回 status {status}：{preview}")
            };
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            let preview: String = text.chars().take(160).collect();
            out.status = Some(code);
            out.ok = false;
            out.detail = format!("上游返回 status {code}：{preview}");
        }
        Err(e) => {
            out.ok = false;
            out.detail = format!("传输层失败：{e}");
        }
    }
    out.latency_ms = now_ms() - started;
    out
}

// ==================== 成本核算 ====================

/// 策略列表
#[tauri::command]
pub fn hub_cost_policies(state: tauri::State<'_, AppState>) -> Vec<pricing::CostPolicy> {
    pricing::load_policies(&state.data_dir)
}

/// 保存策略（id 由内容重算）
#[tauri::command]
pub fn hub_cost_policy_save(
    state: tauri::State<'_, AppState>,
    policy: pricing::CostPolicy,
) -> Result<pricing::CostPolicy, String> {
    let saved = pricing::upsert_policy(&state.data_dir, policy)?;
    activity::record(
        &state.data_dir,
        "gateway",
        "hub",
        &format!("保存计价策略：{}（{}）", saved.model, saved.id),
        serde_json::json!({"model": saved.model, "policy_id": saved.id, "route": "pricing"}),
    );
    Ok(saved)
}

/// 删除策略
#[tauri::command]
pub fn hub_cost_policy_remove(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    pricing::remove_policy(&state.data_dir, &id)
}

/// 成本汇总（最近 N 天；0/None = 全量）
#[tauri::command]
pub fn hub_cost_summary(
    state: tauri::State<'_, AppState>,
    days: Option<u32>,
) -> pricing::CostSummary {
    pricing::summary(&state.data_dir, days.unwrap_or(30))
}

/// 成本明细（最近 N 条）
#[tauri::command]
pub fn hub_cost_records(
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
) -> Vec<pricing::CostRecord> {
    pricing::recent_records(&state.data_dir, limit.unwrap_or(100))
}

/// 按给定单价试算（UI「试算器」，不落库）
#[tauri::command]
pub fn hub_cost_preview(
    policy: pricing::CostPolicy,
    in_tokens: u64,
    out_tokens: u64,
) -> serde_json::Value {
    let pin_out = pricing::unit_price(&policy, in_tokens);
    serde_json::json!({
        "cost": pricing::price(&policy, in_tokens, out_tokens),
        "unit_in": pin_out.0,
        "unit_out": pin_out.1,
        "currency": if policy.currency.is_empty() { "CNY".to_string() } else { policy.currency.clone() },
    })
}

// ==================== 活动时间线 ====================

/// 最近活动（`kind` 为空 = 全部）
#[tauri::command]
pub fn hub_activity_recent(
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
    kind: Option<String>,
) -> Vec<activity::ActivityEvent> {
    let limit = limit.unwrap_or(100);
    match kind {
        Some(k) if !k.trim().is_empty() => activity::recent_by_kind(&state.data_dir, k.trim(), limit),
        _ => activity::recent(&state.data_dir, limit),
    }
}

/// 清空活动（JSONL 移到 `.1` 保留一代）
#[tauri::command]
pub fn hub_activity_clear(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    activity::clear(&state.data_dir)
}

// ==================== 出口探测 ====================

/// 出口信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct EgressInfo {
    pub ok: bool,
    pub ip: String,
    pub region: String,
    pub isp: String,
    pub latency_ms: i64,
    pub source_url: String,
    pub probed_at: i64,
    pub error: String,
}

/// 出口探测缓存（60s；与 hub「同一事实不重复打第三方」口径一致）
fn egress_cache() -> &'static std::sync::Mutex<Option<(i64, EgressInfo)>> {
    static C: std::sync::OnceLock<std::sync::Mutex<Option<(i64, EgressInfo)>>> =
        std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(None))
}

const EGRESS_TTL: i64 = 60;

fn probe_egress_blocking() -> EgressInfo {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_secs(8))
        .build();
    let started = now_ms();
    let mut out = EgressInfo {
        probed_at: now_ts_secs(),
        ..Default::default()
    };
    // 第一跳：拿公网 IP
    match agent.get("https://api.ipify.org?format=json").call() {
        Ok(resp) => {
            let text = resp.into_string().unwrap_or_default();
            let ip = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("ip").and_then(|i| i.as_str()).map(str::to_string))
                .unwrap_or_default();
            out.ip = ip;
            out.source_url = "https://api.ipify.org".to_string();
            out.ok = !out.ip.is_empty();
        }
        Err(e) => {
            out.error = format!("ipify 探测失败：{e}");
        }
    }
    // 第二跳：地区/运营商（失败不覆盖 ok，只是缺信息）
    if out.ok {
        if let Ok(resp) = agent.get("https://ipinfo.io/json").call() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp.into_string().unwrap_or_default())
            {
                out.region = v
                    .get("country")
                    .and_then(|c| c.as_str())
                    .map(|c| {
                        let city = v.get("city").and_then(|x| x.as_str()).unwrap_or("");
                        if city.is_empty() {
                            c.to_string()
                        } else {
                            format!("{c}/{city}")
                        }
                    })
                    .unwrap_or_default();
                out.isp = v
                    .get("org")
                    .and_then(|o| o.as_str())
                    .map(str::to_string)
                    .unwrap_or_default();
            }
        }
    }
    out.latency_ms = now_ms() - started;
    if !out.ok && out.error.is_empty() {
        out.error = "出口探测失败（网络不可达或第三方接口变更）".to_string();
    }
    out
}

fn now_ts_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 出口 IP 情报（缓存 60s；`force=true` 绕过缓存）
#[tauri::command]
pub async fn hub_egress_info(force: Option<bool>) -> EgressInfo {
    let force = force.unwrap_or(false);
    if !force {
        let guard = egress_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, info)) = guard.as_ref() {
            if now_ts_secs() - *at <= EGRESS_TTL {
                return info.clone();
            }
        }
    }
    let info = tauri::async_runtime::spawn_blocking(probe_egress_blocking)
        .await
        .unwrap_or_else(|_| EgressInfo {
            error: "出口探测任务失败".to_string(),
            probed_at: now_ts_secs(),
            ..Default::default()
        });
    {
        let mut guard = egress_cache().lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some((now_ts_secs(), info.clone()));
    }
    info
}

// ==================== 模型能力目录 ====================

/// 目录状态
#[tauri::command]
pub fn hub_catalog_status(state: tauri::State<'_, AppState>) -> modelsdev::CatalogStatus {
    modelsdev::status(&state.data_dir)
}

/// 同步远程目录（阻塞任务放后台线程）
#[tauri::command]
pub async fn hub_catalog_sync(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let data_dir = state.data_dir.clone();
    let n = tauri::async_runtime::spawn_blocking(move || modelsdev::sync_remote(&data_dir))
        .await
        .map_err(|e| format!("同步任务失败: {e}"))??;
    activity::record(
        &state.data_dir,
        "system",
        "hub",
        &format!("同步模型能力目录：{n} 条"),
        serde_json::json!({"bytes": n}),
    );
    Ok(n)
}

/// 四级查找（本地值由调用方传入：自定义模型的 context_length / max_tokens）
#[tauri::command]
pub fn hub_catalog_lookup(
    state: tauri::State<'_, AppState>,
    model: String,
    local_context: Option<u64>,
    local_output: Option<u64>,
) -> modelsdev::ModelCapability {
    let local = match (local_context, local_output) {
        (Some(c), Some(o)) => Some((c, o)),
        (Some(c), None) => Some((c, 0)),
        (None, Some(o)) => Some((0, o)),
        (None, None) => None,
    };
    modelsdev::lookup(&state.data_dir, &model, local)
}

// ==================== 静态加密信封 ====================

/// 加密任意文本（`code` 用于隔离用途，如 AGENT_BUNDLE / COST_EXPORT）
#[tauri::command]
pub fn hub_seal(text: String, seed: String, code: String) -> Result<atrest::Envelope, String> {
    atrest::seal(&text, &seed, &code)
}

/// 解密信封
#[tauri::command]
pub fn hub_open(envelope: atrest::Envelope, seed: String) -> Result<String, String> {
    atrest::open(&envelope, &seed)
}

/// 口令强度提示
#[tauri::command]
pub fn hub_seed_strength(seed: String) -> String {
    atrest::seed_strength(&seed).to_string()
}

/// 导出「智能体接入包」并加密（内容 = `agent_import::export_bundle`）
#[tauri::command]
pub fn hub_export_bundle_sealed(
    state: tauri::State<'_, AppState>,
    seed: String,
) -> Result<atrest::Envelope, String> {
    let bundle = crate::commands::agent_import::export_bundle(&state.data_dir);
    let env = atrest::seal(&bundle, &seed, "AGENT_BUNDLE")?;
    activity::record(
        &state.data_dir,
        "agent",
        "hub",
        "导出加密智能体接入包",
        serde_json::json!({"route": "bundle_export", "policy_id": env.key_id}),
    );
    Ok(env)
}

/// 数据目录（前端展示用；不含敏感信息）
#[tauri::command]
pub fn hub_data_dir(state: tauri::State<'_, AppState>) -> String {
    state.data_dir.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_rejects_invalid_url_gracefully() {
        // 非法地址不应 panic，而是返回 ok=false + detail
        let out = probe_endpoint_blocking("http://127.0.0.1:1", "", "m");
        assert!(!out.ok);
        assert!(!out.detail.is_empty());
        assert!(out.latency_ms >= 0);
    }

    #[test]
    fn now_ms_monotonic_nonzero() {
        assert!(now_ms() > 1_600_000_000_000);
    }
}
