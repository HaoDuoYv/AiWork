use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::api_keys::{self, ApiKeysFile, KeyCheck};
use super::usage::KeyId;
use super::ApiSharedState;

/// API Key 鉴权中间件：
/// - 仅 /health 豁免且仅限环回监听（响应已收敏为 status/running 存活布尔，完整
///   诊断在需鉴权的 /status）；/healthz 不豁免——其 503 体含
///   reason=no healthy account，非环回走 Key 鉴权（fail-closed）
/// - Key 注册表存 vault（加密），内存权威副本 + flusher 落盘（批次 E），
///   支持 Authorization: Bearer <key>（OpenAI 风格）或 x-api-key: <key>（Anthropic 风格）
/// - 存在启用 Key 时必须鉴权；无任何启用 Key 时：auth_disabled=true（显式关闭鉴权）放行记 anonymous，
///   否则拒绝（默认）并返回引导提示
/// - 命中即累加当日用量（内存权威副本 + 标脏，flusher 2s 落盘），超配额返回 429
/// - UI 增删/禁用/开关经 api_keys::save 即时刷新内存副本，立即生效
/// - 组/标签约束经 org_index 锁外展开（D4）随 ResolvedKey 下发
/// 校验通过后向 request extensions 插入命中的 Key 标识（KeyId），供 handler 用量记账
pub async fn bearer_auth(
    State(state): State<Arc<ApiSharedState>>,
    mut request: Request,
    next: Next,
) -> Response {
    // 环回豁免与 bind 物理同源（2026-10-12 审查 M3）：按实际绑定 host 判定，
    // 不重读配置——改配置未重启时不会误用新配置值放行（fail-open）。非环回
    // 监听时不豁免，与其他端点一致走 Key 鉴权（fail-closed）。
    //
    // 2026-10-19 实机修复：豁免范围从 /health 扩到 /healthz。两者都是「服务是否
    // 可达」类探活端点，而 /healthz 此前未豁免——面板/自动化探活打到的是
    // 401 `api_key_required`，被当成「API 服务无法访问」，实际服务运行正常。
    // 环回下放行不额外暴露：/healthz 无鉴权上下文时 key 名记为 anonymous，
    // 归因文案不含具体 Key；非环回监听仍 fail-closed（503 体含账号细节）。
    let bind_host = state
        .listen_host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let loopback = super::server::is_loopback_host(&bind_host);
    if loopback && matches!(request.uri().path(), "/health" | "/healthz") {
        return next.run(request).await;
    }

    // 请求体预检（issue #21）：Content-Length 超过网关上限时在 body 提取器之前
    // 返回结构化 413（axum 提取器超限默认回纯文本，客户端难以解析识别）；
    // 无 Content-Length（chunked）的超限请求仍由 DefaultBodyLimit 兜底拒绝
    if let Some(len) = request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
    {
        if len > super::routes::MAX_BODY_BYTES {
            return body_too_large();
        }
    }

    // 提取呈现的 Key（Bearer 优先，其次 x-api-key）
    let authz = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    let bearer = authz
        .filter(|s| s.len() > 7 && s[..7].eq_ignore_ascii_case("Bearer "))
        .map(|s| s[7..].to_string());
    let xkey = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let presented = bearer.or(xkey);

    // 鉴权热路径（P1 修复5b）：load + verify_and_consume_locked（内含记账写盘）
    // 全为同步磁盘 IO，整体移入 spawn_blocking（参数转 owned），避免阻塞
    // async 调度线程；锁与原子语义不变（verify_and_consume_locked 进程级锁内完成）
    let (auth_required, check) = {
        let data_dir = state.data_dir.clone();
        tokio::task::spawn_blocking(move || {
            let keys: ApiKeysFile = api_keys::load(&data_dir);
            // 存在启用 Key 时必须鉴权；无启用 Key 时由显式开关决定放行或拒绝
            let auth_required = keys.has_enabled() || !keys.auth_disabled;
            let check = presented.map(|p| {
                api_keys::verify_and_consume_locked(&data_dir, &p, &super::usage::today_key())
            });
            (auth_required, check)
        })
        .await
        // join 失败（panic 等）按最严格处理：要求鉴权 + 视为无效 Key → 401
        .unwrap_or((true, Some(KeyCheck::Invalid)))
    };

    match check {
        Some(KeyCheck::Ok(rk)) => {
            let id = rk.id.clone();
            request.extensions_mut().insert(rk);
            request.extensions_mut().insert(KeyId(id));
            return next.run(request).await;
        }
        Some(KeyCheck::QuotaExceeded { limit }) => {
            return quota_exceeded(limit);
        }
        Some(KeyCheck::Invalid) => {}
        None => {
            // 未携带 Key
            if !auth_required {
                request.extensions_mut().insert(KeyId("anonymous".into()));
                return next.run(request).await;
            }
            return auth_required_rejected(&api_bind_display(&state), loopback).into_response();
        }
    }
    // 无启用 Key 且显式关闭鉴权：放行携带未知 Key 的请求并记为 anonymous
    if !auth_required {
        request.extensions_mut().insert(KeyId("anonymous".into()));
        return next.run(request).await;
    }

    (StatusCode::UNAUTHORIZED, "invalid api key").into_response()
}

/// 413 请求体超限响应（issue #21）：结构化 JSON 错误体（OpenAI/Anthropic 客户端
/// 均可解析 message），替代 axum 提取器的纯文本 413
fn body_too_large() -> Response {
    let body = json!({
        "error": {
            "message": super::routes::body_too_large_msg(),
            "type": "invalid_request_error",
            "code": "request_too_large",
        }
    });
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// 当前监听地址展示（401 文案用）：`host:port`。
/// host 取**进程实际绑定值**（state.listen_host），port 取设置项——两者都不代表
/// 客户端可达性，文案里要明确这一点（见 auth_required_message）
fn api_bind_display(state: &ApiSharedState) -> String {
    let host = state
        .listen_host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let port = crate::store::db(&state.data_dir)
        .kv_get::<crate::models::Settings>("app_settings")
        .api_port;
    format!("{host}:{port}")
}

/// 401 未带 Key 的文案（纯函数便于单测）：2026-10-19 实机「开启服务后 API 无法
/// 访问」的真实成因有两类——① 客户端没带 Key（本 401）；② 服务只监听 127.0.0.1，
/// 而客户端在容器/WSL/另一台机器上（连接被拒）。旧文案只说"启用鉴权"，两种场景
/// 都读不出下一步，所以把「服务运行正常 + 当前监听地址 + 何时该改 0.0.0.0」写进去。
fn auth_required_message(bind_addr: &str, loopback: bool) -> String {
    if loopback {
        format!(
            "API 服务运行正常，但本端点需要 API Key：请在应用的「API 服务」页创建并启用 API Key，\
             或在该页显式关闭鉴权（不推荐，任何本机程序均可调用）。\
             当前监听 {bind_addr}（仅本机可连）——若客户端在容器 / WSL / 局域网另一台设备上，\
             请把监听地址改为 0.0.0.0 并启用 API Key"
        )
    } else {
        format!(
            "API 服务运行正常，但本端点需要 API Key：请在应用的「API 服务」页创建并启用 API Key\
             （当前监听 {bind_addr}）"
        )
    }
}

/// 401：要求鉴权但不满足（未配置启用 Key 且未显式关闭鉴权，或未携带 Key）。
/// JSON 错误体给出下一步指引（OpenAI/Anthropic 客户端均可解析 message）。
fn auth_required_rejected(bind_addr: &str, loopback: bool) -> axum::response::Response {
    let body = json!({
        "error": {
            "message": auth_required_message(bind_addr, loopback),
            "type": "invalid_request_error",
            "code": "api_key_required",
        }
    });
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归锁（2026-10-19 实机）：401 文案必须写清「服务运行正常」+ 当前监听地址，
    /// 环回监听时补「客户端不在本机就要改 0.0.0.0」——否则「无法访问」无从下手
    #[test]
    fn auth_required_message_explains_running_and_bind() {
        let loopback = auth_required_message("127.0.0.1:7864", true);
        assert!(loopback.contains("运行正常"), "{loopback}");
        assert!(loopback.contains("127.0.0.1:7864"), "{loopback}");
        assert!(loopback.contains("0.0.0.0"), "环回监听需给出外部访问指引: {loopback}");
        // 已监听 0.0.0.0：不再提示改监听地址（避免误导）
        let wildcard = auth_required_message("0.0.0.0:7864", false);
        assert!(wildcard.contains("运行正常"), "{wildcard}");
        assert!(!wildcard.contains("0.0.0.0 并启用"), "{wildcard}");
    }
}

/// 429 配额超限响应（JSON 错误体，OpenAI/Anthropic 客户端均可解析 message）
fn quota_exceeded(limit: u64) -> Response {
    let body = json!({
        "error": {
            "message": format!("API Key 已达今日配额上限（{limit} 次/日），请明天再试或调整限额"),
            "type": "quota_exceeded",
            "code": "daily_quota_exceeded",
        }
    });
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}
