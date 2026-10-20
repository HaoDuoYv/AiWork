use serde::Serialize;
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, State};

use crate::state::AppState;

#[derive(Serialize)]
pub struct EnvStatus {
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

// 以下命令含耗时操作（注册表全量搜索、PowerShell 进程调用、最多 5s 的进程关闭轮询），
// 一律标记 async 派发到线程池执行，避免阻塞 UI 主线程（与 updater 冻结修复同因）。

#[tauri::command]
pub fn open_trae_website(_app: AppHandle) -> Result<(), String> {
    Command::new("cmd")
        .args(["/c", "start", "https://www.trae.cn"])
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 检测 Trae 客户端（双应用合并后唯一客户端）。
/// 原 `env_check`（Trae Work 域）已随双应用合并退役，本命令为唯一 Trae 环境入口。
#[tauri::command(async)]
pub fn env_check_trae_cn(_app: AppHandle, state: State<AppState>) -> EnvStatus {
    let loc = app_locate_inner(&state, "trae");
    EnvStatus {
        installed: loc.exe.is_some(),
        running: is_running_cn(),
        version: loc.version,
        path: loc.exe,
    }
}

/// 打开 Trae 客户端（双应用合并后唯一客户端）。传入 proxy_port（代理运行中）时以
/// `--proxy-server` 启动，让 Trae 的流量走本地 MITM 代理（捕获账号/观察请求）。
#[tauri::command(async)]
pub fn open_trae_cn_app(_app: AppHandle, state: State<AppState>, proxy_port: Option<u16>) -> Result<(), String> {
    let loc = app_locate_inner(&state, "trae");
    let exe = loc
        .exe
        .ok_or("未检测到 Trae 安装，请在「环境配置」中指定 Trae 安装路径")?;
    persist_detected_path(&state, "trae_cn_path", &exe);
    // 直开前清理可能指向已停止本地代理的残留系统代理
    if proxy_port.is_none() {
        crate::commands::proxy::cleanup_stale_local_proxy(&state);
    }
    // Electron 单实例：已运行的窗口会忽略新启动参数，注入代理前先三级关闭现有进程确保生效
    if proxy_port.is_some() {
        crate::commands::process::graceful_kill_app("Trae")?;
    }
    // Agent 窗口修正（2026-10-14 合并版实测）：启动窗口类型取 live storage.json 的
    // runtimeWindow.lastTask（缺省回落 Agent）。客户端未运行时补丁为 Agent，保证
    // 顶部打开与切换恢复两条路径恒开 Agent 窗口；运行中不写（退出时会被客户端
    // 自身状态覆盖，写了也无效）。
    if !is_running_cn() {
        // 复用 app_locate 的 user_data_dir（与 app_profile/档案表同源，
        // 审查修复 2026-10-14：原手拼 %APPDATA%\Trae CN 为第三处硬编码）
        let live_dir = std::path::PathBuf::from(&loc.user_data_dir);
        // best-effort 不阻断打开，但失败落日志（与 restore 路径的 Warn 口径一致，
        // 便于 storage.json 异常时排查「为何开的是 IDE 窗口」）
        if let Err(e) = crate::switcher::icube::apply_agent_window_fixup(&live_dir) {
            crate::fs_utils::app_log_default(&format!("Agent 窗口修正失败（打开路径）: {e}"));
        }
    }
    let mut cmd = Command::new(&exe);
    if let Some(port) = proxy_port {
        cmd.arg(format!("--proxy-server=http://127.0.0.1:{port}"));
    }
    cmd.spawn().map_err(|e| format!("启动 Trae 失败: {e}"))?;
    Ok(())
}

/// exe 路径持久化兜底（F-47）：自动探测成功时把结果写入 app_settings.json，
/// 之后即使注册表/默认目录变化，也能用上次成功的路径直接启动。
/// 用户手动指定（设置页）优先级更高，且仅在探测值与存量值不同时写盘。
fn persist_detected_path(state: &State<AppState>, key: &str, exe: &str) {
    // SQLite 化（P2）：app_settings 入 kv 文档
    let store = crate::store::db(&state.data_dir);
    let mut current: serde_json::Value = store.kv_get("app_settings");
    if !current.is_object() {
        current = serde_json::json!({});
    }
    let changed = current
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s != exe)
        .unwrap_or(true);
    if changed {
        if let Some(obj) = current.as_object_mut() {
            obj.insert(key.to_string(), serde_json::json!(exe));
        }
        let _ = store.kv_set("app_settings", &current);
    }
}

/// exe 文件版本（ProductVersion 优先，回退 FileVersion）——qoder_env_check 复用。
/// 原实现每次都 spawn powershell 读 VersionInfo（单次实测 1.2s+），而概览页 / 顶栏 /
/// 环境页在同一轮界面切换里会对同一个 exe 重复问；现改为三级：
/// 进程内按 (路径, mtime, 大小) 缓存 → 直读 PE 版本资源（`pe_version` 模块，微秒级）→
/// 读不到再回退原 powershell 实现（非 PE / 无版本资源 / 权限受限时兜底）。
pub(crate) fn version_of(path: &str) -> Option<String> {
    version_cached(path, || {
        crate::pe_version::product_or_file_version(path)
            .or_else(|| powershell_version_of(path))
            .map(normalize_version)
    })
}

/// 原实现（保留作 PE 资源读取失败时的兜底）：ProductVersion 优先，缺失回退 FileVersion。
/// 实测 Electron 系客户端两者差异巨大（CodeBuddy CN.exe FileVersion=1.106.1.0 而
/// ProductVersion=4.12.0），
/// 旧版恒读 FileVersion 导致顶栏版本显示为构建号而非产品版本
fn powershell_version_of(path: &str) -> Option<String> {
    let ps = format!(
        "$v=(Get-Item '{}').VersionInfo; if ($v.ProductVersion) {{ $v.ProductVersion }} else {{ $v.FileVersion }}",
        path.replace('\'', "''")
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 版本串归一化：4 段式去掉末尾冗余 ".0"（WorkBuddy 5.4.7.0 → 5.4.7）；
/// 3 段式保持原样（CodeBuddy 4.12.0 不能截成 4.12）
fn normalize_version(s: String) -> String {
    if s.matches('.').count() == 3 && s.ends_with(".0") {
        s[..s.len() - 2].to_string()
    } else {
        s
    }
}

/// 版本缓存条目：(mtime 毫秒, 文件大小) → 版本串（None 也缓存，避免反复读无版本资源的文件）
type VersionEntry = (u64, u64, Option<String>);

fn version_cache() -> &'static Mutex<HashMap<String, VersionEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, VersionEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 文件「版本是否变化」指纹（mtime + 大小）；取不到（文件不存在/无权限）时不做缓存
fn file_stamp(path: &str) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Some((mtime, meta.len()))
}

/// 版本读取的记忆化：指纹（路径 + mtime + 大小）未变则直接返回缓存，否则 `compute` 后写回
/// （None 同样缓存，避免反复读无版本资源的文件）；文件 stat 不到时不缓存，每次重试。
fn version_cached(path: &str, compute: impl FnOnce() -> Option<String>) -> Option<String> {
    let key = path.to_ascii_lowercase();
    let stamp = file_stamp(path);
    if let Some(stamp) = stamp {
        let guard = version_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((mtime, len, v)) = guard.get(&key) {
            if (*mtime, *len) == stamp {
                return v.clone();
            }
        }
    }
    let v = compute();
    if let Some((mtime, len)) = stamp {
        let mut guard = version_cache().lock().unwrap_or_else(|e| e.into_inner());
        // 防御：键集合本应有界（客户端 exe 数量级），异常膨胀时整体清空重来
        if guard.len() > 256 {
            guard.clear();
        }
        guard.insert(key, (mtime, len, v.clone()));
    }
    v
}

/// Trae 客户端进程检测：原 tasklist 子进程（单次实测 ~260ms）改为 sysinfo 进程表
/// 枚举 + 短 TTL 复用（见 `switcher::proc::any_running`）
fn is_running_cn() -> bool {
    crate::switcher::proc::any_running(&["Trae CN"])
}

// ── F-01：安装位置自动识别 app_locate（跨应用通用，四级探测）────────────────
// 探测顺序：用户手动指定（app_settings.json 持久化值）→ 默认路径候选 → 注册表卸载键
// → 运行进程反查（F-45 对齐全库原有语义：默认路径优先、注册表兜底）。
// 方案依据 doubao-trae-switch-plan.md §1.3 / workbuddy-switch-plan.md §2.1。

#[derive(Serialize)]
pub struct AppLocate {
    /// 应用标识：trae | doubao | workbuddy
    pub app: String,
    pub exe: Option<String>,
    pub user_data_dir: String,
    pub version: Option<String>,
    /// settings | registry | default | process | not_found
    pub source: String,
}

struct AppProfile {
    display: &'static str,
    /// 注册表 DisplayName 匹配片段（按顺序尝试，大小写不敏感）
    reg_patterns: &'static [&'static str],
    /// 注册表 InstallLocation 下尝试的 exe 名
    reg_exe_names: &'static [&'static str],
    exe_candidates: &'static [&'static str],
    /// 进程名（不带 .exe）
    proc_names: &'static [&'static str],
    user_data_dir: String,
    /// 设置页手动路径的 settings 键（无则跳过 settings 级）
    settings_key: Option<&'static str>,
}

fn app_profile(target_app: Option<&str>) -> AppProfile {
    let key = target_app.unwrap_or("trae").to_lowercase();
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    match key.as_str() {
        "doubao" => AppProfile {
            display: "豆包",
            reg_patterns: &["Doubao", "豆包"],
            reg_exe_names: &["Doubao.exe"],
            exe_candidates: &[
                "%LOCALAPPDATA%\\Doubao\\Application\\Doubao.exe",
                "%ProgramFiles%\\Doubao\\Application\\Doubao.exe",
            ],
            proc_names: &["Doubao"],
            user_data_dir: format!("{local}\\Doubao\\User Data"),
            settings_key: Some("doubao_path"),
        },
        "workbuddy" => AppProfile {
            display: "WorkBuddy",
            reg_patterns: &["WorkBuddy", "CodeBuddy"],
            reg_exe_names: &["WorkBuddy.exe"],
            exe_candidates: &["%LOCALAPPDATA%\\Programs\\WorkBuddy\\WorkBuddy.exe"],
            proc_names: &["WorkBuddy"],
            user_data_dir: format!("{home}\\.workbuddy"),
            settings_key: Some("workbuddy_path"),
        },
        // CodeBuddy IDE 独立探测（顶栏安装徽标/打开客户端）：默认路径同 WorkBuddy 的 Electron 惯例。
        // 本机实测（2026-09-12）：安装形态为 CodeBuddy CN（带空格），exe/进程名均为 "CodeBuddy CN"，
        // 注册表 DisplayName=CodeBuddy CN (User)，故 exe/进程/注册表候选同时覆盖不带 CN 的通用形态
        "codebuddy" => AppProfile {
            display: "CodeBuddy",
            reg_patterns: &["CodeBuddy"],
            reg_exe_names: &["CodeBuddy.exe", "CodeBuddy CN.exe"],
            exe_candidates: &[
                "%LOCALAPPDATA%\\Programs\\CodeBuddy\\CodeBuddy.exe",
                "%LOCALAPPDATA%\\Programs\\CodeBuddy CN\\CodeBuddy CN.exe",
            ],
            proc_names: &["CodeBuddy", "CodeBuddy CN"],
            user_data_dir: format!("{home}\\.codebuddy"),
            settings_key: Some("codebuddy_path"),
        },
        // trae / trae_cn / traecn / ide 及其它值 → 默认 Trae（双应用合并后唯一客户端）
        _ => AppProfile {
            display: "Trae",
            reg_patterns: &["Trae CN"],
            reg_exe_names: &["Trae CN.exe"],
            exe_candidates: &[
                "%LOCALAPPDATA%\\Programs\\Trae CN\\Trae CN.exe",
                "%ProgramFiles%\\Trae CN\\Trae CN.exe",
            ],
            proc_names: &["Trae CN"],
            user_data_dir: format!("{appdata}\\Trae CN"),
            settings_key: Some("trae_cn_path"),
        },
    }
}

// ── macOS 移植备忘（issue #45 修复时预埋）────────────────────────────────
// 本探测链路的 OS 耦合点集中在以下三处，移植时只需替换实现、档案表结构不变：
// 1. app_profile 本函数：exe_candidates / user_data_dir / proc_names 需按 macOS
//    实测路径出第二套档案（cfg(target_os) 分派）。已知映射：
//    - exe: /Applications/Trae Code.app/Contents/MacOS/Trae Code（默认安装无
//      per-user 目录，无 LOCALAPPDATA 等价物）
//    - 数据目录: ~/Library/Application Support/Trae CN（对应 %APPDATA%\Trae CN）
// 2. registry_app_path：注册表探测仅 Windows 有 → mac 实现直接返回 None
//    （或扩展为 Info.plist/Spotlight 查询，非必须）
// 3. process_exe_path / version_of / is_running_*：powershell+tasklist →
//    pgrep -l / mdls 或可执行文件属性读取；CommandExt.creation_flags 为
//    Windows 专属 API，需 cfg 包裹
// 注意：切换器侧 switcher/locate.rs（lnk/注册表/进程六级）有同构耦合点，
// mac 移植需一并处理；快照数据目录布局（switcher AppProfile.data_dir，即本文件
// AppProfile.user_data_dir 的对位字段）是行为核心，
// 移植前必须实机确认 macOS 版 Trae 的登录态文件结构与 Windows 同构。

/// 打开豆包桌面版（复用 app_locate 豆包档案四级探测；命中即回写设置以便下次直开）。
/// 与 Trae 同款代理注入：proxy_port 存在时以 --proxy-server 启动，让豆包客户端流量
/// 必走本地 MITM 代理（凭证/额度自动抓取不再依赖系统代理设置）。
#[tauri::command(async)]
pub fn open_doubao_app(state: State<AppState>, proxy_port: Option<u16>) -> Result<(), String> {
    let loc = app_locate_inner(&state, "doubao");
    let exe = loc.exe.ok_or("未检测到豆包安装，请在豆包「环境配置」中指定 Doubao.exe 路径")?;
    // 直开（不注入代理）前，清理可能指向已停止本地代理的残留系统代理
    if proxy_port.is_none() {
        crate::commands::proxy::cleanup_stale_local_proxy(&state);
    }
    // Chromium 单实例：已运行的窗口会忽略新启动参数，注入代理前先关闭现有进程确保生效
    if proxy_port.is_some() {
        crate::commands::process::graceful_kill_app("Doubao")?;
    }
    let mut cmd = Command::new(&exe);
    if let Some(port) = proxy_port {
        cmd.arg(format!("--proxy-server=http://127.0.0.1:{port}"));
    }
    cmd.spawn().map_err(|e| format!("启动豆包失败: {e}"))?;
    Ok(())
}

// ── Buddy 双应用（WorkBuddy / CodeBuddy）打开与环境检测 ────────────────────

/// 打开 WorkBuddy 桌面版：复用 app_locate workbuddy 档案四级探测取 exe。
/// 分离启动（spawn 不等待），不注入代理、不做三级关闭。
#[tauri::command(async)]
pub fn open_workbuddy_app(state: State<AppState>) -> Result<(), String> {
    open_buddy_app(&state, "workbuddy", "WorkBuddy")
}

/// 打开 CodeBuddy 桌面版：复用 app_locate codebuddy 档案四级探测取 exe。
/// 分离启动（spawn 不等待），不注入代理、不做三级关闭。
#[tauri::command(async)]
pub fn open_codebuddy_app(state: State<AppState>) -> Result<(), String> {
    open_buddy_app(&state, "codebuddy", "CodeBuddy")
}

/// Buddy 双应用打开共用实现（open_doubao_app 的极简版：无代理注入、无进程关闭）
fn open_buddy_app(state: &State<AppState>, app: &str, display: &str) -> Result<(), String> {
    let loc = app_locate_inner(state, app);
    let exe = loc
        .exe
        .ok_or_else(|| format!("未检测到 {display} 客户端，请先安装或手动指定路径"))?;
    Command::new(&exe)
        .spawn()
        .map_err(|e| format!("启动 {display} 失败: {e}"))?;
    Ok(())
}

/// CodeBuddy 桌面环境检测结果（Buddy 双应用域；字段 snake_case 直出前端）
#[derive(Serialize)]
pub struct CodeBuddyEnvCheck {
    pub installed: bool,
    pub running: bool,
    pub exe: Option<String>,
    pub version: Option<String>,
    /// auth 文件当前登录 uid（解析失败/未登录为 None）
    pub uid: Option<String>,
    /// auth 文件当前登录昵称（解析失败/未登录为 None）
    pub nickname: Option<String>,
}

/// CodeBuddy 桌面环境检测：exe/版本走 app_locate codebuddy 档案；uid/昵称复用
/// workbuddy_scan_auth_file 的 auth 文件解析（CodeBuddy 与 WorkBuddy 共享同一 auth 文件
/// %LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\workbuddy-desktop.info，本机实测）。
/// 环境检查不抛错：auth 文件不存在/解析失败时 uid/nickname 置 None。
#[tauri::command(async)]
pub fn codebuddy_env_check(state: State<AppState>) -> CodeBuddyEnvCheck {
    let loc = app_locate_inner(&state, "codebuddy");
    let running = is_running_codebuddy();
    let (uid, nickname) = match crate::commands::workbuddy::workbuddy_scan_auth_file(state.clone())
    {
        Ok(Some(scan)) => (
            (!scan.uid.is_empty()).then_some(scan.uid),
            (!scan.nickname.is_empty()).then_some(scan.nickname),
        ),
        _ => (None, None),
    };
    CodeBuddyEnvCheck {
        installed: loc.exe.is_some(),
        running,
        exe: loc.exe,
        version: loc.version,
        uid,
        nickname,
    }
}

/// CodeBuddy 进程检测：同时覆盖通用形态 CodeBuddy.exe 与本机实测的 "CodeBuddy CN.exe"。
/// 原实现逐个名字跑 tasklist 子进程（未运行时 2 次 ≈ 520ms），改 sysinfo 进程表一次枚举
/// （见 `switcher::proc::any_running`）
fn is_running_codebuddy() -> bool {
    crate::switcher::proc::any_running(&["CodeBuddy CN", "CodeBuddy"])
}

/// 兜底两级（注册表搜索 / 进程反查）结果短 TTL 缓存（P1 去重）。
/// 一轮界面切换里概览页与顶栏会对同一应用各探测一次（CodeBuddy 本机默认路径不存在，
/// 每轮都会走 `reg query`，实测 ~250ms/次），命中缓存直接复用上一轮结果。
/// 只缓存兜底两级：手动指定与默认路径仍实时判定，用户刚改设置需立即生效。
const LOCATE_FALLBACK_TTL: Duration = Duration::from_secs(3);

fn locate_fallback_cached<F>(app: &str, level: &str, f: F) -> Option<String>
where
    F: FnOnce() -> Option<String>,
{
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Option<String>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{app}:{level}");
    {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        // 「未找到」同样缓存：避免同轮切换里重复跑一次注定失败的注册表全量搜索
        if let Some((at, v)) = guard.get(&key) {
            if at.elapsed() < LOCATE_FALLBACK_TTL {
                return v.clone();
            }
        }
    }
    let v = f();
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    // 防御：键集合本应有界（应用数 × 两级），异常膨胀时整体清空重来
    if guard.len() > 64 {
        guard.clear();
    }
    guard.insert(key, (Instant::now(), v.clone()));
    v
}

/// app_locate 的内部版本（供 open_* 命令与 workbuddy 模块复用；无需 Option 包装）
pub(crate) fn app_locate_inner(state: &State<AppState>, app: &str) -> AppLocate {
    let profile = app_profile(Some(app));

    if let Some(sk) = profile.settings_key {
        let settings = state.settings();
        let custom = match sk {
            "trae_cn_path" => settings.trae_cn_path,
            "doubao_path" => settings.doubao_path,
            "workbuddy_path" => settings.workbuddy_path,
            "codebuddy_path" => settings.codebuddy_path,
            _ => None,
        };
        if let Some(p) = custom {
            let p = p.trim().to_string();
            if !p.is_empty() && std::path::Path::new(&p).is_file() {
                return finish_locate(&profile, p, "settings", None);
            }
        }
    }
    // 探测顺序对齐全库原有语义（旧 detect_trae/detect_trae_cn 与切换器 locate.rs
    // 均为默认路径优先、注册表兜底）：多安装共存时优先取官方默认位置的最新安装，
    // 注册表残留的过期条目只作兜底，避免选到已卸载/迁移的旧路径
    for c in profile.exe_candidates {
        let expanded = c
            .replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default())
            .replace("%ProgramFiles%", &std::env::var("ProgramFiles").unwrap_or_default());
        if std::path::Path::new(&expanded).is_file() {
            return finish_locate(&profile, expanded, "default", None);
        }
    }
    if let Some(exe) = locate_fallback_cached(app, "registry", || registry_app_path(&profile)) {
        return finish_locate(&profile, exe, "registry", None);
    }
    if let Some(exe) = locate_fallback_cached(app, "process", || process_exe_path(profile.proc_names))
    {
        return finish_locate(&profile, exe, "process", None);
    }
    AppLocate {
        app: app.to_string(),
        exe: None,
        user_data_dir: profile.user_data_dir,
        version: None,
        source: "not_found".into(),
    }
}

/// 安装位置自动识别：统一返回 {exe, userDataDir, version, source}。
/// async 派发：内部有注册表全量搜索与 PowerShell 调用，同步会冻结 UI。
#[tauri::command(async)]
pub fn app_locate(state: State<AppState>, target_app: Option<String>) -> AppLocate {
    app_locate_inner(&state, target_app.as_deref().unwrap_or("trae"))
}

/// 命中后统一补齐版本号并组装结果
fn finish_locate(profile: &AppProfile, exe: String, source: &str, version: Option<String>) -> AppLocate {
    let mut version = version.or_else(|| version_of(&exe));
    // 豆包客户端版本号官方形态带平台后缀（与安装包命名一致，如 2.28.13_win）
    if profile.settings_key == Some("doubao_path") {
        if let Some(v) = version.as_mut() {
            if !v.ends_with("_win") {
                *v = format!("{v}_win");
            }
        }
    }
    AppLocate {
        app: profile.display.to_lowercase().replace(' ', "_"),
        exe: Some(exe),
        user_data_dir: profile.user_data_dir.clone(),
        version,
        source: source.into(),
    }
}

/// 注册表卸载键搜索（按应用档案的 DisplayName 片段与 exe 名参数化）
fn registry_app_path(profile: &AppProfile) -> Option<String> {
    for pattern in profile.reg_patterns {
        for root in ["HKCU", "HKLM"] {
            let out = match Command::new("reg")
                .args([
                    "query",
                    &format!("{root}\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
                    "/s",
                    "/f",
                    pattern,
                ])
                .creation_flags(0x08000000)
                .output()
            {
                Ok(o) => o,
                Err(_) => continue,
            };
            let s = String::from_utf8_lossy(&out.stdout);
            let pat_upper = pattern.to_uppercase();
            let mut icon: Option<String> = None;
            let mut loc: Option<String> = None;
            let mut name_ok = false;
            for line in s.lines() {
                let line = line.trim();
                if line.starts_with("HKEY_") {
                    if name_ok {
                        if let Some(hit) = resolve_reg_profile_candidate(&icon, &loc, profile) {
                            return Some(hit);
                        }
                    }
                    icon = None;
                    loc = None;
                    name_ok = false;
                    continue;
                }
                if let Some(v) = line.strip_prefix("DisplayName") {
                    if let Some(val) = v.split("REG_SZ").nth(1) {
                        if val.to_uppercase().contains(&pat_upper) {
                            name_ok = true;
                        }
                    }
                } else if let Some(v) = line.strip_prefix("DisplayIcon") {
                    if let Some(val) = v.split("REG_SZ").nth(1) {
                        icon = Some(val.trim().to_string());
                    }
                } else if let Some(v) = line.strip_prefix("InstallLocation") {
                    if let Some(val) = v.split("REG_SZ").nth(1) {
                        loc = Some(val.trim().to_string());
                    }
                }
            }
            if name_ok {
                if let Some(hit) = resolve_reg_profile_candidate(&icon, &loc, profile) {
                    return Some(hit);
                }
            }
        }
    }
    None
}

/// 从注册表 DisplayIcon / InstallLocation 推导 exe 路径（按档案 exe 名匹配）
fn resolve_reg_profile_candidate(
    icon: &Option<String>,
    loc: &Option<String>,
    profile: &AppProfile,
) -> Option<String> {
    if let Some(icon) = icon {
        // DisplayIcon 可能带 ",0" 图标索引后缀
        let clean = icon.split(',').next().unwrap_or(icon).trim().to_string();
        if clean.to_lowercase().ends_with(".exe") && std::path::Path::new(&clean).is_file() {
            return Some(clean);
        }
    }
    if let Some(loc) = loc {
        for name in profile.reg_exe_names {
            let cand = format!("{loc}\\{name}");
            if std::path::Path::new(&cand).is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// 运行进程反查 exe 路径（应用运行中时最准）：精确映像名匹配，取首个带路径的进程。
/// 原实现 spawn powershell（`Get-Process -Name @(...) | Select -First 1`，单次实测 1.2s+），
/// 改由 sysinfo 进程表直读（`switcher::proc::running_exe_exact`）。
fn process_exe_path(proc_names: &[&str]) -> Option<String> {
    let exe = crate::switcher::proc::running_exe_exact(proc_names)?;
    let s = exe.to_string_lossy().to_string();
    if s.is_empty() || !std::path::Path::new(&s).is_file() {
        None
    } else {
        Some(s)
    }
}
