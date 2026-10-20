//! WorkBuddy 共享底层（原 workbuddy.rs 机械拆分）：路径常量、账号池/凭证库/设置读写、
//! 字段提取、进程检测、续期互斥锁、会话 uid 防护、CLI 设置回滚等被多域复用的辅助。
//! 函数逻辑零改动，仅将跨子模块引用项提升为 `pub(super)`。

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tauri::{AppHandle, State};

use crate::fs_utils;
use crate::state::AppState;

// ── 路径常量（与 wb_common.py / trae-switch-bridge.ps1 保持一致）────────────

pub(super) fn auth_file_path() -> PathBuf {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    PathBuf::from(local)
        .join("CodeBuddyExtension")
        .join("Data")
        .join("Public")
        .join("auth")
        .join("workbuddy-desktop.info")
}

/// auth 文件读取路径：settings.wb_auth_file_path 人工指定优先（环境配置页），否则默认布局。
/// 仅作用于读取类路径（检测/列表/导入/续期/用量）；环境重置等清理动作仍针对客户端真实落盘位置。
pub(super) fn auth_file_path_of(state: &AppState) -> PathBuf {
    if let Some(p) = state.settings().wb_auth_file_path.as_deref() {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    auth_file_path()
}

/// auth 目录下**全部**候选凭据文件（人工指定 → 该文件；否则按 mtime 倒序的 `*.info`）。
///
/// 背景（issue：扫描本机账号失败）：WorkBuddy 桌面端同时存在多份凭据文件——
/// - `workbuddy-desktop.info`（CN：自 2026-09-24 起 `accessToken` 改 `$wbEncrypted` 信封）
/// - `workbuddy-desktop-ai.info`（INTL：仍是明文 JWT）
/// - `<name>.<timestamp>.<pid>.<uuid>.info`（客户端备份）
///
/// 旧实现只认第一份（加密那份），于是「扫描本机账号」必然失败。这里改为枚举全部，
/// 由调用方逐个尝试、跳过信封，命中明文即导入。
pub(super) fn auth_file_candidates(state: &AppState) -> Vec<PathBuf> {
    if let Some(p) = state.settings().wb_auth_file_path.as_deref() {
        let p = p.trim();
        if !p.is_empty() {
            return vec![PathBuf::from(p)];
        }
    }
    let default = auth_file_path();
    let Some(dir) = default.parent() else {
        return vec![default];
    };
    let mut found: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_info = path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("info"))
                .unwrap_or(false);
            if !is_info {
                continue;
            }
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            found.push((path, mtime));
        }
    }
    if found.is_empty() {
        return vec![default];
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found.into_iter().map(|(p, _)| p).collect()
}

/// 默认读取的 auth 文件：候选里**最新写入**的一份（客户端正在用的那份）。
///
/// 历史语义是「第一个不是 at-rest 信封的文件」——那时国区文件解不开、只能退到
/// 国际区明文文件；现在信封可解密（`read_auth_file`），再按「绕过加密」挑文件
/// 会让环境检测永远指向国际区那份，与实际登录态不符。
pub(super) fn auth_file_path_of_readable(state: &AppState) -> PathBuf {
    let candidates = auth_file_candidates(state);
    candidates
        .iter()
        .find(|p| p.is_file())
        .cloned()
        .unwrap_or_else(|| auth_file_path_of(state))
}

/// 探测 `accessToken` 是否被客户端 at-rest 加密（`{"$wbEncrypted":1,"envelope":"..."}`）。
/// 命中返回信封所在字段名（当前恒为 `auth.accessToken`），未命中/缺失返回 None。
/// 保留为诊断辅助（真正解密走 `read_auth_file`）。
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn access_token_envelope(raw: &serde_json::Value) -> Option<&'static str> {
    let v = raw.get("auth").and_then(|a| a.get("accessToken"))?;
    if crate::tasks::wb_atrest::is_envelope(v) {
        return Some("auth.accessToken");
    }
    None
}

// ── 区域识别（国际区 / 国区是两个独立账号体系）─────────────────────────────
//
// 实测（2026-10-18，本机双端同时登录）：
// - `workbuddy-desktop-ai.info` = 国际区（`auth.domain = www.workbuddy.ai`，uid/nickname
//   明文，计费走 `www.workbuddy.ai`）；
// - `workbuddy-desktop.info` = 国区（`auth.domain = www.workbuddy.cn`，token/昵称/手机号
//   为 `$wbEncrypted` 信封，计费走 `www.codebuddy.cn`）；
// 两者 uid 不同、凭证不通用，必须作为**两个账号**入池（不得按 uid/文件名互相覆盖）。

/// 由 auth 文件路径 + `auth.domain` 判定区域：`intl` | `cn`（未知一律 cn，存量兼容）。
pub(super) fn realm_of_auth_file(path: &std::path::Path, raw: &serde_json::Value) -> &'static str {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.contains("-ai.") || name.contains("-ai.info") {
        return "intl";
    }
    let domain = as_str(fs_utils::dig(raw, &["domain"]))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if domain.contains("workbuddy.ai") || domain.contains("codebuddy.ai") {
        "intl"
    } else {
        "cn"
    }
}

/// 区域展示名（UI/日志）
#[cfg_attr(not(test), allow(dead_code))]
pub fn realm_label(realm: &str) -> &'static str {
    if realm == "intl" {
        "国际区"
    } else {
        "国区"
    }
}

/// auth 文件读取产物（明文凭证 + 账号身份 + 区域；凭证字段不回传前端）
pub(super) struct AuthFileCreds {
    pub access_token: String,
    pub refresh_token: String,
    pub uid: String,
    pub nickname: String,
    pub edition_type: String,
    pub domain: String,
    pub realm: String,
    pub access_token_expires_at: Option<i64>,
    pub refresh_token_expires_at: Option<i64>,
    /// 读取时是否解开了 at-rest 信封（诊断展示）
    pub decrypted_at_rest: bool,
}

/// 读取一份 auth 文件（自动解开 at-rest 信封）。失败返回可执行的中文原因。
///
/// 密钥来源见 `tasks::wb_atrest::recover_key`：环境变量 / 数据目录 `wb_atrest.key` /
/// 运行中客户端进程内存。国区账号可识别的前提就是这一步能拿到密钥。
pub(super) fn read_auth_file(state: &AppState, path: &std::path::Path) -> Result<AuthFileCreds, String> {
    let raw = fs_utils::read_json::<serde_json::Value>(path);
    if !raw.is_object() {
        return Err(format!("{}：JSON 解析失败", path.display()));
    }
    let realm = realm_of_auth_file(path, &raw).to_string();
    let domain = as_str(fs_utils::dig(&raw, &["domain"])).unwrap_or_else(|| {
        if realm == "intl" {
            "www.workbuddy.ai".to_string()
        } else {
            "www.workbuddy.cn".to_string()
        }
    });
    let access_raw = fs_utils::dig(&raw, &["accessToken", "access_token", "token"])
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let encrypted = crate::tasks::wb_atrest::is_envelope(&access_raw);
    let mut decrypted_at_rest = false;
    let key = if encrypted {
        let data_dir = state.data_dir.clone();
        let log_dir = data_dir.clone();
        let key = crate::tasks::wb_atrest::recover_key(Some(&data_dir), &|msg| {
            fs_utils::app_log(&log_dir, msg)
        })
        .map_err(|e| format!("{}：{e}", path.display()))?;
        decrypted_at_rest = true;
        Some(key)
    } else {
        None
    };
    let decode = |v: Option<&serde_json::Value>| -> String {
        let Some(v) = v else { return String::new() };
        match &key {
            Some(k) => crate::tasks::wb_atrest::decrypt_field(k, v).unwrap_or_default(),
            None => v.as_str().unwrap_or("").to_string(),
        }
    };
    let access_token = decode(fs_utils::dig(&raw, &["accessToken", "access_token", "token"]));
    if access_token.is_empty() {
        // 失败分类与 issue #58 同口径：区分「未登录（null/空串）」与「结构变更」
        return Err(match extract_access_token(&raw) {
            Err(e) => format!("{}：{e}", path.display()),
            Ok(_) => format!("{}：accessToken 解密后为空", path.display()),
        });
    }
    let refresh_token = decode(fs_utils::dig(&raw, &["refreshToken", "refresh_token"]));
    let uid = as_str(fs_utils::dig(&raw, &["uid"])).unwrap_or_default();
    let nickname = decode(fs_utils::dig(&raw, &["nickname", "displayName", "name"]));
    Ok(AuthFileCreds {
        access_token,
        refresh_token,
        uid,
        nickname,
        edition_type: as_str(fs_utils::dig(&raw, &["editionType", "edition"])).unwrap_or_default(),
        domain,
        realm,
        access_token_expires_at: as_ts_seconds(fs_utils::dig(
            &raw,
            &["expiresAtMs", "expires_at_ms", "expiresAt", "expires_in_ms", "accessTokenExpiresAtMs"],
        )),
        refresh_token_expires_at: as_ts_seconds(fs_utils::dig(
            &raw,
            &["refreshExpiresAt", "refresh_expires_at"],
        )),
        decrypted_at_rest,
    })
}



pub(super) fn wb_data_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join(".workbuddy")
}

pub(super) fn snapshot_json_path() -> PathBuf {
    wb_data_dir().join("storage").join("skeleton").join("account-snapshot.json")
}

pub(super) fn account_id_of(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let digest = h.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("wb-{}", &hex[..12])
}

// ── 数据结构 ────────────────────────────────────────────────────────────────

/// 账号池记录（§3.3 结构）
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct WorkBuddyAccount {
    /// wb-<sha256(token) 前 12 位>（同 token 稳定同 id，F-04）
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub phone_masked: String,
    #[serde(default)]
    pub edition_type: String,
    /// 区域：`intl`（国际区，www.workbuddy.ai）| `cn`（国区，www.codebuddy.cn）。
    /// 空 = 存量账号（按国区口径处理）。国际区与国区是**两套独立账号体系**，
    /// uid/token 都不通用，身份判定必须带上区域，否则同名会互相覆盖。
    #[serde(default)]
    pub realm: String,
    /// accessToken 过期时间（Unix 秒；刷新/导入时回填）
    #[serde(default)]
    pub access_token_expires_at: Option<i64>,
    #[serde(default)]
    pub refresh_token_expires_at: Option<i64>,
    #[serde(default)]
    pub auth_saved_at: Option<i64>,
    #[serde(default)]
    pub needs_relogin: bool,
    #[serde(default)]
    pub relogin_reason: String,
    #[serde(default)]
    pub group_id: String,
    #[serde(default)]
    pub note: String,
    /// 余额缓存（credits_fetch 成功后回写，供列表/概述展示）
    #[serde(default)]
    pub credits_balance: Option<f64>,
    #[serde(default)]
    pub credits_fetched_at: Option<String>,
    /// 最早积分包到期时间缓存（Unix 秒；积分查询/每日快照回写）：
    /// 剩余>0 且未过期包取 min，Buddy 不分包类型（issue #28 调度口径）；
    /// None = 无到期信息或全部包长期有效，键值随每次回写覆盖（不留 stale）
    #[serde(default)]
    pub credits_expire_at: Option<i64>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(super) struct WbPool {
    #[serde(default)]
    pub(super) accounts: Vec<WorkBuddyAccount>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct WorkBuddySettings {
    /// 启动自动补签（F-55）：启动时核验未签账号自动补签
    #[serde(default)]
    pub auto_checkin: bool,
    /// 保活阈值（天）；0 = 每天无条件刷新全部带 refreshToken 账号
    #[serde(default)]
    pub keepalive_days: i64,
    /// 惰性刷新（小时）：剩余有效期低于该值才刷新
    #[serde(default = "default_lazy_hours")]
    pub lazy_refresh_hours: i64,
    // 成长中心开关（F-17，批次2 消费；先落配置）
    #[serde(default = "default_true")]
    pub growth_travel: bool,
    #[serde(default = "default_true")]
    pub growth_lottery: bool,
    #[serde(default = "default_true")]
    pub growth_tasks: bool,
    // CLI 五重防护自动轮换（F-59，批次3 T3.4）
    #[serde(default)]
    pub cli_rotate_enabled: bool,
    /// 检查间隔（分钟，后台线程周期触发）
    #[serde(default = "default_cli_interval")]
    pub cli_rotate_interval_minutes: i64,
    /// ① 冷却期：切换后 N 分钟内不切
    #[serde(default = "default_cli_cooldown")]
    pub cli_cooldown_minutes: i64,
    /// ② 到期差异阈值：目标比当前早到期超过 N 小时才切（防横跳）
    #[serde(default = "default_cli_gap")]
    pub cli_min_gap_hours: i64,
    /// ③ 到期紧迫阈值：目标剩余超过 N 小时 = 都还早，不切
    #[serde(default = "default_cli_urgency")]
    pub cli_min_urgency_hours: i64,
    /// ④ 活跃保护：CLI 最近会话写入 N 分钟内不切
    #[serde(default = "default_cli_guard")]
    pub cli_active_guard_minutes: i64,
    /// ⑤ 最小剩余积分：目标低于该值不切（0 = 关闭）
    #[serde(default)]
    pub cli_min_remaining_credits: f64,
    // 失败通知渠道（F-19）：已迁移至 app Settings（通知渠道面板，Trae/Buddy 全平台共用）
    // UI 坐标点击签到兜底（F-18，批次4 T4.2）：仅手动触发，默认关闭
    #[serde(default)]
    pub ui_click_enabled: bool,
    /// 签到按钮屏幕坐标（0 = 未配置）
    #[serde(default)]
    pub ui_click_x: i64,
    #[serde(default)]
    pub ui_click_y: i64,
}

fn default_lazy_hours() -> i64 {
    24
}
fn default_true() -> bool {
    true
}
fn default_cli_interval() -> i64 {
    30
}
fn default_cli_cooldown() -> i64 {
    120
}
fn default_cli_gap() -> i64 {
    24
}
fn default_cli_urgency() -> i64 {
    72
}
fn default_cli_guard() -> i64 {
    30
}

impl WorkBuddySettings {
    fn with_defaults() -> Self {
        Self {
            auto_checkin: false,
            keepalive_days: 0,
            lazy_refresh_hours: 24,
            growth_travel: true,
            growth_lottery: true,
            growth_tasks: true,
            cli_rotate_enabled: false,
            cli_rotate_interval_minutes: 30,
            cli_cooldown_minutes: 120,
            cli_min_gap_hours: 24,
            cli_min_urgency_hours: 72,
            cli_active_guard_minutes: 30,
            cli_min_remaining_credits: 0.0,
            ui_click_enabled: false,
            ui_click_x: 0,
            ui_click_y: 0,
        }
    }
}

// ── 工具函数 ────────────────────────────────────────────────────────────────

pub(super) fn load_pool(state: &AppState) -> WbPool {
    // SQLite 化（P3）：workbuddy_accounts.json → wb_accounts 表
    let v = crate::store::docs::wb_pool_load(&crate::store::db(&state.data_dir));
    serde_json::from_value(v).unwrap_or_default()
}

pub(super) fn save_pool(state: &AppState, pool: &WbPool) -> Result<(), String> {
    let v = serde_json::to_value(pool).map_err(|e| format!("序列化失败: {e}"))?;
    crate::store::docs::wb_pool_save(&crate::store::db(&state.data_dir), &v)
}

pub(crate) fn load_settings(state: &AppState) -> WorkBuddySettings {
    // SQLite 化（P2）：workbuddy_settings.json → kv `workbuddy_settings`
    let mut s: WorkBuddySettings = crate::store::db(&state.data_dir).kv_get("workbuddy_settings");
    // 审查 P2：单字段非法只钳制该字段为默认值，不再整体 with_defaults() 重置
    //（避免损坏一个字段连带丢掉轮换/通知等其余配置）
    if s.lazy_refresh_hours <= 0 {
        s.lazy_refresh_hours = WorkBuddySettings::with_defaults().lazy_refresh_hours;
    }
    s
}

/// 失败通知统一入口（F-19；通知渠道面板，Trae/Buddy 全平台共用）：
/// 桌面通知（有 AppHandle 时）+ Bark/通用 Webhook/Server酱 可选渠道。
/// 配置来自 app Settings（kv `app_settings`，通知渠道面板维护）；
/// 总开关 + 事件开关过滤；渠道失败静默记日志，不影响主流程。
pub fn push_notify(app: Option<&AppHandle>, data_dir: &std::path::Path, title: &str, body: &str, event: crate::notify::NotifyEvent) {
    let s: crate::models::Settings = crate::store::db(data_dir).kv_get("app_settings");
    if !s.notify_enabled {
        return;
    }
    let on = match event {
        crate::notify::NotifyEvent::CheckinDone => s.notify_on_checkin,
        crate::notify::NotifyEvent::TaskFail => s.notify_on_task_fail,
        crate::notify::NotifyEvent::Other => true,
    };
    if !on {
        return;
    }
    let channels = crate::notify::NotifyChannels {
        bark_url: s.notify_bark_url.clone().filter(|x| !x.trim().is_empty()),
        wechat_webhook: s.notify_webhook_url.clone().filter(|x| !x.trim().is_empty()),
        serverchan_sendkey: s.notify_serverchan_sendkey.clone().filter(|x| !x.trim().is_empty()),
    };
    crate::notify::notify_all(app, data_dir, title, body, &channels);
}

pub(super) fn is_running() -> bool {
    // 原 tasklist 子进程（单次实测 ~260ms）改为 sysinfo 进程表枚举 + 短 TTL 复用
    // （见 switcher::proc::any_running）；概览页与顶栏同轮切换会各问一次
    crate::switcher::proc::any_running(&["WorkBuddy"])
}

// 宽容字段提取统一走 fs_utils::dig（含 data/result/resp/response/info/auth/account
// 包裹键下钻，与新版客户端 auth 文件嵌套结构 {account, auth} 兼容；审查 P2：消除本地重复实现漂移）。

pub(super) fn as_str(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(|x| x.as_str()).map(|s| s.to_string())
}

pub(super) fn as_ts_seconds(v: Option<&serde_json::Value>) -> Option<i64> {
    let raw = v?;
    if let Some(ms) = raw.as_i64() {
        // expiresAtMs 毫秒级（>1e12），统一折算秒
        return Some(if ms > 1_000_000_000_000 { ms / 1000 } else { ms });
    }
    if let Some(f) = raw.as_f64() {
        return Some(if f > 1_000_000_000_000.0 { (f / 1000.0) as i64 } else { f as i64 });
    }
    raw.as_str().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp())
            .or_else(|| s.parse::<i64>().ok().map(|ms| if ms > 1_000_000_000_000 { ms / 1000 } else { ms }))
    })
}

/// 值类型名（诊断用，绝不含值本身）
fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// 诊断用键名提取（issue #51）：仅列键名 + 值类型（绝不含值），顶层 + auth/account 一层子对象。
/// 供「未找到 accessToken」类错误自带结构线索，用户截图即可定位，省去跑 PowerShell 往返。
/// 类型标注（issue #58）：键存在但值为 null 时「未找到」报错与键名清单自相矛盾，
/// 附类型后用户截图可直接看出 accessToken（null）→ 未登录，而非结构变更。
pub(super) fn auth_key_names(raw: &serde_json::Value) -> String {
    if !raw.is_object() {
        return "（非 JSON 对象）".into();
    }
    let mut names: Vec<String> = Vec::new();
    if let Some(map) = raw.as_object() {
        names.extend(map.iter().map(|(k, v)| format!("{k}（{}）", json_type_name(v))));
        for wk in ["auth", "account"] {
            if let Some(child) = map.get(wk).and_then(|v| v.as_object()) {
                names.extend(
                    child
                        .iter()
                        .map(|(k, v)| format!("{wk}.{k}（{}）", json_type_name(v))),
                );
            }
        }
    }
    if names.is_empty() { "（对象无键）".into() } else { names.join(", ") }
}

/// auth 文件 access token 提取 + 失败原因分类（issue #58）：
/// 对候选键逐个 dig，命中即决（语义与 dig 一致，null 不回退尝试后续候选键，避免误采其他 token 字段串号）：
/// 命中且为非空字符串 → Ok；命中但为 null/非字符串/空串 → 「未登录/凭证为空」类错误（不再误报结构变更）；
/// 全部候选键未命中 → 「结构可能已变更」+ 键名诊断（含值类型）。
pub(super) fn extract_access_token(raw: &serde_json::Value) -> Result<String, String> {
    for k in ["accessToken", "access_token", "token"] {
        let Some(v) = fs_utils::dig(raw, &[k]) else { continue };
        if let Some(s) = v.as_str() {
            if !s.is_empty() {
                return Ok(s.to_string());
            }
            return Err(format!(
                "auth 文件 accessToken 为空字符串（可能未登录）：键 {k} 存在但无值，请先在 WorkBuddy 客户端登录后重试"
            ));
        }
        return Err(format!(
            "auth 文件凭证不可用：键 {k} 存在但值为 {}（预期字符串；null 多为未登录或客户端已清空登录态），请先在 WorkBuddy 客户端登录后重试",
            json_type_name(v)
        ));
    }
    Err(format!(
        "auth 文件中未找到 accessToken（结构可能已变更）；实际键名: {}",
        auth_key_names(raw)
    ))
}

// ── 设置（F-55 配置化）──────────────────────────────────────────────────────

#[tauri::command]
pub fn workbuddy_settings_get(state: State<AppState>) -> WorkBuddySettings {
    load_settings(&state)
}

#[tauri::command]
pub fn workbuddy_settings_set(state: State<AppState>, patch: WorkBuddySettings) -> Result<(), String> {
    crate::store::db(&state.data_dir).kv_set("workbuddy_settings", &patch)
}

// ── 工具侧凭证副本写入（F-10 双源化）───────────────────────────────────────

pub(super) fn upsert_token_store(state: &AppState, id: &str, creds: &serde_json::Value) -> Result<(), String> {
    // H-1：表级读改写互斥（与 tasks 侧 save_token_store 共用 WB_TOKEN_STORE_LOCK，
    // 导入/OAuth 与签到/积分刷新并发写不互相覆盖）
    let _table =
        crate::tasks::wb_common::WB_TOKEN_STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 凭证收敛（P0-1）：读走 secure 回填，写走 secure 占位（敏感字段进 vault，DB 不落明文）
    let existing = crate::tasks::wb_common::token_store_load_secure(&state.data_dir);
    let mut rec = existing
        .get("tokens")
        .and_then(|t| t.get(id))
        .cloned()
        .unwrap_or(serde_json::json!({}));
    if let Some(rm) = rec.as_object_mut() {
        for (k, v) in creds.as_object().unwrap_or(&serde_json::Map::new()) {
            if !v.is_null() {
                rm.insert(k.clone(), v.clone());
            }
        }
        rm.insert("updated_at".into(), serde_json::json!(fs_utils::now_iso()));
    }
    crate::tasks::wb_common::token_store_upsert_secure(&state.data_dir, id, &rec)
}

// ── M3 凭证续期互斥（F-09，Rust 侧手动触发；schtasks 每周兜底走 python --renew-only）──

/// 每账号续期互斥（审查 P1）：外层 std Mutex 只保护锁表本身（短临界区），
/// 内层 tokio Mutex 才是账号级互斥——try_lock 拿不到即拒绝，不排队不阻塞。
/// 锁表用 OnceLock 惰性初始化（HashMap::new 非 const，无法直接用于 static）。
pub(super) fn wb_renew_locks(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    LOCKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

// ── CLI settings.json 回滚（审查 P2 原子写）────────────────────────────────

/// 回滚 settings.json 到写入前原文（审查 P2 原子写）：
/// JSON 内容走 fs_utils::write_json（临时文件 + rename，防断电/崩溃时文件处于半写损坏态）；
/// 原文非法 JSON（历史脏数据）或序列化路径失败时，退化为「临时文件 + rename」原文回写。
pub(super) fn restore_cli_settings(previous: &str) {
    let path = cli_settings_path();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(previous) {
        if fs_utils::write_json(&path, &v).is_ok() {
            return;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("json.rollback.{}.{}", std::process::id(), nanos));
    let wrote = std::fs::File::create(&tmp).and_then(|mut f| {
        use std::io::Write as _;
        f.write_all(previous.as_bytes())?;
        f.flush()
    });
    if wrote.is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// CodeBuddy CLI settings.json 路径（.codebuddy/settings.json）。
pub(super) fn cli_settings_path() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join(".codebuddy").join("settings.json")
}

// ── M8 会话三件套命令的 uid 防护（审查 P0-1）───────────────────────────────

/// 会话三件套命令的 user_id 入参防护（审查 P0-1）：路径段只允许池内账号 id，
/// 且必须匹配白名单字符集——杜绝 `..`/绝对路径/分隔符注入导致的目录逃逸
/// （backup 对该路径有 remove_dir_all，逃逸即任意目录删除，restore/info 可读任意路径）。
pub(super) fn wb_chat_uid_guard(state: &AppState, user_id: &str) -> Result<(), String> {
    if user_id.is_empty()
        || user_id.len() > 64
        || !user_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || user_id.contains("..")
    {
        return Err(format!("非法账号标识: {user_id}"));
    }
    let pool = load_pool(state);
    if !pool.accounts.iter().any(|a| a.id == user_id) {
        return Err(format!("账号不在池中: {user_id}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::NotifyEvent;

    // ── 集成测试：kv app_settings → push_notify 门控 → 渠道分发 → app_log ──
    // 渠道统一配无效值（格式校验必失败 → 落日志），用日志作为「已尝试发送」的可观测信号，全程不触网。

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("twa_push_notify_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn seed_app_settings(dir: &std::path::Path, json: &str) {
        crate::store::db(dir).kv_set_raw("app_settings", json).unwrap();
    }

    fn read_log(dir: &std::path::Path) -> String {
        std::fs::read_to_string(dir.join("logs").join("app.log")).unwrap_or_default()
    }

    fn assert_sent(dir: &std::path::Path) {
        let log = read_log(dir);
        assert!(log.contains("通知渠道(Bark)失败"), "log: {log}");
        assert!(log.contains("通知渠道(企业微信)失败"), "log: {log}");
        assert!(log.contains("通知渠道(Server酱)失败"), "log: {log}");
    }

    fn assert_not_sent(dir: &std::path::Path) {
        assert!(!read_log(dir).contains("通知渠道"), "不应有任何渠道发送记录");
    }

    /// 区域识别（国际区 / 国区是两个账号）：文件名与 auth.domain 两条证据都要认。
    /// 实测样本：`workbuddy-desktop-ai.info` = intl；`workbuddy-desktop.info` = cn。
    #[test]
    fn realm_of_auth_file_reads_name_and_domain() {
        let intl_name = std::path::Path::new("C:/x/workbuddy-desktop-ai.info");
        let cn_name = std::path::Path::new("C:/x/workbuddy-desktop.info");
        // 文件名优先（-ai → intl），即便 domain 缺失
        assert_eq!(realm_of_auth_file(intl_name, &serde_json::json!({})), "intl");
        // domain 证据：workbuddy.ai → intl
        assert_eq!(
            realm_of_auth_file(
                std::path::Path::new("C:/x/whatever.info"),
                &serde_json::json!({"auth": {"domain": "www.workbuddy.ai"}})
            ),
            "intl"
        );
        // 国区两条证据都指向 cn
        assert_eq!(realm_of_auth_file(cn_name, &serde_json::json!({})), "cn");
        assert_eq!(
            realm_of_auth_file(
                std::path::Path::new("C:/x/backup.info"),
                &serde_json::json!({"auth": {"domain": "www.workbuddy.cn"}})
            ),
            "cn"
        );
    }

    /// 总开关关闭：事件开关/渠道配置再齐也全静默
    #[test]
    fn push_notify_total_switch_off_blocks_all() {
        let dir = tmp_dir("total_off");
        seed_app_settings(
            &dir,
            r#"{"notify_enabled":false,"notify_bark_url":"http://b","notify_webhook_url":"ftp://w","notify_serverchan_sendkey":"bad key"}"#,
        );
        push_notify(None, &dir, "标题", "内容", NotifyEvent::TaskFail);
        assert_not_sent(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 事件门控按类别路由：TaskFail 关 → 静默；CheckinDone 开 → 发出
    #[test]
    fn push_notify_event_gate_routes_by_event_kind() {
        let dir = tmp_dir("event_gate");
        seed_app_settings(
            &dir,
            r#"{"notify_enabled":true,"notify_on_checkin":true,"notify_on_task_fail":false,"notify_bark_url":"http://b","notify_webhook_url":"ftp://w","notify_serverchan_sendkey":"bad key"}"#,
        );
        push_notify(None, &dir, "标题", "内容", NotifyEvent::TaskFail);
        assert_not_sent(&dir);
        push_notify(None, &dir, "标题", "内容", NotifyEvent::CheckinDone);
        assert_sent(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Other 事件不受事件开关限制（手动操作结果始终尝试推送）
    #[test]
    fn push_notify_other_event_bypasses_event_switches() {
        let dir = tmp_dir("other");
        seed_app_settings(
            &dir,
            r#"{"notify_enabled":true,"notify_on_checkin":false,"notify_on_task_fail":false,"notify_bark_url":"http://b","notify_webhook_url":"ftp://w","notify_serverchan_sendkey":"bad key"}"#,
        );
        push_notify(None, &dir, "标题", "内容", NotifyEvent::Other);
        assert_sent(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 渠道值全空白：push_notify 过滤后 is_empty 短路（无日志、不 panic）
    #[test]
    fn push_notify_blank_channel_values_skipped() {
        let dir = tmp_dir("blank");
        seed_app_settings(
            &dir,
            r#"{"notify_enabled":true,"notify_on_checkin":true,"notify_bark_url":"   ","notify_webhook_url":"","notify_serverchan_sendkey":"  "}"#,
        );
        push_notify(None, &dir, "标题", "内容", NotifyEvent::CheckinDone);
        assert_not_sent(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// kv 完全缺失：Settings 走 serde 默认（notify_enabled 默认开）→ 渠道 None → 静默短路，不 panic
    #[test]
    fn push_notify_kv_missing_defaults_silent() {
        let dir = tmp_dir("no_kv");
        push_notify(None, &dir, "标题", "内容", NotifyEvent::TaskFail);
        assert_not_sent(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
