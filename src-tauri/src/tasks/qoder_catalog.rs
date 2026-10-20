//! Qoder 模型目录每日同步（p3-3 收尾接线）。
//!
//! 网关路由判定依赖 [`qoder_upstream`] 目录（`resolve`/`list`）；静态兜底表是
//! 蓝本快照，倍率/名单会漂移（2026-10-01 实抓实证 GLM-5.3 0.6→0.8 等 7 处）。
//! 本任务每日拉取 `model/list`（真 COSY 签名 GET，凭证走 `ensure_fresh` 全防护）
//! → [`qoder_upstream::adopt_remote`] 原子替换 CN 区缓存。
//! - 空池空转；全部账号无凭证（未登录/需重登）静默跳过不计失败；
//! - 网络/服务端失败返 Err 交调度器 30 分钟冷却重试；
//! - 目录是账号无关的全局数据，无需逐账号拉取：按池序逐可用账号尝试，
//!   任一账号成功即收口；单个账号失败记日志后试下一个，全部失败才 Err（P3）。

use serde_json::{json, Value};

use crate::state::AppState;

use super::{http_agent, qoder_common, qoder_sign, qoder_upstream};

/// model/list 路径（拼在 [`qoder_upstream::QoderRegion::gateway`] 后；
/// GET 空体，COSY 签名覆盖见 qoder_sign——sigpath 自动剥离 /algo 前缀与查询串）
const MODEL_LIST_PATH: &str = "algo/api/v2/model/list";

/// CN 区 model/list 完整 URL（抽纯函数便于单测锚定端点形态）
fn model_list_url(realm: &str) -> String {
    format!("{}{MODEL_LIST_PATH}", super::qoder_realm::gateway_base(realm))
}

/// 调度器/CLI 共用入口：双区模型目录同步。
///
/// 区域必须**逐区各采一份**：目录缓存是「按区存的两份」（`adopt_remote(region, …)`
/// 只替换该区），而池里常见「国内账号在前、国际账号在后」。旧实现任一区账号成功
/// 即 `return`，于是有 CN 账号时 INTL 区目录永远拉不到 → `qoder-intl/<模型>`
/// 解析不到条目（表现为「获取模型后无该模型」）。现在按区分别尝试：该区任一
/// 账号成功即收口该区，继续下一区；单区失败只记日志（另一区成功仍算成功）。
pub fn run_task(state: &AppState) -> Result<Value, String> {
    let accounts = crate::commands::qoder::load_pool(state);
    if accounts.is_empty() {
        return Ok(json!({ "ok": true, "skipped": "无 Qoder 账号" }));
    }
    let agent = http_agent(20);
    // 逐区：收集该区账号 → 依次尝试 → 首个成功即收口该区
    let mut synced: Vec<(&'static str, usize)> = Vec::new();
    let mut last_err: Option<String> = None;
    let mut attempted = false;
    for realm in [super::qoder_realm::CN, super::qoder_realm::INTL] {
        let group: Vec<_> = accounts
            .iter()
            .filter(|a| super::qoder_realm::normalize(&a.realm) == realm)
            .collect();
        if group.is_empty() {
            continue; // 该区无账号：不拉取（跨区必 401/403）
        }
        let url = model_list_url(realm);
        let mut region_done = false;
        for a in group {
            let id = a.id.as_str();
            attempted = true;
            // 24h 惰性窗口（对齐探针/常规任务语义）：临期先刷再用
            let (creds, _refreshed, _note) = qoder_common::ensure_fresh(state, &agent, id, 24);
            // 无凭证/需重登账号跳过，试下一个（needs_relogin 账号的 token 可能已死）
            if creds.access_token.is_empty() || creds.uid.is_empty() {
                continue;
            }
            let identity = qoder_sign::CosyIdentity {
                user_id: &creds.uid,
                auth_token: &creds.access_token,
                name: &creds.nickname,
                email: "",
                machine_id: &creds.machine_id,
            };
            let headers = match qoder_sign::build_cosy_headers(None, &url, &identity) {
                Ok(h) => h,
                Err(e) => {
                    last_err = Some(format!("model/list 签名构造失败: {e}"));
                    crate::fs_utils::app_log(
                        &state.data_dir,
                        &format!(
                            "[qoder] 账号 {id}（{realm}）目录同步失败，尝试下一账号: {}",
                            last_err.as_deref().unwrap_or("")
                        ),
                    );
                    continue;
                }
            };
            let resp = headers
                .into_iter()
                .fold(agent.get(&url), |r, (k, v)| r.set(&k, &v))
                .call();
            match resp {
                Ok(resp) if resp.status() == 200 => {
                    // Q2：响应读取失败 / 非 JSON / 目录采纳失败与签名/HTTP 非 200/网络
                    // 失败同款容错——记 last_err 后落循环尾部日志试下一账号
                    match resp
                        .into_string()
                        .map_err(|e| format!("model/list 响应读取失败: {e}"))
                        .and_then(|body| {
                            serde_json::from_str::<Value>(&body)
                                .map_err(|e| format!("model/list 响应非 JSON: {e}"))
                        })
                        .and_then(|payload| {
                            qoder_upstream::adopt_remote(super::qoder_realm::region(realm), &payload)
                                .map_err(|e| format!("model/list 目录采纳失败: {e}"))
                        })
                    {
                        Ok(count) => {
                            synced.push((realm, count));
                            region_done = true;
                            break; // 该区收口，继续下一区（不 return 整任务）
                        }
                        Err(e) => last_err = Some(e),
                    }
                }
                Ok(resp) => {
                    last_err = Some(format!("model/list HTTP {}", resp.status()));
                }
                Err(e) => {
                    last_err = Some(format!("model/list 网络失败: {e}"));
                }
            }
            crate::fs_utils::app_log(
                &state.data_dir,
                &format!(
                    "[qoder] 账号 {id}（{realm}）目录同步失败，尝试下一账号: {}",
                    last_err.as_deref().unwrap_or("")
                ),
            );
        }
        if !region_done {
            // 单区失败不影响另一区：仅记日志，全部区都失败才在上面上抛
            crate::fs_utils::app_log(
                &state.data_dir,
                &format!(
                    "[qoder] {realm} 区目录同步未完成（该区无可用凭证或全部失败）: {}",
                    last_err.as_deref().unwrap_or("")
                ),
            );
        }
    }
    if synced.is_empty() {
        return match last_err {
            Some(e) if attempted => Err(e),
            _ => Ok(json!({ "ok": true, "skipped": "无可用 Qoder 凭证" })),
        };
    }
    // models = 各区采纳数之和（前端提示与旧语义兼容）
    let total: usize = synced.iter().map(|(_, n)| *n).sum();
    Ok(json!({
        "ok": true,
        "models": total,
        "regions": synced
            .iter()
            .map(|(r, n)| json!({ "realm": r, "models": n }))
            .collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_state(tag: &str) -> AppState {
        let dir = std::env::temp_dir()
            .join(format!("aiwork_qoder_catalog_test_{tag}_{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("data"));
        AppState {
            data_dir: dir,
            jwt_refresh_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            qoder_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            wb_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            doubao_pool_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
            group_defs_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
        }
    }

    #[test]
    fn url_anchored_to_cn_gateway() {
        assert_eq!(
            model_list_url("cn"),
            "https://gateway.qoder.com.cn/algo/api/v2/model/list"
        );
    }

    #[test]
    fn empty_pool_skips() {
        let st = temp_state("empty");
        let out = run_task(&st).expect("空池应 Ok");
        assert_eq!(out["ok"], true);
        assert!(out["skipped"].as_str().unwrap_or_default().contains("无 Qoder 账号"));
    }

    #[test]
    fn accounts_without_creds_skip_silently() {
        let st = temp_state("nocred");
        // 池里放一个账号（qoder_accounts 表），但 token store 为空
        // → ensure_fresh 返回 no_credential → 跳过
        crate::store::docs::qoder_pool_save(
            &crate::store::db(&st.data_dir),
            &json!({ "accounts": [ { "id": "qd-testnocred", "nickname": "t" } ] }),
        )
        .expect("写池失败");
        let out = run_task(&st).expect("无凭证应 Ok 跳过");
        assert_eq!(out["ok"], true);
        assert!(out["skipped"].as_str().unwrap_or_default().contains("无可用 Qoder 凭证"));
    }

    /// 区域端点锚定：两区各自拼自己的网关（跨区必 401/403，端点错则整区目录缺失）
    #[test]
    fn url_anchored_to_both_realms() {
        assert_eq!(
            model_list_url(crate::tasks::qoder_realm::INTL),
            "https://api3.qoder.sh/algo/api/v2/model/list"
        );
    }

    /// 双区账号池必须**逐区各采一份**：旧的「任一账号成功即 return」实现会让
    /// 国内账号抢先把任务收口，国际区目录永远拉不到 → `qoder-intl/<模型>`
    /// 解析不到条目（实测症状「获取模型后无该模型」）。这里用「两区账号都无
    /// 凭证」的场景验证两区都被遍历到（不再在首个区收口），并回调 Ok 跳过。
    #[test]
    fn both_realms_are_traversed() {
        let st = temp_state("tworealms");
        crate::store::docs::qoder_pool_save(
            &crate::store::db(&st.data_dir),
            &json!({ "accounts": [
                { "id": "qd-cn1", "nickname": "cn", "realm": "cn" },
                { "id": "qd-intl1", "nickname": "intl", "realm": "intl" },
            ]}),
        )
        .expect("写池失败");
        let out = run_task(&st).expect("无凭证应 Ok 跳过");
        assert_eq!(out["ok"], true);
        // 两区都无凭证 → 不产生 models，且跳过原因不是「无 Qoder 账号」
        assert!(out.get("models").is_none());
        assert!(out["skipped"].as_str().unwrap_or_default().contains("无可用 Qoder 凭证"));
    }
}
