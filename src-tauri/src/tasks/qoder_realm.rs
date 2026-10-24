//! Qoder 双区域（CN / 国际版）单一事实源。
//!
//! ## 为什么需要
//! 桌面端与两个兄弟项目一致存在**两套互不相通的账号体系**：
//!
//! | 项 | CN（国内版） | INTL（国际版） |
//! |---|---|---|
//! | OpenAPI | `openapi.qoder.com.cn` | `openapi.qoder.sh` |
//! | 推理网关 | `gateway.qoder.com.cn` | `api1/api2/api3.qoder.sh` |
//! | 官网域 | `qoder.cn` | `qoder.com` |
//! | OAuth client_id | `1c5e33e1-…` | `e883ade2-…` |
//! | 客户端数据目录 | `com.qodercn.app.stable` / `QoderCN` | `com.qoder.app.stable` |
//! | CLI 主目录 | `~/.qoder-cn` | `~/.qoder` |
//!
//! token 不能混用：拿 CN token 打 `.sh` 端点、或反过来，都是 401/403。所以
//! 「区域」必须是**账号级**属性（`QoderAccount.realm`），而不是进程级全局
//! （hub 自己也记录了这个坑：`CURRENT_REALM` 是全局，多区域混跑会串区）。
//!
//! ## 与既有代码的关系
//! `qoder_upstream::QoderRegion` 早已存在（双区模型目录缓存），但只用于**目录解析**，
//! 传输层恒打 CN 网关。本模块把「台账（realm 字符串）」映射到 `QoderRegion`，
//! 并提供各端点的基址，供传输 / 积分 / 签到 / 目录统一取值。

use super::qoder_upstream::QoderRegion;

/// 国内版
pub const CN: &str = "cn";
/// 国际版
pub const INTL: &str = "intl";

/// OAuth 设备流 client_id（按区域）
pub const CLIENT_ID_CN: &str = "1c5e33e1-364d-4ce6-b02c-acaa81274a5c";
pub const CLIENT_ID_INTL: &str = "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb";

/// 归一：空/未知一律按 CN（存量账号无 realm 字段，默认沿用旧行为）
pub fn normalize(realm: &str) -> &'static str {
    let r = realm.trim().to_ascii_lowercase();
    match r.as_str() {
        "intl" | "international" | "global" | "com" => INTL,
        _ => CN,
    }
}

/// 是否国际版
pub fn is_intl(realm: &str) -> bool {
    normalize(realm) == INTL
}

/// realm → 上游目录/网关区域
pub fn region(realm: &str) -> QoderRegion {
    if is_intl(realm) {
        QoderRegion::Global
    } else {
        QoderRegion::Cn
    }
}

/// 区域展示名
pub fn label(realm: &str) -> &'static str {
    if is_intl(realm) {
        "国际版"
    } else {
        "国内版"
    }
}

/// OpenAPI 基址（userinfo / plan / sash 活动 / usage / token refresh）
pub fn openapi_base(realm: &str) -> &'static str {
    if is_intl(realm) {
        "https://openapi.qoder.sh"
    } else {
        "https://openapi.qoder.com.cn"
    }
}

/// 官网域基址（`me/usages/big_model_credits` 等仅官网域存在的路由）
pub fn web_base(realm: &str) -> &'static str {
    if is_intl(realm) {
        "https://qoder.com"
    } else {
        "https://qoder.cn"
    }
}

/// 推理网关基址（带结尾斜杠；与 `QoderRegion::gateway()` 保持一致）
pub fn gateway_base(realm: &str) -> &'static str {
    if is_intl(realm) {
        "https://api3.qoder.sh/"
    } else {
        "https://gateway.qoder.com.cn/"
    }
}

/// 对话上游 URL（SSE）
pub fn chat_url(realm: &str) -> &'static str {
    if is_intl(realm) {
        "https://api3.qoder.sh/algo/api/v2/service/pro/sse/agent_chat_generation?FetchKeys=llm_model_result&AgentId=agent_common&Encode=1"
    } else {
        "https://gateway.qoder.com.cn/algo/api/v2/service/pro/sse/agent_chat_generation?FetchKeys=llm_model_result&AgentId=agent_common&Encode=1"
    }
}

/// 对话上游 host（传输错误文案用）
pub fn chat_host(realm: &str) -> &'static str {
    if is_intl(realm) {
        "https://api3.qoder.sh"
    } else {
        "https://gateway.qoder.com.cn"
    }
}

/// 模型目录 URL（`model/list`，签名 body 必须是 `qoder_encode(b"{}")`）
pub fn model_list_url(realm: &str) -> String {
    format!("{}algo/api/v2/model/list", gateway_base(realm))
}

/// 设备授权 client_id
pub fn client_id(realm: &str) -> &'static str {
    if is_intl(realm) {
        CLIENT_ID_INTL
    } else {
        CLIENT_ID_CN
    }
}

/// 由域名 / JWT issuer 推断区域（导入外部账号时用）
pub fn from_domain(domain: &str) -> &'static str {
    let d = domain.trim().to_ascii_lowercase();
    if d.contains("qoder.sh")
        || d == "qoder.com"
        || d.ends_with(".qoder.com")
        || d.contains("qoder.com/")
    {
        INTL
    } else {
        CN
    }
}

/// 由客户端数据目录名推断区域（IDE 扫描用；与 `commands/qoder/common.rs::realm_of_ide_dir` 同口径）
pub fn from_dir_name(name: &str) -> &'static str {
    let n = name.trim().to_ascii_lowercase();
    if n.contains("qodercn") || n.contains("qoder cn") || n.contains("qoder-cn") {
        CN
    } else if n.contains("com.qoder") || n == "qoder" {
        INTL
    } else {
        CN
    }
}

/// 池标签（方案 C：区域升为**池维度**）。
/// 两个池各自一套账号/目录/冷却/粘性，跨区不混轮；`qoder` 仅作兼容别名。
pub fn pool_tag(realm: &str) -> &'static str {
    if is_intl(realm) {
        "qoder_intl"
    } else {
        "qoder_cn"
    }
}

/// 池标签 → 区域（`qoder` / 未知 → cn，保持存量行为）
pub fn realm_of_pool(tag: &str) -> &'static str {
    match tag.trim().to_ascii_lowercase().as_str() {
        "qoder_intl" => INTL,
        _ => CN,
    }
}

/// 账号身份判定（元组键 = 区域 + uid）。
///
/// **国际区与国区是两个账号**：同一个人在两区各有 uid（实测 CN `01a12a3d…` /
/// INTL `019d066e…`），凭证/积分/签到互不通用；即便极端情况下 uid 相同，也不能
/// 当成同一个账号互相覆盖。存量账号 `realm` 为空视为「未定区」，允许被首次识别
/// 到的区域补全（迁移友好），此后即按区锁定。
pub fn same_identity(
    account_realm: &str,
    account_uid: &str,
    realm: &str,
    uid: &str,
) -> bool {
    if uid.is_empty() || account_uid != uid {
        return false;
    }
    account_realm.trim().is_empty() || normalize(account_realm) == normalize(realm)
}

/// 模型 ID 规范化：`qoder-intl/x` → (intl, x)；`qoder/x` / `qoder-cn/x` → (cn, x)；
/// 无前缀 → (None, 原名)，由调用方按 `qoder_default_realm` / Key 绑定决定归属。
/// 返回值第一元 = 显式区域（None = 未指定）。
pub fn normalize_model_id(id: &str) -> (Option<&'static str>, String) {
    let trimmed = id.trim();
    let lower = trimmed.to_ascii_lowercase();
    for (prefix, realm) in [
        ("qoder-intl/", Some(INTL)),
        ("qoder-intl:", Some(INTL)),
        ("qoder-cn/", Some(CN)),
        ("qoder-cn:", Some(CN)),
        ("qoder_cn/", Some(CN)),
        ("qoder/", Some(CN)),
        ("qoder:", Some(CN)),
    ] {
        if lower.starts_with(prefix) {
            let rest = trimmed[prefix.len()..].trim().to_string();
            if !rest.is_empty() {
                return (realm, rest);
            }
            return (None, trimmed.to_string());
        }
    }
    (None, trimmed.to_string())
}

/// 定区优先级（纯函数，handler 与单测共用）：
/// 模型显式区域（`qoder-intl/x`）> Key 绑区（`bind_pool=qoder_cn/qoder_intl`）> 默认国内版。
/// 返回值是**池标签**（`qoder_cn` / `qoder_intl`），直接喂 `ApiSharedState::qoder_pool_for`。
pub fn resolve_pool_tag(model_realm: Option<&str>, key_bind_pool: &str) -> &'static str {
    if let Some(r) = model_realm {
        return pool_tag(r);
    }
    if let Some(r) = crate::api_server::api_keys::pool_realm_of(key_bind_pool) {
        return pool_tag(r);
    }
    pool_tag(CN)
}

/// 池化 uid：`qoder_cn:<uid>` / `qoder_intl:<uid>`（方案 C 第 3 步的落库/调度键）。
///
/// Qoder 池内 uid 目前是裸 uid；升为双池后必须带前缀，否则同 uid 在两区分身会
/// 互相覆盖（额度/冷却/设备指纹）。前缀只影响**池内键**，对外 API 与 UI 仍展示裸 uid。
pub fn uid_with_pool(uid: &str, realm: &str) -> String {
    format!("{}:{}", pool_tag(realm), uid.trim())
}

/// 拆分池化 uid：`qoder_intl:x` → ("qoder_intl", "x")；无前缀 → ("qoder_cn", 原名)
/// （存量裸 uid 一律视为国内版，保证零迁移）。
pub fn split_pooled_uid(value: &str) -> (&'static str, String) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    for tag in ["qoder_intl:", "qoder_cn:", "qoder:"] {
        if lower.starts_with(tag) {
            let rest = trimmed[tag.len()..].trim().to_string();
            if !rest.is_empty() {
                return (realm_of_pool(&tag[..tag.len() - 1]), rest);
            }
            break;
        }
    }
    (pool_tag(CN), trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_defaults_to_cn() {
        assert_eq!(normalize(""), CN);
        assert_eq!(normalize("cn"), CN);
        assert_eq!(normalize("CN"), CN);
        assert_eq!(normalize("intl"), INTL);
        assert_eq!(normalize("Global"), INTL);
        assert_eq!(normalize("whatever"), CN, "未知值按 CN（存量兼容）");
    }

    #[test]
    fn endpoints_follow_realm() {
        assert_eq!(openapi_base(CN), "https://openapi.qoder.com.cn");
        assert_eq!(openapi_base(INTL), "https://openapi.qoder.sh");
        assert_eq!(web_base(CN), "https://qoder.cn");
        assert_eq!(web_base(INTL), "https://qoder.com");
        assert!(chat_url(CN).contains("gateway.qoder.com.cn"));
        assert!(chat_url(INTL).contains("api3.qoder.sh"));
        assert!(model_list_url(INTL).starts_with("https://api3.qoder.sh/"));
        assert_eq!(client_id(INTL), CLIENT_ID_INTL);
        assert_eq!(client_id(CN), CLIENT_ID_CN);
    }

    #[test]
    fn region_mapping() {
        assert_eq!(region(CN), QoderRegion::Cn);
        assert_eq!(region(INTL), QoderRegion::Global);
        assert_eq!(label(INTL), "国际版");
    }

    /// 账号身份键 = (区域, uid)：国际区与国区是两个账号，同 uid 也不得互相覆盖；
    /// 存量账号（realm 空）允许被首次识别到的区域补全（迁移友好）。
    #[test]
    fn identity_is_realm_scoped() {
        // 实测两区 uid（2026-10-18 本机双端登录）：各自独立
        let cn_uid = "01a12a3d-a8ed-7462-87fc-dc2d6e6ac3a4";
        let intl_uid = "019d066e-63b6-72ea-940f-42297049ed2c";
        assert!(same_identity("cn", cn_uid, CN, cn_uid));
        assert!(same_identity("intl", intl_uid, INTL, intl_uid));
        // 跨区不合并（即便 uid 相同）
        assert!(!same_identity("cn", cn_uid, INTL, cn_uid));
        // 同区换 token 仍命中
        assert!(same_identity(CN, cn_uid, CN, cn_uid));
        // 存量账号（realm 空）任一分区都可认领
        assert!(same_identity("", cn_uid, CN, cn_uid));
        assert!(same_identity("", cn_uid, INTL, cn_uid));
        // uid 缺失/不同一律不命中
        assert!(!same_identity("cn", "", CN, ""));
        assert!(!same_identity("cn", cn_uid, CN, intl_uid));
    }

    #[test]
    fn domain_and_dir_inference() {        assert_eq!(from_domain("qoder.com.cn"), CN);
        assert_eq!(from_domain("www.qoder.cn"), CN);
        assert_eq!(from_domain("openapi.qoder.sh"), INTL);
        assert_eq!(from_domain("qoder.com"), INTL);
        assert_eq!(from_dir_name("com.qodercn.app.stable"), CN);
        assert_eq!(from_dir_name("QoderCN"), CN);
        assert_eq!(from_dir_name("com.qoder.app.stable"), INTL);
        assert_eq!(from_dir_name("Qoder CN"), CN);
    }

    #[test]
    fn pool_tags_map_both_ways() {
        assert_eq!(pool_tag(CN), "qoder_cn");
        assert_eq!(pool_tag(INTL), "qoder_intl");
        assert_eq!(pool_tag(""), "qoder_cn");
        assert_eq!(realm_of_pool("qoder_intl"), INTL);
        assert_eq!(realm_of_pool("qoder_cn"), CN);
        assert_eq!(realm_of_pool("qoder"), CN, "兼容别名按 cn");
        assert_eq!(realm_of_pool("unknown"), CN);
    }

    #[test]
    fn pool_tag_priority_prefix_over_key_over_default() {
        // 模型前缀最高：即便 Key 绑国内版，带 intl 前缀仍走国际版池
        assert_eq!(resolve_pool_tag(Some(INTL), "qoder_cn"), "qoder_intl");
        assert_eq!(resolve_pool_tag(Some(CN), "qoder_intl"), "qoder_cn");
        // 无前缀：Key 绑区次之
        assert_eq!(resolve_pool_tag(None, "qoder_intl"), "qoder_intl");
        assert_eq!(resolve_pool_tag(None, "qoder_cn"), "qoder_cn");
        // 未绑区 / 其它池：默认国内版
        assert_eq!(resolve_pool_tag(None, ""), "qoder_cn");
        assert_eq!(resolve_pool_tag(None, "buddy"), "qoder_cn");
    }

    #[test]
    fn model_id_prefix_normalization() {
        assert_eq!(normalize_model_id("qoder-intl/glm-5.3"), (Some(INTL), "glm-5.3".into()));
        assert_eq!(normalize_model_id("Qoder-INTL: glm-5.3"), (Some(INTL), "glm-5.3".into()));
        assert_eq!(normalize_model_id("qoder/glm-5.3"), (Some(CN), "glm-5.3".into()));
        assert_eq!(normalize_model_id("qoder-cn/glm-5.3"), (Some(CN), "glm-5.3".into()));
        // 裸名：不指定区域
        assert_eq!(normalize_model_id(" glm-5.3 "), (None, "glm-5.3".into()));
        // 前缀后为空：视为裸名，避免产生空模型
        assert_eq!(normalize_model_id("qoder/"), (None, "qoder/".into()));
    }

    /// 回归锁（P2 防跨区 failover 死循环）：区域判定是**纯函数映射**，不存在
    /// 「取另一区」的推导——qoder2api-hub 曾因 `"intl" if realm == "cn" else "cn"`
    /// 在 realm=None 时误判「另一区 = 自己」，导致 cn↔cn 无限切换直到超时。
    /// 本实现的 region()/pool_tag() 只由输入 realm 决定，天然无此退化路径，
    /// 此测试把该契约钉死：任何输入（含空串/未知值）都映射到确定的一区。
    #[test]
    fn region_resolution_is_total_and_never_self_referential() {
        for input in ["cn", "intl", "", "  ", "Global", "international", "unknown-zone"] {
            let r = normalize(input);
            // 归一结果只能落在 CN / INTL 两者之一（全函数，无 Option 悬空）
            assert!(r == CN || r == INTL, "normalize({input:?}) = {r}");
            // pool_tag 与 region 同源同区：不可能出现「另一区 = 本区」的歧义
            let tag = pool_tag(r);
            assert_eq!(realm_of_pool(tag), r, "pool_tag/realm_of_pool 必须互逆");
            // region 取 realm（不是 pool_tag）作为输入：由 realm 唯一决定网关区域
            assert_eq!(
                region(r),
                if r == INTL { QoderRegion::Global } else { QoderRegion::Cn },
                "region 必须由 realm 唯一决定"
            );
        }
    }
}
