//! 应用档案表（F-48 表驱动）：6 应用 × 4 快照布局（原 trae-switch-bridge.ps1
//! 82-198 行对译）。icube 布局（Trae/Qoder）同为 icube 内核的 VSCode fork，
//! 登录态文件结构完全同构，按档案参数化复用全部切换逻辑；chromium（豆包）/
//! authfile（WorkBuddy/CodeBuddy）布局各有独立快照管线。
//! （2026-10-14：Trae Work 与 Trae CN 合并为单一客户端，TraeWork 档案退役删除）

use std::path::PathBuf;

use super::TargetApp;

/// 快照布局（PS $Script:SnapshotLayout）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Icube,
    Chromium,
    /// Electron 根级 Chromium 会话（Qoder Work：userData 根直挂 Network/Local
    /// Storage，无 Default/Profile N 子目录，与 Chromium 布局不同构）
    ElectronRoot,
    Authfile,
}

impl Layout {
    /// 字符串形态（与 PS 值一致，用于日志/错误消息）
    pub fn as_str(self) -> &'static str {
        match self {
            Layout::Icube => "icube",
            Layout::Chromium => "chromium",
            Layout::ElectronRoot => "electron-root",
            Layout::Authfile => "authfile",
        }
    }
}

pub struct AppProfile {
    /// 应用显示名（PS $Script:AppName，进入全部进度文案）
    pub app_name: &'static str,
    pub layout: Layout,
    /// 应用真实数据目录（PS $Script:TraeDataDir）
    pub data_dir: PathBuf,
    /// 快照槽根目录（PS $Script:ProfilesDir）
    pub profiles_dir: PathBuf,
    /// app_settings.json 的手动路径键（PS $Script:SettingsPathKey）
    pub settings_path_key: &'static str,
    /// 优雅关闭等待秒数（豆包 8 / WB+CB 5 / 默认 3）
    pub graceful_wait_secs: u64,
    /// 进程名白名单（不带 .exe，精确匹配 = Get-Process -Name 语义；Stop 用）
    pub proc_names: &'static [&'static str],
    /// 进程名通配组（**exe 发现第 5 级专用**，比 Stop 的精确组更宽，再经
    /// exe_names 白名单过滤防串台；PS $Script:ProcPatterns，双轨刻意保留）
    pub proc_patterns: &'static [&'static str],
    /// exe 文件名白名单（Test-ExeMatchesApp 语义：lnk/注册表/进程回退防串台）
    pub exe_names: &'static [&'static str],
    /// .lnk 文件名匹配模式（大小写双形态，PS -like 语义）
    pub lnk_patterns: &'static [&'static str],
    /// 注册表 DisplayName 匹配模式
    pub reg_patterns: &'static [&'static str],
    /// exe 候选路径（环境变量展开后的绝对路径，PS $Script:ExeCandidates）
    pub exe_candidates: Vec<PathBuf>,
    /// 仅 CodeBuddy：L3 vscdb 登录真源目录（%APPDATA%\CodeBuddy CN\User\globalStorage）
    pub cb_global_storage_dir: Option<PathBuf>,
    /// icube 布局快照白名单（F-80 M3 档案化：TRAE_ICUBE_ITEMS / QODER_IDE_ITEMS；
    /// 非 icube 布局为空表，backup/restore 不消费）
    pub icube_items: &'static [super::icube::Item],
    /// 恢复快照后把 `runtimeWindow.lastTask.windowType` 补丁为 Agent（合并版 Trae
    /// 客户端启动窗口类型取该键，缺省回落 Agent；快照冻结旧窗口类型会让恢复后
    /// 打开 IDE 窗口。2026-10-14 实测，见 switcher/icube.rs apply_agent_window_fixup）
    pub force_agent_window: bool,
}

impl AppProfile {
    /// current_account.txt 路径（PS $Script:CurrentAccountFile）
    pub fn current_account_file(&self) -> PathBuf {
        self.profiles_dir.join("current_account.txt")
    }
}

/// Test-ExeMatchesApp 对译：路径文件名必须 ∈ exe_names 白名单
///（防 lnk/注册表/进程回退解析到另一个应用；大小写不敏感）
pub fn exe_matches(path: &std::path::Path, prof: &AppProfile) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => prof.exe_names.iter().any(|e| e.eq_ignore_ascii_case(name)),
        None => false,
    }
}

/// 档案表（PS switch 块逐项对译；参数顺序见各分支注释）
pub fn profile_for(app: TargetApp, app_data_dir: &std::path::Path) -> AppProfile {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let appdata = env("APPDATA");
    let local = env("LOCALAPPDATA");
    let home = env("USERPROFILE");
    let program_files = env("ProgramFiles");
    let data = app_data_dir;
    match app {
        TargetApp::Trae => AppProfile {
            app_name: "Trae",
            layout: Layout::Icube,
            data_dir: PathBuf::from(format!("{appdata}\\Trae CN")),
            profiles_dir: data.join("data").join("profiles_trae"),
            settings_path_key: "trae_cn_path",
            // 审查修复（2026-09-15）：3s 实测恒超时 → 每次切换都强杀，vscdb WAL 残留
            // 被客户端启动重放导致旧账号复活（与豆包 8s 同理：落盘/退出需要时间）
            graceful_wait_secs: 8,
            proc_names: &["Trae CN"],
            proc_patterns: &["Trae*", "TRAE*"],
            exe_names: &["Trae CN.exe"],
            lnk_patterns: &["*TRAE*", "*Trae*"],
            reg_patterns: &["*TRAE*", "*Trae*"],
            exe_candidates: vec![
                PathBuf::from(format!("{local}\\Programs\\Trae CN\\Trae CN.exe")),
                PathBuf::from(format!("{program_files}\\Trae CN\\Trae CN.exe")),
                PathBuf::from("D:\\Programs\\Trae CN\\Trae CN.exe"),
            ],
            cb_global_storage_dir: None,
            icube_items: super::icube::TRAE_ICUBE_ITEMS,
            force_agent_window: true,
        },
        TargetApp::Doubao => AppProfile {
            app_name: "豆包",
            layout: Layout::Chromium,
            data_dir: PathBuf::from(format!("{local}\\Doubao\\User Data")),
            profiles_dir: data.join("data").join("profiles_doubao"),
            settings_path_key: "doubao_path",
            // chromium 壳退出前要落盘 leveldb/cookie，3 秒实测经常不够（强杀导致
            // 文件锁 → 备份静默缺文件 → 恢复后登录态丢失）
            graceful_wait_secs: 8,
            proc_names: &["Doubao"],
            proc_patterns: &["Doubao*"],
            exe_names: &["Doubao.exe"],
            lnk_patterns: &["*Doubao*", "*豆包*"],
            reg_patterns: &["*Doubao*", "*豆包*"],
            exe_candidates: vec![
                PathBuf::from(format!("{local}\\Doubao\\Application\\Doubao.exe")),
                PathBuf::from(format!("{program_files}\\Doubao\\Application\\Doubao.exe")),
            ],
            cb_global_storage_dir: None,
            icube_items: &[],
            force_agent_window: false,
        },
        TargetApp::WorkBuddy => AppProfile {
            app_name: "WorkBuddy",
            layout: Layout::Authfile,
            data_dir: PathBuf::from(format!("{home}\\.workbuddy")),
            profiles_dir: data.join("data").join("profiles_workbuddy"),
            settings_path_key: "workbuddy_path",
            graceful_wait_secs: 5,
            // F2-3 双端解耦：auth 文件虽与 CodeBuddy 共用同一物理文件，但实测确认
            // CodeBuddy 从不回写共享 auth 文件（登录真源在自身 vscdb）——
            // 切/存 WorkBuddy 不关停 CodeBuddy，两端完全独立（ProcNames 仅本端）
            proc_names: &["WorkBuddy"],
            proc_patterns: &["WorkBuddy*"],
            exe_names: &["WorkBuddy.exe"],
            lnk_patterns: &["*WorkBuddy*"],
            reg_patterns: &["*WorkBuddy*"],
            exe_candidates: vec![PathBuf::from(format!(
                "{local}\\Programs\\WorkBuddy\\WorkBuddy.exe"
            ))],
            cb_global_storage_dir: None,
            icube_items: &[],
            force_agent_window: false,
        },
        TargetApp::CodeBuddy => AppProfile {
            app_name: "CodeBuddy",
            layout: Layout::Authfile,
            data_dir: PathBuf::from(format!("{home}\\.codebuddy")),
            profiles_dir: data.join("data").join("profiles_codebuddy"),
            settings_path_key: "codebuddy_path",
            graceful_wait_secs: 5,
            // F2-1 进程解耦：CodeBuddy 登录真源在自身 state.vscdb（%APPDATA%\CodeBuddy CN），
            // 不消费共享 auth 文件——切/存 CodeBuddy 不关停在跑的 WorkBuddy
            proc_names: &["CodeBuddy", "CodeBuddy CN"],
            proc_patterns: &["CodeBuddy*"],
            exe_names: &["CodeBuddy.exe", "CodeBuddy CN.exe"],
            lnk_patterns: &["*CodeBuddy*"],
            reg_patterns: &["*CodeBuddy*"],
            exe_candidates: vec![
                PathBuf::from(format!("{local}\\Programs\\CodeBuddy\\CodeBuddy.exe")),
                PathBuf::from(format!("{local}\\Programs\\CodeBuddy CN\\CodeBuddy CN.exe")),
            ],
            // F1-1 L3 层：CodeBuddy CN（VS Code fork）登录真源实测在自身 roaming 的
            // state.vscdb secret storage，不在共享 auth 文件——快照/恢复必须覆盖此处
            cb_global_storage_dir: Some(PathBuf::from(format!(
                "{appdata}\\CodeBuddy CN\\User\\globalStorage"
            ))),
            icube_items: &[],
            force_agent_window: false,
        },
        TargetApp::Qoder => AppProfile {
            // 【跨平台审查 2026-10-03】macOS 适配预留：Qoder 档案是 macOS 分支的
            // 单点扩展位——AppProfile 为纯数据表，macOS 无需动切换管线，只需
            // profile_for 内按 cfg(target_os) 返回 mac 档案。预期映射：
            //   data_dir        = ~/Library/Application Support/QoderCN
            //                     （Electron userData 惯例，需真机实测）
            //   exe_candidates  = /Applications/Qoder CN IDE.app/Contents/MacOS/<可执行名>
            //   proc_names/exe_names = 去 .exe 后缀形态（"Qoder CN IDE"），
            //                     strip_exe_suffix（switcher/proc.rs）在 mac 上天然
            //                     剥不动无后缀名，双平台白名单可直接并存
            //   lnk_patterns/reg_patterns = Windows 专属发现级（.lnk/注册表），
            //                     macOS 分支对应「/Applications 下 .app 扫描 +
            //                     Spotlight (mdfind)」，AppProfile 字段语义可承载，
            //                     由 locate.rs 分平台消费
            app_name: "Qoder",
            layout: Layout::Icube,
            // F-80 M0 实测 2026-09-27：IDE 数据目录为 %APPDATA%\QoderCN（设计文档
            // §2.3 的 com.qodercn.app.stable 与本机不符，按实测修正）
            data_dir: PathBuf::from(format!("{appdata}\\QoderCN")),
            profiles_dir: data.join("data").join("profiles_qoder"),
            // F-80 R-2：与前端写入键对齐（QoderSettings 写 qoder_ide_path = IDE exe 路径，
            // commands/qoder/common.rs::ide_exe_candidates 同源消费）；原 "qoder_path" 为死键，
            // locate 按它读 settings 永得 None，用户显式指定的路径在切换链路中失效
            settings_path_key: "qoder_ide_path",
            // 同 Trae 系：VSCode fork 强杀后 vscdb WAL 残留被启动回放，8s 优雅落盘
            graceful_wait_secs: 8,
            // 2026-10-02 收窄：IDE 全部进程均名为 "Qoder CN IDE"（安装目录仅此一个
            // exe，实测）；旧「壳进程 Qoder CN.exe」已被 Qoder Work 独立客户端接管
            //（%LOCALAPPDATA%\Programs\Qoder CN\Qoder CN.exe），保留会让
            // exe_names/lnk/注册表/运行进程发现三级串台启动 Work exe → 白名单只留
            // IDE 本体。切 IDE 不再连带关 Work（进程名已无交集）
            proc_names: &["Qoder CN IDE"],
            proc_patterns: &["Qoder CN IDE*"],
            exe_names: &["Qoder CN IDE.exe"],
            lnk_patterns: &["*Qoder*"],
            reg_patterns: &["*Qoder*"],
            exe_candidates: vec![
                PathBuf::from(format!("{local}\\Programs\\Qoder CN IDE\\Qoder CN IDE.exe")),
                PathBuf::from(format!("{program_files}\\Qoder CN IDE\\Qoder CN IDE.exe")),
            ],
            cb_global_storage_dir: None,
            icube_items: super::icube::QODER_IDE_ITEMS,
            force_agent_window: false,
        },
        TargetApp::QoderWork => AppProfile {
            // 【跨平台审查 2026-10-03】macOS 适配预留：ElectronRoot 布局快照管线
            // （switcher/electron_root.rs）纯文件拷贝，跨平台零改动；macOS 差异仅
            //   data_dir        = ~/Library/Application Support/com.qodercn.app.stable
            //                     （Electron userData 惯例，与 Windows 同名不同根，需实测）
            //   exe_candidates  = /Applications/Qoder CN.app/Contents/MacOS/<可执行名>
            //   proc_names      = "Qoder CN"（macOS 映像名无 .exe 后缀）
            // 注意：macOS 上 QoderWork 与 Qoder IDE 进程名同样可能含 "Qoder" 前缀，
            // 精确匹配防串台的收窄语义（见下方 proc_names 注释）必须保持
            app_name: "Qoder Work",
            // 2026-10-02 实测：独立 Electron 客户端（0.4.3，.qoder-versions 滚动更新），
            // 数据目录 %APPDATA%\com.qodercn.app.stable（默认会话挂 userData 根：
            // Network/Cookies、Local Storage、Session Storage、Local State、Preferences）
            layout: Layout::ElectronRoot,
            data_dir: PathBuf::from(format!("{appdata}\\com.qodercn.app.stable")),
            profiles_dir: data.join("data").join("profiles_qoder_work"),
            settings_path_key: "qoder_work_path",
            // Electron 退出前要落盘 leveldb/cookie（同豆包 chromium 布局 8s 理由）
            graceful_wait_secs: 8,
            // 进程名 "Qoder CN" 与 IDE 壳同名（Work 接管了该进程名）：精确匹配
            // 只停 Work 本体，不 wildcard（"Qoder CN*" 会误杀 Qoder CN IDE）
            proc_names: &["Qoder CN"],
            proc_patterns: &["Qoder CN"],
            exe_names: &["Qoder CN.exe"],
            lnk_patterns: &["*Qoder*"],
            reg_patterns: &["*Qoder*"],
            exe_candidates: vec![
                PathBuf::from(format!("{local}\\Programs\\Qoder CN\\Qoder CN.exe")),
                PathBuf::from(format!("{program_files}\\Qoder CN\\Qoder CN.exe")),
            ],
            cb_global_storage_dir: None,
            icube_items: &[],
            force_agent_window: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_data() -> PathBuf {
        std::env::temp_dir().join(format!("sw-profile-test-{}", std::process::id()))
    }

    #[test]
    fn 六应用档案字段与ps常量表一致() {
        let data = temp_data();
        let db = profile_for(TargetApp::Doubao, &data);
        assert_eq!(db.layout, Layout::Chromium);
        assert_eq!(db.graceful_wait_secs, 8);
        assert_eq!(db.profiles_dir, data.join("data").join("profiles_doubao"));
        assert!(db.icube_items.is_empty());

        let wb = profile_for(TargetApp::WorkBuddy, &data);
        assert_eq!(wb.layout, Layout::Authfile);
        assert_eq!(wb.graceful_wait_secs, 5);
        assert_eq!(wb.proc_names, &["WorkBuddy"]);

        let cb = profile_for(TargetApp::CodeBuddy, &data);
        assert_eq!(cb.proc_names, &["CodeBuddy", "CodeBuddy CN"]);
        assert!(cb.cb_global_storage_dir.is_some());

        let trae = profile_for(TargetApp::Trae, &data);
        assert_eq!(trae.settings_path_key, "trae_cn_path");
        assert_eq!(trae.profiles_dir, data.join("data").join("profiles_trae"));
        assert_eq!(trae.exe_candidates.len(), 3);
        assert_eq!(trae.icube_items.len(), 15);
        // 合并版实测（2026-10-14）：仅 Trae 开启恢复后 Agent 窗口修正
        assert!(trae.force_agent_window);
        assert!(!db.force_agent_window);

        // F-80 M3：Qoder IDE 档案（icube 布局，复用 Trae 切号管线；M0 实测数据目录 QoderCN）
        let qd = profile_for(TargetApp::Qoder, &data);
        assert_eq!(qd.app_name, "Qoder");
        assert_eq!(qd.layout, Layout::Icube);
        assert_eq!(
            qd.data_dir,
            PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("QoderCN")
        );
        assert_eq!(qd.profiles_dir, data.join("data").join("profiles_qoder"));
        assert_eq!(qd.settings_path_key, "qoder_ide_path");
        assert_eq!(qd.graceful_wait_secs, 8);
        // 2026-10-02 收窄：Qoder CN.exe 已归 Qoder Work，IDE 白名单只留本名
        assert_eq!(qd.proc_names, &["Qoder CN IDE"]);
        assert_eq!(qd.exe_names, &["Qoder CN IDE.exe"]);
        assert_eq!(qd.exe_candidates.len(), 2);
        assert!(qd.cb_global_storage_dir.is_none());
        assert_eq!(qd.icube_items.len(), 15);

        // 2026-10-02：Qoder Work 独立客户端（electron-root 布局，数据目录 com.qodercn.app.stable）
        let qw = profile_for(TargetApp::QoderWork, &data);
        assert_eq!(qw.app_name, "Qoder Work");
        assert_eq!(qw.layout, Layout::ElectronRoot);
        assert_eq!(
            qw.data_dir,
            PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("com.qodercn.app.stable")
        );
        assert_eq!(qw.profiles_dir, data.join("data").join("profiles_qoder_work"));
        assert_eq!(qw.settings_path_key, "qoder_work_path");
        assert_eq!(qw.graceful_wait_secs, 8);
        assert_eq!(qw.proc_names, &["Qoder CN"]);
        assert_eq!(qw.exe_candidates.len(), 2);
        assert!(qw.cb_global_storage_dir.is_none());
        assert!(qw.icube_items.is_empty());
    }

    #[test]
    fn current_account_file_位于profiles根() {
        let data = temp_data();
        let trae = profile_for(TargetApp::Trae, &data);
        assert_eq!(
            trae.current_account_file(),
            data.join("data").join("profiles_trae").join("current_account.txt")
        );
    }

    #[test]
    fn exe_matches_大小写不敏感与白名单外拒绝() {
        let data = temp_data();
        let trae = profile_for(TargetApp::Trae, &data);
        assert!(exe_matches(std::path::Path::new("C:\\x\\Trae CN.exe"), &trae));
        assert!(exe_matches(std::path::Path::new("D:\\a\\trae cn.EXE"), &trae));
        // 白名单外的 exe 拒绝（TRAE SOLO CN 随 Trae Work 退役，恒不在白名单）
        assert!(!exe_matches(std::path::Path::new("C:\\x\\TRAE SOLO CN.EXE"), &trae));
        assert!(!exe_matches(std::path::Path::new("C:\\x\\Doubao.exe"), &trae));
    }
}
