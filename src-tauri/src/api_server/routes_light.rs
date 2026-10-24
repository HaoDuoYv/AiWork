//! 网关的轻量端点（P1 拆分自 routes.rs）：探活 + 按 Key 用量查询。
//!
//! 与 routes.rs 的协议转换/调度大块分离——这几处改动频率高、依赖少，
//! 独立成文件便于小步修改（红线：大文件上用 Edit 的出错面更大）。
//! 路由注册仍在 `server.rs::build_router`。

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::{json, Value};

use super::usage::KeyId;
use super::ApiSharedState;

pub async fn health(State(state): State<Arc<ApiSharedState>>) -> impl IntoResponse {
    // 收敏（2026-10-12 审查 P2）：/health 在 auth.rs 免鉴权放行（探活豁免），
    // 原实现透出 total_credits/active_uid/last_error/池账号数——非环回监听时
    // 局域网内无 Key 即可抓取账号与积分画像，与「非环回强制鉴权」防线矛盾。
    // 探活只需存活布尔；完整诊断由需鉴权的 GET /status 承载（字段超集）
    let _ = &state;
    Json(json!({
        "status": "ok",
        "running": true,
    }))
}

/// /healthz（T2.3/F-32）：三池均无可用账号 → 503，供探活/看门狗。
/// H1 修复（审查）：旧口径仅 Trae ∨ WB 且不查 hard_credit/积分过期/零积分——
/// 仅部署 Qoder 账号或全部账号 hard_credit 冷却时探活恒 503（可能触发误重启）。
/// 改用 has_selectable()（= pool::selectable 口径：健康 + 未禁用 + 未冷却 +
/// 积分未过期且非零），三池 × enabled 开关，与 /v1/models 徽章判定同源
pub async fn healthz(State(state): State<Arc<ApiSharedState>>) -> Response {
    let trae_enabled = state.trae_enabled.load(std::sync::atomic::Ordering::Relaxed);
    let solo_ok = trae_enabled && state.pool.has_selectable();
    let wb_enabled = state.wb_enabled.load(std::sync::atomic::Ordering::Relaxed);
    let wb_ok = wb_enabled && state.wb_pool.has_selectable();
    let qoder_enabled = state.qoder_enabled.load(std::sync::atomic::Ordering::Relaxed);
    // 双区池：CN 或 INTL 任一有可选账号即视为 Qoder 可用；INTL 池受
    // `qoder_intl_enabled` 开关约束（关闭时国际池不参调，与 dispatch 三级路由
    // 回落国内口径一致）
    let qoder_intl_enabled = {
        let s: crate::models::Settings =
            crate::store::db(&state.data_dir).kv_get("app_settings");
        s.qoder_intl_enabled
    };
    let qoder_ok = qoder_enabled
        && (state.qoder_pool.has_selectable()
            || (qoder_intl_enabled && state.qoder_intl_pool.has_selectable()));
    if solo_ok || wb_ok || qoder_ok {
        (
            StatusCode::OK,
            axum::Json(json!({
                "status": "ok",
                "solo_available": solo_ok,
                "wb_available": wb_ok,
                "qoder_available": qoder_ok,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "status": "unavailable", "reason": "no healthy account" })),
        )
            .into_response()
    }
}

/// `GET /v1/usage`：当前调用方 API Key 的当日用量（CC Switch「用量查询」自定义脚本用）。
///
/// 鉴权已由 bearer_auth 完成（未带 Key 时根据配置记为 anonymous 或 401），这里
/// 只取鉴权结果里命中的 Key 记账：返回 `{success, data:[{planName, used, total,
/// remaining, unit}]}`——CC Switch 的 extractor 可把 `data` 数组原样返回当作
/// 多套餐展示（参照其 New API 模板）。`used` / `total` 为「请求次数」，`total`
/// 在 daily_limit=0（不限）时为 null，表示无上限。本端点不消耗请求配额：
/// 用量查询是观测行为，不应反过来推高被观测的计数。
pub async fn usage(
    State(state): State<Arc<ApiSharedState>>,
    Extension(key): Extension<KeyId>,
) -> Response {
    if key.0 == "anonymous" {
        return (
            StatusCode::OK,
            Json(json!({
                "success": false,
                "message": "API 鉴权已关闭，无法按 Key 统计用量（请在 API 服务页启用 API Key 后用其查询）",
            })),
        )
            .into_response();
    }
    let data_dir = state.data_dir.clone();
    let key_id = key.0.clone();
    let view = tokio::task::spawn_blocking(move || {
        crate::api_server::api_keys::usage_view(&data_dir, &key_id, &super::usage::today_key())
    })
    .await
    .ok()
    .flatten();
    let Some(e) = view else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "success": false, "message": "API Key 已不存在（可能已被删除）" })),
        )
            .into_response();
    };
    let unlimited = e.daily_limit == 0;
    let (week_sum, week_days) = week_usage(&e);
    let mut extra = if unlimited {
        "不限每日请求数".to_string()
    } else {
        format!("每日上限 {} 次", e.daily_limit)
    };
    if week_days > 0 {
        extra.push_str(&format!(" · 近 {} 天共 {} 次", week_days, week_sum));
    }
    Json(json!({
        "success": true,
        "data": [{
            "planName": e.name,
            "used": e.used_today,
            "total": if unlimited { Value::Null } else { json!(e.daily_limit) },
            "remaining": if unlimited { Value::Null } else { json!(e.daily_limit.saturating_sub(e.used_today)) },
            "unit": "times",
            "extra": extra,
            "date": e.used_date,
            "week_total": week_sum,
            "week_days": week_days,
        }]
    }))
    .into_response()
}

/// 近 7 日合计（含今日）：额度不限时卡片只有一个裸数字，没有参照物；
/// 用「近 N 天共 M 次」给出量级。返回 (合计请求数, 有记账的天数)。
/// 纯函数便于单测：daily_stats 升序保留最近 90 天，超期项按日期字符串（YYYY-MM-DD）
/// 与今日比较后剔除——不依赖时区解析。
fn week_usage(e: &crate::api_server::api_keys::ApiKeyEntry) -> (u64, usize) {
    let today = super::usage::today_key();
    let from = (chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
        .map(|d| (d - chrono::Duration::days(6)).format("%Y-%m-%d").to_string()))
    .unwrap_or_else(|_| String::new());
    let mut sum = 0u64;
    let mut days = 0usize;
    for s in e.daily_stats.iter().filter(|s| s.date.as_str() >= from.as_str()) {
        sum = sum.saturating_add(s.requests);
        days += 1;
    }
    (sum, days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_server::api_keys::{ApiKeyEntry, KeyDailyStat};

    fn key_with(stats: Vec<(&str, u64)>) -> ApiKeyEntry {
        let mut e: ApiKeyEntry = serde_json::from_value(json!({
            "id": "k", "name": "n", "key": "ck-x", "enabled": true,
        }))
        .unwrap();
        e.daily_stats = stats
            .into_iter()
            .map(|(date, requests)| KeyDailyStat { date: date.to_string(), requests })
            .collect();
        e
    }

    /// 只统计最近 7 天（含今日）；更早的记账天不计入；空数据返回 (0, 0)
    #[test]
    fn week_usage_window_and_sum() {
        let today = super::super::usage::today_key();
        let old = "2000-01-01";
        let e = key_with(vec![(old, 999), (&today, 3)]);
        let (sum, days) = week_usage(&e);
        assert_eq!(sum, 3, "窗口外的旧数据不得计入");
        assert_eq!(days, 1);
        assert_eq!(week_usage(&key_with(vec![])), (0, 0));
    }
}