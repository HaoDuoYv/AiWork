//! OAuth 登录代理直连豁免（F-78 批次 2，issue #10 根因①）。
//! MITM 代理运行时系统代理指向 127.0.0.1:<port>，浏览器 OAuth 登录页流量会被
//! 自签 CA 解密，CA 未受信即报 ERR_CERT_AUTHORITY_INVALID。
//! 发起 OAuth 前把登录/鉴权域名追加进系统代理 ProxyOverride（直连白名单），
//! 登录结束只移除本次追加的条目，绝不触碰用户原有配置。
//! 仅当系统代理确实指向本软件 MITM 端口（data/last_proxy_port.txt 记录）时才改写；
//! 用户自己的远程代理（VPN 等）不会被本软件解密，无需也绝不改动。

use std::path::Path;
use std::sync::Mutex;

/// OAuth 登录链路需直连的域名（登录页 / 鉴权 API / 本机回调）
pub const OAUTH_BYPASS_ENTRIES: &[&str] = &[
    "www.trae.cn",
    "api.trae.cn",
    "api.trae.com.cn",
    "127.0.0.1",
    "localhost",
];

/// 本次已追加进 ProxyOverride 的条目（disable 时只删这些，保证幂等且不误删用户配置）
static ADDED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 崩溃残留标记（enable 时写入 / disable 时删除）：进程异常退出未来得及还原时，
/// 下次启动据此只清理本软件追加过的条目（缺陷13）
fn marker_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("oauth_bypass_pending.json")
}

fn save_marker(data_dir: &Path, added: &[String]) {
    if let Ok(json) = serde_json::to_string(added) {
        let _ = std::fs::write(marker_path(data_dir), json);
    }
}

/// 启动时清理崩溃残留（main.rs setup 调用）：上次进程未正常还原 ProxyOverride 时，
/// 按标记文件只移除本软件追加的条目；无标记则幂等空操作
#[cfg(target_os = "windows")]
pub fn cleanup_residual_bypass(data_dir: &Path) {
    let marker = marker_path(data_dir);
    let Ok(json) = std::fs::read_to_string(&marker) else {
        return;
    };
    let Ok(added) = serde_json::from_str::<Vec<String>>(&json) else {
        // 标记损坏：删掉即可，残留条目（若有）由用户手动处理，不再二次猜测
        let _ = std::fs::remove_file(&marker);
        return;
    };
    if added.is_empty() {
        let _ = std::fs::remove_file(&marker);
        return;
    }
    let key = win::KEY_PATH;
    // 审查修复：读失败（reg 命令执行失败等）必须与「值缺失」区分——此前统一按空表
    // 处理会覆盖写清空用户白名单。读失败保留标记，下次启动重试
    let Ok(enable) = win::reg_query_value(key, "ProxyEnable") else {
        crate::fs_utils::app_log(data_dir, "清理 OAuth 豁免残留：读取 ProxyEnable 失败，保留标记待下次启动重试");
        return;
    };
    if enable.as_deref() == Some("0x1") {
        let Ok(override_val) = win::reg_query_value(key, "ProxyOverride") else {
            crate::fs_utils::app_log(data_dir, "清理 OAuth 豁免残留：读取 ProxyOverride 失败，保留标记待重试（不以空表覆盖写）");
            return;
        };
        let current = override_val.unwrap_or_default();
        let remaining: Vec<&str> = current
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty() && !added.iter().any(|a| a.eq_ignore_ascii_case(s)))
            .collect();
        if win::run_reg(key, "ProxyOverride", "REG_SZ", &remaining.join(";")).is_err() {
            // 失败保留标记，下次启动重试
            return;
        }
        let _ = win::notify_wininet_changed();
    }
    let _ = std::fs::remove_file(&marker);
}

#[cfg(not(target_os = "windows"))]
pub fn cleanup_residual_bypass(_data_dir: &Path) {}

/// 当前生效的 MITM 代理端口（与 commands::proxy 写入的 last_proxy_port.txt 对齐）
fn mitm_port(data_dir: &Path) -> Option<u16> {
    std::fs::read_to_string(data_dir.join("last_proxy_port.txt"))
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
}

/// 发起 OAuth 前启用直连豁免。返回 Ok(true) 表示本次实际修改了系统代理白名单。
#[cfg(target_os = "windows")]
pub fn enable_oauth_bypass(data_dir: &Path) -> Result<bool, String> {
    let key = win::KEY_PATH;

    let Some(port) = mitm_port(data_dir) else {
        // 本软件未启动过 MITM 代理，无需豁免
        return Ok(false);
    };
    // ProxyEnable/ProxyServer 读失败保守按「不匹配」处理（Ok(false) 不改写）：
    // 无法确认系统代理指向本软件时绝不触碰用户配置
    if win::reg_query_value(key, "ProxyEnable")
        .ok()
        .flatten()
        .as_deref()
        != Some("0x1")
    {
        return Ok(false);
    }
    // 仅当系统代理确实指向本软件 MITM 端口时才改写（用户自己的代理不碰）
    let server = win::reg_query_value(key, "ProxyServer")
        .ok()
        .flatten()
        .unwrap_or_default();
    if server != format!("127.0.0.1:{port}") {
        return Ok(false);
    }

    let mut added = ADDED.lock().unwrap_or_else(|e| e.into_inner());
    if !added.is_empty() {
        // 已豁免（幂等）：上次 enable 后未还原
        return Ok(true);
    }
    // 审查修复：ProxyOverride 读失败绝不能按空表覆盖写（会清空用户已有白名单），
    // 报错跳过本次 enable；值缺失（Ok(None)）= 用户本无白名单，按空表继续
    let current = win::reg_query_value(key, "ProxyOverride")
        .map_err(|e| format!("读取系统代理白名单失败，已跳过 OAuth 直连豁免: {e}"))?
        .unwrap_or_default();
    let existing: Vec<String> = current
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let mut merged = existing.clone();
    for e in OAUTH_BYPASS_ENTRIES {
        if !existing.iter().any(|x| x.eq_ignore_ascii_case(e)) {
            merged.push((*e).to_string());
            added.push((*e).to_string());
        }
    }
    if added.is_empty() {
        return Ok(false);
    }
    win::run_reg(key, "ProxyOverride", "REG_SZ", &merged.join(";"))?;
    win::notify_wininet_changed();
    // 持久化追加清单：进程崩溃未还原时下次启动清理（缺陷13）
    save_marker(data_dir, &added);
    Ok(true)
}

/// OAuth 结束后还原直连豁免：只移除本次追加的条目（未修改过则幂等成功）
#[cfg(target_os = "windows")]
pub fn disable_oauth_bypass(data_dir: &Path) -> Result<(), String> {
    let key = win::KEY_PATH;

    let mut added = ADDED.lock().unwrap_or_else(|e| e.into_inner());
    if added.is_empty() {
        return Ok(());
    }
    // 系统代理已被关闭/还原（如 MITM 代理停止时整体还原了快照）时豁免已随之失效，直接清账。
    // 审查修复：读失败不再静默清账——保留 added 清单与标记返 Err 重试，防还原丢失
    // 导致豁免条目滞留系统代理
    let enable = win::reg_query_value(key, "ProxyEnable")
        .map_err(|e| format!("读取 ProxyEnable 失败，保留 OAuth 豁免清单待重试: {e}"))?;
    match enable {
        // ProxyEnable 值缺失 = 系统代理从未开启，豁免天然失效，清账
        None => {}
        Some(v) if v.as_str() == "0x1" => {
            let Ok(override_val) = win::reg_query_value(key, "ProxyOverride") else {
                return Err("读取 ProxyOverride 失败，保留 OAuth 豁免清单待重试（不以空表覆盖写）".into());
            };
            let current = override_val.unwrap_or_default();
            let remaining: Vec<&str> = current
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty() && !added.iter().any(|a| a.eq_ignore_ascii_case(s)))
                .collect();
            win::run_reg(key, "ProxyOverride", "REG_SZ", &remaining.join(";"))?;
            win::notify_wininet_changed();
        }
        // 系统代理已关闭：豁免失效，清账
        Some(_) => {}
    }
    added.clear();
    let _ = std::fs::remove_file(marker_path(data_dir));
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn enable_oauth_bypass(_data_dir: &Path) -> Result<bool, String> {
    // 非 Windows 平台暂无 MITM 系统代理设置逻辑，无需豁免
    Ok(false)
}

#[cfg(not(target_os = "windows"))]
pub fn disable_oauth_bypass(_data_dir: &Path) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows 注册表工具（与 commands::proxy 同款实现；该模块函数为私有，此处复刻）
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
mod win {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    pub(super) fn reg_query_value(key: &str, name: &str) -> Result<Option<String>, String> {
        let out = Command::new("reg")
            .args(["query", key, "/v", name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("reg query 执行失败: {e}"))?;
        if !out.status.success() {
            // 退出码 1 = 值不存在（正常语义），不视为错误
            if out.status.code() == Some(1) {
                return Ok(None);
            }
            return Err(format!(
                "reg query 异常退出(code={:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text
            .lines()
            .find(|l| l.trim_start().starts_with(name))
            .and_then(|l| l.split_whitespace().last().map(|s| s.to_string())))
    }

    pub(super) fn run_reg(key: &str, name: &str, ty: &str, value: &str) -> Result<(), String> {
        let out = Command::new("reg")
            .args(["add", key, "/v", name, "/t", ty, "/d", value, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).to_string())
        }
    }

    /// 通知 WinINet 系统代理设置已变化（否则浏览器可能继续用旧配置）
    pub(super) fn notify_wininet_changed() {
        extern "system" {
            fn InternetSetOptionW(
                h_internet: *mut std::ffi::c_void,
                option: u32,
                buffer: *mut std::ffi::c_void,
                buffer_length: u32,
            ) -> i32;
        }
        const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;
        const INTERNET_OPTION_REFRESH: u32 = 37;

        unsafe {
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_SETTINGS_CHANGED,
                std::ptr::null_mut(),
                0,
            );
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_REFRESH,
                std::ptr::null_mut(),
                0,
            );
        }
    }

    pub(super) const KEY_PATH: &str = KEY;
}
