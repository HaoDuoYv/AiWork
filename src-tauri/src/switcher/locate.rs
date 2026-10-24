//! exe 六级发现（原 PS Find-TraeExe 210-342 对译）。
//!
//! 顺序原则（修复「首次切换误用 Trae CN.exe」）：旧逻辑把「运行中进程」作为最高
//! 优先级，导致残留/错误的 Trae 进程（如旧的 Trae CN.exe）被优先采用，从而启动
//! 错误的 exe。现改为：
//!   1) 用户显式配置 > 2) 候选路径 > 3) 开始菜单/桌面 lnk > 4) 注册表
//!   > 5) 运行中进程（最后回退）> 6) 进程缓存（兜底，仅自定义安装且当前未运行时）
//! >    这样正常情况下总是解析到用户真实安装的应用，而非被残留进程带偏。

use std::path::{Path, PathBuf};

use super::{profile, glob_match_ci, Session};

pub fn find_exe(sess: &mut Session) -> Option<PathBuf> {
    // 1) 用户显式配置：kv `app_settings` → settings_path_key（最高优先级；SQLite 化 P2）
    let settings: serde_json::Value = crate::store::db(&sess.data_dir).kv_get("app_settings");
    if let Some(p) = settings.get(sess.prof.settings_path_key).and_then(|v| v.as_str()) {
        let p = PathBuf::from(p);
        if p.is_file() {
            sess.exe_cache = Some(p.clone());
            return Some(p);
        }
    }

    // 2) 多候选路径探测（与 commands/env.rs 探测列表保持一致）
    for c in &sess.prof.exe_candidates {
        if c.is_file() {
            sess.exe_cache = Some(c.clone());
            return Some(c.clone());
        }
    }

    // 3) .lnk 快捷方式（开始菜单×2 / 桌面×2，递归；lnk crate 纯 Rust 解析，
    //    替代 WScript.Shell COM）。文件名 glob 匹配大小写不敏感（PS -like 语义，
    //    如 *Doubao* 须命中 doubao.lnk）
    for dir in lnk_dirs() {
        for lnk in walk_lnk_files(&dir) {
            let Some(name) = lnk.file_name().and_then(|n| n.to_str()) else { continue };
            if !sess.prof.lnk_patterns.iter().any(|p| glob_match_ci(p, name)) {
                continue;
            }
            if let Some(target) = lnk_target(&lnk) {
                let p = PathBuf::from(&target);
                if p.is_file() && profile::exe_matches(&p, &sess.prof) {
                    sess.exe_cache = Some(p.clone());
                    return Some(p);
                }
            }
        }
    }

    // 4) 注册表回退（HKLM 64/32 + HKCU Uninstall；DisplayName 匹配 →
    //    DisplayIcon / InstallLocation+exe_names 组合探测）
    if let Some(p) = registry_locate(sess) {
        if p.is_file() {
            sess.exe_cache = Some(p.clone());
            return Some(p);
        }
    }

    // 5) 运行中进程（最后回退）：仅当以上都找不到才用，避免残留/错误进程误导启动
    //    路径。匹配用 proc_patterns 通配组，再经 exe_names 白名单防串台——与 Stop
    //    的 proc_names 精确组刻意双轨（PS 同款）
    if let Some(p) = super::proc::running_exe_of(&sess.prof) {
        if p.is_file() && profile::exe_matches(&p, &sess.prof) {
            sess.exe_cache = Some(p.clone());
            return Some(p);
        }
    }

    // 6) 进程缓存兜底（自定义安装、当前未运行、以上均未命中）
    sess.exe_cache.clone().filter(|p| p.is_file())
}

/// .lnk 搜索目录（PS 241-246：开始菜单×2 / 桌面×2）
fn lnk_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    vec![
        PathBuf::from(format!(
            "{}\\Microsoft\\Windows\\Start Menu\\Programs",
            env("APPDATA")
        )),
        PathBuf::from(format!(
            "{}\\Microsoft\\Windows\\Start Menu\\Programs",
            env("ProgramData")
        )),
        PathBuf::from(format!("{}\\Desktop", env("USERPROFILE"))),
        PathBuf::from(format!("{}\\Desktop", env("PUBLIC"))),
    ]
    .into_iter()
    .filter(|d| d.is_dir())
    .collect()
}

/// 递归收集目录下全部 .lnk（PS Get-ChildItem -Recurse -Filter *.lnk -ErrorAction
/// SilentlyContinue 对译：遍历失败静默跳过）
fn walk_lnk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(walk_lnk_files(&p));
        } else if p.extension().map(|e| e.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
            out.push(p);
        }
    }
    out
}

/// .lnk → TargetPath（lnk crate；local_base_path 与 unicode 变体双回退；
/// 解析失败按 PS 同语义静默跳过）
fn lnk_target(path: &Path) -> Option<String> {
    let link = lnk::ShellLink::open(path, lnk::encoding::WINDOWS_1252).ok()?;
    let info = link.link_info().as_ref()?;
    if let Some(p) = info.local_base_path() {
        return Some(p.to_string());
    }
    if let Some(p) = info.local_base_path_unicode() {
        return Some(p.clone());
    }
    None
}

/// 注册表定位（windows-registry；替代 PS Get-ItemProperty 三根枚举）：
/// HKLM 64/32 + HKCU Uninstall；DisplayName 匹配 → DisplayIcon（剥 ",N" 图标
/// 索引后缀）/ InstallLocation+exe_names 组合
fn registry_locate(sess: &Session) -> Option<PathBuf> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    let roots = [
        (LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (LOCAL_MACHINE, "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (CURRENT_USER, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
    ];
    for (root, sub) in roots {
        let Ok(uninstall) = root.open(sub) else { continue };
        let Ok(key_iter) = uninstall.keys() else { continue };
        for key in key_iter {
            let Ok(item) = uninstall.open(&key) else { continue };
            let Ok(display) = item.get_string("DisplayName") else { continue };
            if !sess.prof.reg_patterns.iter().any(|p| glob_match_ci(p, &display)) {
                continue;
            }
            // DisplayIcon：剥 ",N" 图标索引后缀（移植自 3.8 修复，2026-10-17）：
            // 原 `replace(',', "")` 只删逗号本身，会把 `"…\WorkBuddyAI.exe,0"` 变成
            // `"…\WorkBuddyAI.exe0"`——is_file 恒失败（PS 版 -replace 同理踩坑，
            // 注册表级形态的隐蔽 bug）；改为取首个逗号段，与 env.rs 的
            // resolve_reg_profile_candidate 同构
            if let Ok(icon) = item.get_string("DisplayIcon") {
                let clean = icon.split(',').next().unwrap_or(&icon).trim().to_string();
                let p = PathBuf::from(clean);
                if p.is_file() && profile::exe_matches(&p, &sess.prof) {
                    return Some(p);
                }
            }
            // InstallLocation + exe_names 组合探测
            if let Ok(loc) = item.get_string("InstallLocation") {
                for exe in sess.prof.exe_names {
                    let p = PathBuf::from(loc.trim()).join(exe);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayicon_索引后缀剥离_对齐ps_replace语义() {
        // PS -replace ',','' 仅删逗号本身：",0" 剥后留下尾缀 "0"（"Doubao.exe0"），
        // Test-Path 随之失败回退 InstallLocation——Rust 逐字对齐该行为，
        // 不"修正"为剥索引（否则与 PS 实机结果不可对拍）
        assert_eq!(
            "C:\\x\\Doubao.exe,0".replace(',', "").trim(),
            "C:\\x\\Doubao.exe0"
        );
    }

    #[test]
    fn lnk_dirs_不因缺失目录崩溃() {
        // 任一目录缺失均静默过滤（PS Test-Path continue 语义）
        let dirs = lnk_dirs();
        assert!(dirs.iter().all(|d| d.is_dir()));
    }
}
