# Trae 双应用合并适配调整方案

> | 项 | 内容 |
> |---|---|
> | 版本 | v1.0（Pending，待评审） |
> | 状态 | 方案设计完成，未实施 |
> | 定位 | Trae Work（TRAE SOLO CN）与 Trae CN IDE 官方合并为单一客户端后，本工程移除全部 TraeWork 相关逻辑、收敛为单 Trae 应用（Trae CN）的完整整改方案 |
> | 适用 | 本机（Windows）已升级至合并版，可直接实测验证 |
> | 关联 | `docs/pending/TraeCLI桥接落地方案.md`、`backlog.md`（实施时登记条目，编号顺延） |

---

## TL;DR

| 项 | 结论 |
|---|---|
| 合并事实 | 官方合并后**唯一存续客户端**为 `C:\Users\tianw\AppData\Local\Programs\Trae CN\Trae CN.exe`（exe/进程/数据目录均为 `Trae CN`，icube 内核 + `modules/ai-agent` + `bin\trae-cn` CLI）；`TRAE SOLO CN` 已卸载，安装与数据目录仅剩残留 |
| 整改原则 | **彻底移除** `TargetApp::TraeWork` 及其全部派生逻辑（档案表、枚举分支、双循环、双池、双键、双 entitlement、双 UI 入口），不做兼容别名；存量数据一次性迁移 |
| 唯一存续身份 | 现有 `TargetApp::Trae` 档案与合并版客户端**完全吻合**（data_dir=`%APPDATA%\Trae CN`、proc=`Trae CN`、exe 首选 `...\Programs\Trae CN\Trae CN.exe`、池 `profiles_trae`、键 `trae_cn_path`），**档案本体不动**，全部整改是"删除 TraeWork 侧 + 收敛到 Trae 侧" |
| 影响面 | 后端 17 个文件、前端 15 个文件、文档 5 篇、存量数据迁移 2 项（settings 键 + 快照池） |
| 明确不动 | ① 本应用自身品牌痕迹（`TraeWorkAssistant`/`Trae Work Assistant`：state.rs、main.rs、misc.rs 旧任务名、updater.rs 仓库 URL）——与本方案无关；② `api.trae.cn` 服务端端点与 `solo_work_lite`/`solo_agent` 协议常量；③ 网关 Trae 池与调度 trae-* 域任务；④ 签到 UA 字面量（服务端兼容性未验证，见开放问题） |
| 分期 | P0 后端收敛（编译期收口）→ P1 数据迁移 + 本机验证 → P2 前端收敛 → P3 文档联动 + 实测验收 |
| 最大风险 | ① TraeWork 存量快照恢复进合并版的可用性（设备指纹来自旧应用）；② 合并版数据目录未见顶层 `logs/`，`detect_live_uid` 日志探测可能失效（hybrid 兜底已在，功能不丢、准确率或降级） |

---

## 1. 背景与合并事实

### 1.1 官方合并

Trae Work（独立 Solo 客户端，exe=`TRAE SOLO CN.exe`）与 Trae CN IDE（exe=`Trae CN.exe`）已官方合并为单一应用。本机（2026-10 实测）：

| 项 | 合并前（双应用） | 合并后（实测） |
|---|---|---|
| 安装目录 | `%LOCALAPPDATA%\Programs\TRAE SOLO CN\` 与 `%LOCALAPPDATA%\Programs\Trae CN\` | **仅** `%LOCALAPPDATA%\Programs\Trae CN\`；TRAE SOLO CN 目录仅剩 `debug.log` 残留 |
| 客户端 exe | `TRAE SOLO CN.exe`（Work）/ `Trae CN.exe`（IDE） | `Trae CN.exe`（同一 icube 内核，内含 `resources\app\modules\ai-agent\`{ai_agent.dll, harness.dll, ctx-cli.exe}、`bin\trae-cn` CLI、`aha_doctor\`） |
| 进程名 | `TRAE SOLO CN` / `Trae CN` | `Trae CN` |
| 数据目录 | `%APPDATA%\TRAE SOLO CN` / `%APPDATA%\Trae CN` | **仅** `%APPDATA%\Trae CN`（icube 布局完整：`User\globalStorage\storage.json`、`state.vscdb`(+`.backup`/`.f68.bak`)、`machineid`、`aha\TinyStorage`、`ahanet\`、`Local Storage`、`Network\Cookies`、`Local State`、`Preferences`、`User\mcp.json`/`settings.json`、`migration-job.json`）；TRAE SOLO CN 数据目录仅剩 `logs\20261013T213703\` + `aha_log\` 残留 |

关键判断：

1. **合并版身份 = 现有 `TargetApp::Trae` 档案**（[profile.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/profile.rs#L89-L110)）：data_dir、proc_names、exe_candidates、`profiles_trae` 池、`trae_cn_path` 键全部吻合，**档案本体零修改**。
2. 本工程为"双 Trae 应用"设计的一切均需收敛：`TargetApp::TraeWork` 枚举与档案、双快照池（`profiles` + `profiles_trae`）、双 settings 键（`trae_path` + `trae_cn_path`）、双 entitlement（`LocalEntitlement { work, cn }`）、双应用自动发现循环、双 UI 入口。
3. 收敛方式为**移除**而非别名：不留 `parse("TraeWork") → Trae` 的兼容映射（存量持久化数据由迁移一次性改写，见 §5.11）。

### 1.2 本机实测侦察记录（2026-10-14）

- 合并版安装目录确认：`Trae CN.exe`、`Trae CN.VisualElementsManifest.xml`、`manifest.json`、`bin\trae-cn`(+`.cmd`)、`aha_doctor\`、`resources\app\modules\ai-agent\`。
- `%APPDATA%\Trae CN\User\globalStorage\`：`storage.json`、`state.vscdb`、`state.vscdb.backup`、`state.vscdb.f68.bak` 均在——`.f68.bak` 为本工程 F-68 全局键合并机制的回滚痕迹，合并升级未破坏 icube 布局。
- **`%APPDATA%\Trae CN` 顶层未见 `logs/` 目录**（旧 TRAE SOLO CN 有 `logs\<yyyymmddThhmmss>\dynamicConfig.log`）。`icube::detect_live_uid`（[icube.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/icube.rs#L200-L208)）依赖 `data_dir\logs\`，此为**待实测项 OQ-1**（§9）；hybrid 证据链兜底已在，uid 推导不会整体失效。

---

## 2. 现状盘点：TraeWork 引用全景（代码级清单）

> 以下行号为当前 HEAD 快照。标注【删】整段删除、【改】修改、【测】测试改写、【注】仅注释更新。

### 2.1 后端（src-tauri/src）

**A. 切换器核心（高影响）**

| # | 位置 | 现状 | 处理 |
|---|---|---|---|
| 1 | [switcher/mod.rs:67](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/mod.rs#L67) | `TargetApp::TraeWork` 枚举变体 | 【删】变体删除 |
| 2 | [switcher/mod.rs:81-93](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/mod.rs#L81-L93) | `parse` 未知值回退 `TraeWork` | 【改】未知值返回 `Err`（调用方恒传合法值；迁移已改写存量字符串，无需兜底别名） |
| 3 | [switcher/mod.rs:414](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/mod.rs#L414) | `save_identity_guard`：`matches!(sess.target, TraeWork \| Trae)` 时走 `icube::detect_live_uid` | 【改】收敛为 `TargetApp::Trae` 单臂 |
| 4 | [switcher/mod.rs:389,399,527](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/mod.rs#L527) | 注释「icube 布局（TraeWork/Trae）」 | 【注】 |
| 5 | [switcher/mod.rs:1207-1288](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/mod.rs#L1207-L1288) | 单测构造 `TargetApp::TraeWork`、"正在关闭 Trae Work" 文案断言 | 【测】改用 `TargetApp::Trae` |
| 6 | [switcher/profile.rs:179-203](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/profile.rs#L179-L203) | `TargetApp::TraeWork => AppProfile { ... }` 整段档案（data_dir=`%APPDATA%\TRAE SOLO CN`、池 `profiles`、键 `trae_path`、proc/exe 三候选） | 【删】整段 |
| 7 | [switcher/profile.rs:294-375](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/profile.rs#L294-L375) | 档案表单测（TraeWork 断言、未知 kind 断言） | 【测】删除 TraeWork 用例；新增「未知 kind → Err」用例 |
| 8 | [switcher/proc.rs:404-421](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/proc.rs#L404-L421) | `name_matches` 测试用 TraeWork 档案（白名单含 "Trae" 前缀误命中防御用例） | 【测】改用 Trae 档案（`proc_names=["Trae CN"]` 不含裸 "Trae"，该防御用例可简化或删除） |
| 9 | [switcher/proc.rs:132-133](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/proc.rs#L132-L133)、[switcher/icube.rs:1,28](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/icube.rs#L28)、[switcher/icube.rs:273](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/icube.rs#L273) | 注释 + 测试构造 | 【注】【测】 |

**B. 命令层（高影响）**

| # | 位置 | 现状 | 处理 |
|---|---|---|---|
| 10 | [commands/trae_apps.rs:34-48](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs#L34-L48) | `app_data_dirs`：`"TraeWork" => [TRAE SOLO CN, TRAE SOLO]`、`"Trae" => [Trae CN]` | 【删】TraeWork 分支；`app_display`（48 行）同步 |
| 11 | [commands/trae_apps.rs:400](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs#L400)、[488](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs#L488) | `apps_accounts_discover` / `backfill_dc_id_for` 中 `for kind in ["TraeWork", "Trae"]` 双应用循环 | 【改】单应用（`Trae`）循环；kind 参数整体退役或恒传 "Trae" |
| 12 | [commands/trae_apps.rs:493](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs#L493) | dc id 回填扫描 `for prof in ["profiles", "profiles_trae"]` 双池 | 【改】单池 `profiles_trae` |
| 13 | [commands/trae_apps.rs:341-343,615](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs#L341-L343) | 结构体字段注释「应用类别：TraeWork \| Trae」 | 【改】收敛说明 + 前端契约同步（§5.3） |
| 14 | [commands/trae_apps.rs:694-702](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs#L694-L702) | `apps_entitlement_read`：`LocalEntitlement { work, cn }` 双取（`work: app_login("TraeWork",...)`） | 【改】单值结构（§5.3） |
| 15 | [commands/trae_apps.rs:828-944](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs#L828-L944) | 桥标记/live uid 测试：`live_cloud_uid_with(Some(..), "TraeWork", ..)`、`marker_with_ts("TraeWork", ..)`、`TargetApp::TraeWork` 池路由断言 | 【测】 |
| 16 | [commands/env.rs:22-33](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/env.rs#L22-L33) | `env_check` 走 `app_locate_inner(&state, "trae_work")`（顶栏徽标数据源） | 【改】改走 `"trae"`；或直接退役该命令、前端统一用 `env_check_trae_cn`（§5.4 取其一） |
| 17 | [commands/env.rs:45-73](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/env.rs#L45-L73) | `open_trae_app`（Trae Work 打开 + 代理注入 + `graceful_kill_app("TraeWork")` + `persist_detected_path("trae_path")`） | 【删】命令退役（前端改调 `open_trae_cn_app`）；`env.rs:63` 的 `"TraeWork"` 参数随 [commands/process.rs:90-91](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/process.rs#L90-L91) 分支一并删除 |
| 18 | [commands/env.rs:230-232](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/env.rs#L230-L232) | `is_running()` = `any_running(&["TRAE SOLO CN"])` | 【删】（`env_check` 退役后无调用方；若保留 env_check 则改为 `["Trae CN"]`） |
| 19 | [commands/env.rs:264-338](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/env.rs#L264-L338) | `app_profile` 默认分支 = Trae Work 档案（`_ =>` 含 TRAE SOLO 注册表/exe/进程候选、`settings_key: "trae_path"`） | 【改】默认分支改走 Trae（`"trae"|"trae_cn"|"traecn"|"ide"` 分支并入 `_`），TRAE SOLO 候选全删 |
| 20 | [commands/env.rs:494](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/env.rs#L494) | 键映射 `"trae_path" => settings.trae_path` | 【删】 |
| 21 | [commands/switch.rs:99](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/switch.rs#L99) | `TargetApp::parse(target_app.unwrap_or("TraeWork"))` | 【改】默认 `"Trae"` |
| 22 | [commands/switch.rs:163-178,208-212,408-416](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/switch.rs#L163-L178) | JWT 预检/保存守卫的 `is_trae` 判定含 `Some("TraeWork")`、`unwrap_or("TraeWork")` | 【改】`None \| Some("Trae")` |
| 23 | [commands/switch.rs:467-477](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/switch.rs#L467-L477) | `reset_device_ids` 六层设备重置：`_ => "TraeWork"` —— **未升级机器上默认清理目标落空**（TRAE SOLO CN 目录已不存在） | 【改】`Some("Trae") | _ => "Trae"` |
| 24 | [commands/profile.rs:21-49](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/profile.rs#L21-L49) | 通用快照命令双池映射（`Some("Trae") => profiles_trae`，默认 `profiles`）+ `normalize` 默认 "TraeWork" | 【改】映射归一：Trae 域（含默认）恒 `profiles_trae`；normalize 默认 "Trae"；防串台注释更新 |
| 25 | [commands/process.rs:90-91](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/process.rs#L90-L91) | `images_for_app`：`"TraeWork" => ["TRAE SOLO CN.exe","TRAE SOLO.exe","Trae.exe"]` | 【删】分支 |
| 26 | [icube_auth.rs:107-122](file:///d:/code/AIWorkAssistant_main/src-tauri/src/icube_auth.rs#L122) | 本机登录态扫描 `for app in ["Trae CN", "TRAE SOLO CN", "Trae", "Trae Work"]` | 【改】`["Trae CN"]`（保留数组形式以便未来扩展；`source_app` 字段语义同步） |
| 27 | [device_proxy/local_capture.rs:18-34](file:///d:/code/AIWorkAssistant_main/src-tauri/src/device_proxy/local_capture.rs#L18-L34) | `trae_app_dirs`：`%APPDATA%\TRAE SOLO CN` + `%APPDATA%\Trae CN` | 【改】仅 `Trae CN`（对齐 Python `_trae_app_dirs` 注释一并更新） |

**C. 模型与常量（中低影响）**

| # | 位置 | 现状 | 处理 |
|---|---|---|---|
| 28 | [models.rs:175](file:///d:/code/AIWorkAssistant_main/src-tauri/src/models.rs#L175) | `Settings.trae_path` | 【删】字段（`Settings` 无 `deny_unknown_fields`，旧 `app_settings.json` 多余键反序列化安全；配合 P1 迁移清键） |
| 29 | [switcher/locate.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/locate.rs) | 六级 exe 发现（纯消费档案表参数） | 无需改动，随档案表自动收敛 |
| 30 | [trae_checkin.rs:69](file:///d:/code/AIWorkAssistant_main/src-tauri/src/tasks/trae_checkin.rs#L69)、[accounts.rs:121](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/accounts.rs#L121) | 请求头 UA `"VSCode 1.107.1 (TRAE SOLO CN)"` | **暂不改**（服务端白名单未知，见 OQ-3）；注释标注待定 |
| 31 | [usage_history.rs:1](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/usage_history.rs#L1)、[api_server/models_sync.rs:99](file:///d:/code/AIWorkAssistant_main/src-tauri/src/api_server/models_sync.rs#L99) | 注释中「Trae Work 用量接口 / Trae Work 模式」 | 【注】指服务端 API 语义（`solo_work_lite` 协议），非本地双应用，仅措辞澄清 |

### 2.2 前端（src/）

| # | 位置 | 现状 | 处理 |
|---|---|---|---|
| 1 | [types.ts:257](file:///d:/code/AIWorkAssistant_main/src/types.ts#L257)、[types.ts:358](file:///d:/code/AIWorkAssistant_main/src/types.ts#L358) | 注释「TraeWork \| Trae」；`Settings.trae_path` | 【注】【删】`trae_path` 字段 |
| 2 | [store.ts:103-104,132](file:///d:/code/AIWorkAssistant_main/src/store.ts#L103-L104) | `profileApp: 'TraeWork' \| 'Trae'` 快照查看应用状态（F-03 参数化）+ `setProfileApp` | 【删】状态整体退役（单应用后无切换维度）；或收敛为常量 `'Trae'`（§5.12 取实现项 A） |
| 3 | [store.ts:151,157,159](file:///d:/code/AIWorkAssistant_main/src/store.ts#L151) | `switchTo/saveCurrentLogin/resetDeviceIds` 参数联合类型含 `'TraeWork'` | 【改】移除 `'TraeWork'`；`resetDeviceIds` 参数可整体退役（后端恒 Trae） |
| 4 | [store.ts:206](file:///d:/code/AIWorkAssistant_main/src/store.ts#L206) | 初始 settings `trae_path: null` | 【删】 |
| 5 | [store.ts:342](file:///d:/code/AIWorkAssistant_main/src/store.ts#L342) | `profileApp` 默认 `'TraeWork'` | 【删】（随 #2） |
| 6 | [store.ts:823-852](file:///d:/code/AIWorkAssistant_main/src/store.ts#L823-L852) | 打开 Trae Work + 代理注入流程（调 `open_trae_app`、toast 文案 4 处） | 【改】改调 `open_trae_cn_app`；文案改「Trae」 |
| 7 | [store.ts:967-968,1020](file:///d:/code/AIWorkAssistant_main/src/store.ts#L967-L968) | 切换/重置 toast 文案（`targetApp !== 'TraeWork'` 判断、"6 层设备标识重置（Trae/Trae Work）"） | 【改】 |
| 8 | [lib/tauri.ts:330-356](file:///d:/code/AIWorkAssistant_main/src/lib/tauri.ts#L330-L356) | invoke 包装：6 处联合类型含 `'TraeWork'`；`openTraeApp` 包装 | 【改】类型收敛；`openTraeApp` 退役改 `openTraeCnApp` |
| 9 | [pages/Settings.tsx:26,320-324](file:///d:/code/AIWorkAssistant_main/src/pages/Settings.tsx#L26) | `deviceTarget` 默认 `'TraeWork'` + 设备重置目标下拉（`<option value="TraeWork">`） | 【改】默认 `'Trae'`、移除选项（单目标后下拉可整体退役，§5.12） |
| 10 | [pages/Settings.tsx:160-163,241-255](file:///d:/code/AIWorkAssistant_main/src/pages/Settings.tsx#L241-L255) | 「Trae Work 安装路径」配置块 + 自动定位回写 `trae_path` | 【删】整块 |
| 11 | [pages/Settings.tsx:563](file:///d:/code/AIWorkAssistant_main/src/pages/Settings.tsx#L563) | 重置说明文案三元 | 【改】单文案 |
| 12 | [components/TopBar.tsx:82-111](file:///d:/code/AIWorkAssistant_main/src/components/TopBar.tsx#L82-L111) | 顶栏「Trae Work 已安装/未安装」徽标 + 打开/下载按钮（绑 `env_check`/`open_trae_app`） | 【改】改绑 Trae（`env_check_trae_cn`/`open_trae_cn_app`）；与原 Trae 徽标去重 |
| 13 | [pages/Dashboard.tsx:208,238-239](file:///d:/code/AIWorkAssistant_main/src/pages/Dashboard.tsx#L208) | `workLogin` 展示行 + 「未检测到 Trae Work / Trae 安装」文案 | 【改】单应用文案 |
| 14 | [pages/Accounts.tsx:411,487,521-522](file:///d:/code/AIWorkAssistant_main/src/pages/Accounts.tsx#L411)、[727](file:///d:/code/AIWorkAssistant_main/src/pages/Accounts.tsx#L727) | 扫描/空态/「Trae Work 登录中」Badge 文案；应用选择卡 `{ app: 'TraeWork', label: 'TRAE SOLO CN', ... }` | 【改】移除 TraeWork 卡；Badge 语义改「Trae 登录中」 |
| 15 | [pages/accounts/DiscoverModal.tsx:23,29](file:///d:/code/AIWorkAssistant_main/src/pages/accounts/DiscoverModal.tsx#L23) | `apps = ['TraeWork', 'Trae']` 扫描序列 + 双应用说明文案 | 【改】`['Trae']` |
| 16 | [pages/accounts/SnapshotModal.tsx:45-46,59](file:///d:/code/AIWorkAssistant_main/src/pages/accounts/SnapshotModal.tsx#L45-L46) | `profileApp` props + `{ app: 'TraeWork', label: 'Trae Work' }` Tab | 【删】应用切换 Tab |
| 17 | [pages/accounts/HelpModal.tsx:17-62](file:///d:/code/AIWorkAssistant_main/src/pages/accounts/HelpModal.tsx#L17-L62) | 双应用操作说明 6 处（"Trae Work 或 Trae"、"两应用快照相互独立"） | 【改】单应用表述 |
| 18 | [components/SetupGuide.tsx:36-179](file:///d:/code/AIWorkAssistant_main/src/components/SetupGuide.tsx#L36-L179) | 安装引导双应用（打开 Trae Work、下载页选择逻辑） | 【改】单应用 |
| 19 | [lib/about.ts:10](file:///d:/code/AIWorkAssistant_main/src/lib/about.ts#L10) | 「深度支持 Trae Work 与 Trae（Trae CN IDE）双应用」简介 | 【改】 |
| 20 | lib/appIcons.ts | 应用图标候选表 | 无 TraeWork 直接引用（grep 未命中），随 APP_TABS 核对确认即可 |

### 2.3 文档联动（实施时同步）

| 文档 | 更新点 |
|---|---|
| `AGENT.md` | §5.1 双应用架构描述、§7 切换器口径（双池→单池、uid 证据链单源）、§8 命令契约（退役命令清单）、§5 快照白名单表述（TraeWork/Trae → Trae） |
| `README.md` | 产品定位/功能列表/数据目录结构/版本分支说明 4 处双应用表述 |
| `docs/user-manual.md` | 约 10 处操作指引（安装、打开、保存/切换快照、设备重置、设置页路径配置） |
| `docs/tech-framework.md` | §1 架构总览、§2 应用矩阵、§5.3 快照池、§5.5 settings 键 |
| `backlog.md` | 登记本方案条目（编号登记时顺延），引用本文件 |
| `docs/product-design.md` | **不回改**（历史基线定位不变） |

### 2.4 明确不动的域（防误伤清单）

以下内容含 "TraeWork"/"Trae Work" 字样但**与 Trae Work 应用无关**，禁止在本轮整改中触碰：

1. **本应用自身品牌迁移痕迹**：[state.rs:8-17](file:///d:/code/AIWorkAssistant_main/src-tauri/src/state.rs#L8-L17)（`LEGACY_DATA_DIR_NAME="TraeWorkAssistant"` 目录迁移）、[main.rs:75](file:///d:/code/AIWorkAssistant_main/src-tauri/src/main.rs#L75)、[misc.rs:964,1152,1218](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/misc.rs#L964)（旧计划任务 `TraeWorkAssistant_DailyCheckin` 迁移/清理）、[updater.rs:23-24,304](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/updater.rs#L23-L24)（GitHub 仓库 `smart-open/TraeWorkAssistant`）。
2. **服务端协议常量**：`solo_work_lite` / `solo_agent`（签到、用量、模型列表 API 的协议标识，服务端仍按此区分端点语义）。
3. **网关与调度**：Trae 池调度语义、`sticky_bindings`、`pinned_app`（默认 `"trae"`，小写 UI key，与 `TargetApp::TraeWork` 无关）。

---

## 3. 目标架构（合并后）

```
┌────────────────────────── 唯一 Trae 应用身份 ──────────────────────────┐
│ TargetApp::Trae（档案本体不动，见 switcher/profile.rs）                 │
│   app_name:        "Trae"                                              │
│   data_dir:        %APPDATA%\Trae CN            （icube 布局）          │
│   profiles_dir:    <本应用数据>\data\profiles_trae（唯一快照池）          │
│   settings_path_key: "trae_cn_path"            （唯一路径键）           │
│   proc_names:      ["Trae CN"]                                         │
│   exe_candidates:  Programs\Trae CN\Trae CN.exe（首选，本机已命中）      │
└───────────────────────────────────────────────────────────────────────┘
  快照白名单 TRAE_ICUBE_ITEMS 15 项不变（合并版 icube 布局实测吻合）
  uid 推导：storage.json iCubeAuthInfo(dc) + state.vscdb icube_gtm.users(cloud 证据)
           + hybrid 桥标记回退；日志探测依赖 OQ-1 实测结论
  双应用维度全部退役：TraeWork 枚举 / profiles 池 / trae_path 键 /
                     LocalEntitlement.work / 双应用发现循环 / 双 UI 入口
```

退役命令契约（前端同步改）：

| 退役 | 替代 |
|---|---|
| `env_check`（trae_work 语义） | `env_check_trae_cn` |
| `open_trae_app` | `open_trae_cn_app` |
| `LocalEntitlement { work, cn }` | 单值结构（见 §5.3） |
| `profileApp` / `setProfileApp` / SnapshotModal 应用 Tab | 固定 Trae |
| `resetDeviceIds(targetApp?)` 的 TraeWork 分支 | 恒 Trae |

---

## 4. 整改方案：核心设计决策

### 4.1 枚举收敛：删除变体，不设别名

- `TargetApp::TraeWork` 变体删除；`parse` 未知值由「回退 TraeWork」改为 `Err`。
- 依据：① 现有注释明言"调用方为自家前端恒传合法值"，运行时不会收到非法值；② 存量持久化中含 "TraeWork" 字符串的位置**仅两处**——`app_settings.json` 的 `trae_path` 键（随键退役）与快照池目录名（目录名不含应用标识，见 §5.11），均由迁移一次性消化；③ 保留别名会永久保留死代码路径，违背"去掉原有 traework 相关逻辑"的整改目标。
- 影响代码臂：`save_identity_guard`、`switch.rs` 的 `is_trae` 判定、`profile.rs`/`proc.rs`/`icube.rs` 测试——均为 `matches!(.., TraeWork | Trae)` → `== TargetApp::Trae` 或直接删臂。

### 4.2 settings 键归一：退役 `trae_path`，保留 `trae_cn_path`

- 保留 `trae_cn_path`：它已与存续档案（`TargetApp::Trae.settings_path_key`）及 `env.rs` app_profile Trae 分支、前端设置页 Trae 路径块绑定，改动面最小；且合并后客户端官方名就是 "Trae CN"，键名语义准确。
- `Settings.trae_path` 字段删除（serde 默认忽略未知字段，旧 JSON 安全）；`env.rs:494` 键映射删除。
- 迁移规则见 §5.11-①。

### 4.3 entitlement 收敛：`LocalEntitlement { work, cn }` → 单值

- `apps_entitlement_read` 双取改单取（仅 `Trae`），返回结构收敛为：

```rust
pub struct LocalEntitlement {
    pub cn: AppLoginInfo,   // 或重命名 logged_in / info，避免 "cn" 命名歧义
}
```

- 推荐直接平铺为 `AppLoginInfo`（去壳），前端对应 `entitlement.work` 引用点一并清理；`app` 字段恒 `"Trae"`。
- 依据：合并后只有一个客户端，"work/cn 双登录位"概念消亡。

### 4.4 快照池归一：`profiles_trae` 唯一池

- `commands/profile.rs` 通用快照命令映射归一：Trae 域（含默认）恒 `data\profiles_trae`；`data\profiles` 池退役。
- 存量池迁移与去重规则见 §5.11-②；**不合并快照内容**，仅整槽搬迁，保证可回滚。

### 4.5 uid 推导适配：证据链单源 + OQ-1 观察点

- 合并版走 `TargetApp::Trae` 现有推导链：`icube_gtm.users`（state.vscdb cloud 证据）+ 桥标记 `current_account.txt` hybrid 回退 + `storage.json` dc uid；**Trae Work 专属证据源 `solo.mobile.allowControl` 随 kind 退役不再读取**。
- `detect_live_uid` 日志探测（`data_dir\logs\<session>\dynamicConfig.log`）：合并版数据目录顶层未见 `logs/`（OQ-1）。P0-P2 期间**不改代码**（探测 miss 时自然回退 hybrid，无功能损失）；P3 按 OQ-1 实测结论决定是否给 `detect_live_uid` 增加 `User\logs` 等备选路径。

### 4.6 设备重置重定向（switch.rs:467-477）

- `_ => "TraeWork"` 默认清理目标落空（TRAE SOLO CN 数据目录已不存在，重置等于空转）。改为恒 `"Trae"`；前端 Settings 页 `deviceTarget` 状态与下拉随之退役（单目标无需选择）。

---

## 5. 整改方案：逐项实施明细

### 5.1 switcher 层（P0）

1. [switcher/mod.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/mod.rs)：删 `TraeWork` 变体（66-78 行）；`parse` 改 Err（83-93 行）；`save_identity_guard` 单臂（414 行）；F-68 注释（527 行）；测试改写（1207-1288 行，"正在关闭 Trae Work" 断言改 "正在关闭 Trae"）。
2. [switcher/profile.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/profile.rs)：删 TraeWork 档案段（179-203 行）与对应用例（294-301、365-375 行）；新增「未知 kind → Err」用例；模块头注释（2 行）更新。
3. [switcher/proc.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/proc.rs)：测试改用 Trae 档案（404-421 行）；"TRAE SOLO CN.exe" 精确映像名注释（132-133 行）更新为 Trae CN.exe。
4. [switcher/icube.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/icube.rs)：模块头/白名单注释（1、28 行）"TraeWork/Trae 双应用" → "Trae"；测试构造（273 行）改 `TargetApp::Trae`。白名单 15 项**不动**（合并版实测吻合）。
5. [switcher/locate.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/locate.rs)：不改。

### 5.2 命令层（P0）

1. [commands/trae_apps.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/trae_apps.rs)：
   - `app_data_dirs` 删 TraeWork 分支（34-48 行）；
   - `apps_accounts_discover`（400 行）、`backfill_dc_id_for`（488 行）双循环改单应用；
   - dc 回填扫描单池（493 行）；
   - `apps_entitlement_read` 单值化（694-702 行，见 §4.3）；
   - `live_cloud_uid_with`/`marker_with_ts` 的 kind 参数退役或恒 "Trae"（828-944 行测试同步）。
2. [commands/env.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/env.rs)：
   - **方案 A（推荐）**：`env_check`/`open_trae_app`/`is_running` 三者删除，前端改调 `env_check_trae_cn`/`open_trae_cn_app`；`app_profile` 默认分支并入 Trae（TRAE SOLO 候选全删）；键映射删 `"trae_path"`（494 行）。
   - 方案 B：`env_check`/`open_trae_app` 保留为 Trae 的转发别名——不推荐，徒增一层死接口。
3. [commands/switch.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/switch.rs)：默认值 `"TraeWork"` → `"Trae"`（99、212、416 行）；`is_trae` 判定收敛（165、178、415 行）；`reset_device_ids` 恒 "Trae"（474-477 行）。
4. [commands/profile.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/profile.rs)：池映射归一（21-49 行）；`normalize` 默认 "Trae"（49 行）；265 行注释更新。
5. [commands/process.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/process.rs)：删 `images_for_app` TraeWork 分支（90-91 行）。
6. [icube_auth.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/icube_auth.rs)：扫描列表收敛 `["Trae CN"]`（122 行），`source_app` 注释（107 行）同步。
7. [device_proxy/local_capture.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/device_proxy/local_capture.rs)：`trae_app_dirs` 仅 `Trae CN`（18-34 行），模块头注释同步。

### 5.3 tauri 命令契约变更清单（前端同步）

| 命令 | 变更 | 前端动作 |
|---|---|---|
| `env_check` | 删除 | TopBar 改 `env_check_trae_cn` |
| `open_trae_app` | 删除 | store.ts 打开流程改 `open_trae_cn_app` |
| `apps_entitlement_read` | 返回结构去 `work` 字段 | Dashboard/类型同步 |
| `apps_accounts_discover` | `app` 字段恒 `"Trae"` | DiscoverModal `apps` 序列改 `['Trae']` |
| `reset_device_ids` | `target_app` 参数保留但恒 "Trae" | Settings 页下拉退役 |
| `profile_backup/restore/delete/list` | `target_app` 恒 "Trae" | SnapshotModal Tab 退役 |

### 5.4 models / Settings（P0 末）

- [models.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/models.rs)：删 `trae_path` 字段（175 行）。`Settings` 无 `deny_unknown_fields`，旧 `app_settings.json` 含该键可正常反序列化；P1 迁移负责清键。

### 5.5 UA 字面量（暂不动）

- [trae_checkin.rs:69](file:///d:/code/AIWorkAssistant_main/src-tauri/src/tasks/trae_checkin.rs#L69)、[accounts.rs:121](file:///d:/code/AIWorkAssistant_main/src-tauri/src/commands/accounts.rs#L121) 的 `VSCode 1.107.1 (TRAE SOLO CN)`：签到/账号接口 UA 是否被服务端校验未知（OQ-3）。**本轮不改**；仅加注释标注"合并版客户端 UA 待抓包比对"。

### 5.6 前端收敛（P2）

按 §2.2 清单执行，要点：

1. **类型层先行**：`types.ts` 删 `trae_path`；`lib/tauri.ts` 联合类型移除 `'TraeWork'`、`openTraeApp` 退役。
2. **store 层**：`profileApp` 状态与 `setProfileApp` 整体退役（所有消费点固定 `'Trae'`）；打开客户端流程改 `openTraeCnApp`；toast 文案更新。
3. **UI 层**：Settings 页删「Trae Work 安装路径」块、设备重置下拉退役；TopBar 双徽标去重（仅 Trae）；Dashboard 删 workLogin 双行展示；Accounts/DiscoverModal/SnapshotModal/HelpModal/SetupGuide 文案与选择卡收敛。
4. **存量 UI 状态兜底**：若 `profileApp` 曾被持久化且存量值为 `'TraeWork'`，store 初始化时归一为 `'Trae'`（一行防御，随状态退役可在 P3 清理）。

### 5.7 文档联动（P3）

按 §2.3 执行；`backlog.md` 登记条目并勾选进度；CHANGELOG 手动新增（纯文档不升版，代码整改按版本纪律执行）。

### 5.8 存量数据一次性迁移（P1，新增 store/migrate.rs 步骤）

**迁移范围**（accounts 表无应用维度字段，无需改表；涉及两个持久化位置）：

| # | 对象 | 规则 |
|---|---|---|
| ① | `app_settings` kv：`trae_path` | 若 `trae_path` 非空且 `trae_cn_path` 为空：当该路径存在且为文件 → `trae_cn_path ← trae_path`（回写后删 `trae_path` 键）；否则直接删键。`trae_cn_path` 已有值时仅删键不覆盖 |
| ② | 快照池 `data\profiles\*`（TraeWork 槽） | 逐槽搬迁至 `data\profiles_trae\`：目标槽**不存在** → 整槽移动（sidecar 元数据追加 `"source": "TraeWork-migrated"` 标记）；目标槽**已存在**（同 uid 双应用各存一份）→ 不覆盖，源槽移入 `data\profiles_trae_work_legacy\` 保留 |
| ③ | 迁移标记 | kv 写迁移版本号（幂等：已迁移则跳过，重复启动不重跑） |
| ④ | 收尾 | `data\profiles` 为空目录则删除；`profiles_trae_work_legacy` 保留至 P3 验收通过后人工清理 |

**安全与回滚**：

- 迁移前置条件：Trae 进程不在运行（复用切换器的优雅关闭语义，避免 vscdb WAL 残留）；不满足则本轮跳过、下轮启动重试。
- 搬迁是**移动**而非复制：失败即中止并保留原状（池目录整体原样存在即可回滚）；迁移前无需额外备份（移动操作天然可逆：把槽移回去即可；legacy 目录是第二重保险）。
- **TraeWork 存量快照的可用性说明**：快照内容为 `%APPDATA%\TRAE SOLO CN` 的 icube 登录态，与合并版 Trae CN 数据目录同为 icube 布局、白名单 15 项一致，内核层面可恢复；但设备指纹（machineid/aha）生成于旧应用，恢复后首次启动可能触发重新登录或设备认证刷新——属预期行为，在 user-manual 中提示；P1 对本机存量快照抽样实测（OQ-4）。

### 5.9 测试改写汇总

- `switcher/mod.rs`、`profile.rs`、`proc.rs`、`icube.rs`、`commands/trae_apps.rs` 中全部 `TargetApp::TraeWork` / `"TraeWork"` / `TRAE SOLO` 测试构造与断言改写（约 9 处用例，见 §2.1 标【测】行）。
- 新增用例：① `TargetApp::parse` 未知值 → Err；② P1 迁移：构造 `trae_path` 设置 + 双池槽位 → 断言搬迁/去重/幂等/回滚。

---

## 6. 分期实施计划

| 期 | 内容 | 验收口径 |
|---|---|---|
| **P0 后端收敛** | §5.1 + §5.2 + §5.4（编译期收口：删除变体后由编译器暴露全部残留分支） | `cargo test` 全绿；`cargo check` 无 TraeWork 残留引用（`rg "TraeWork|TRAE SOLO" src-tauri/src` 仅剩 §2.4 白名单域与 UA 注释） |
| **P1 数据迁移** | §5.8 迁移步骤 + 本机验证 | 本机：构造/使用存量数据跑迁移 → 快照出现在 `profiles_trae`、`trae_cn_path` 正确、二次启动幂等不重跑；迁移日志完整 |
| **P2 前端收敛** | §5.6 | `tsc` 无错；设置页/顶栏/仪表盘/账号页/弹框无 Trae Work 残留 UI；打开客户端+代理注入走通 |
| **P3 文档与验收** | §5.7 文档联动 + §7 验收清单全量回归 + OQ 实测定论 | §7 全项通过；`rg "TraeWork|TRAE SOLO|Trae Work" src src-tauri/src docs` 残留仅限 §2.4 白名单域 |

依赖关系：P0 → P1 → P2 → P3 串行（P1 迁移代码依赖 P0 的单池路由；P2 前端依赖 P0 命令契约变更）。

---

## 7. 验收清单（本机可验证）

1. **自动发现**：账号页「扫描本机账号」仅探测 `%APPDATA%\Trae CN`，返回账号 `app="Trae"`；无 TRAE SOLO CN 扫描日志。
2. **登录状态读取**：仪表盘登录态/积分正常（`apps_entitlement_read` 单值结构前端正确渲染）。
3. **打开客户端**：顶栏/引导「打开 Trae」启动合并版客户端；代理运行时注入 `--proxy-server`，登录流量经代理捕获成功。
4. **快照保存/切换/回滚**：对合并版客户端保存登录态 → 槽位落在 `data\profiles_trae`；切换他账号 → 15 项白名单恢复、防误覆盖守卫（live uid 探测或 hybrid）生效；`.bak` 两代回滚可用。
5. **设备重置**：Settings 页重置（恒 Trae）→ `%APPDATA%\Trae CN` 六层标识清理正确（不再指向 TRAE SOLO CN）。
6. **迁移验证**：本机存量 `data\profiles`（若有）迁移后可从 Trae 侧列表看到；`trae_path` 键消失；重跑启动无重复迁移日志。
7. **回归**：签到任务、用量历史、网关四池、豆包/WorkBuddy/CodeBuddy/Qoder 全域功能不受影响（回归冒烟）。
8. **静态**：`cargo test` + `tsc` 全绿；`rg` 残留检查符合 P3 口径。

---

## 8. 风险与缓解

| # | 风险 | 等级 | 缓解 |
|---|---|---|---|
| R1 | TraeWork 存量快照恢复进合并版后要求重登/设备重认证 | 中 | 迁移不覆盖已有 Trae 槽；legacy 池保留可回滚；user-manual 提示预期行为；OQ-4 抽样实测 |
| R2 | 合并版不生成 `logs/`，`detect_live_uid` 恒 miss，切换守卫降级为 hybrid 证据 | 低 | hybrid（桥标记 + state.vscdb 证据）已在链路上，功能不失效；OQ-1 实测后决定是否增补探测路径 |
| R3 | UA 字面量与服务端校验不匹配（若本轮改动） | 高（若改） | 本轮不改 UA（OQ-3）；保持 `VSCode 1.107.1 (TRAE SOLO CN)` 原值 |
| R4 | 前端存量持久化 `profileApp='TraeWork'` 导致 UI 异常 | 低 | store 初始化归一 `'Trae'`（§5.6-4） |
| R5 | 迁移中途失败留下半迁移状态 | 低 | 槽级移动原子性（同盘 rename）+ 幂等设计（下轮续迁）+ legacy 池兜底 |
| R6 | 误伤 §2.4 白名单域（品牌痕迹/协议常量） | 低 | 整改时以 `rg` 清单核对，PR 按文件 review |
| R7 | 未升级机器（仍装双应用）跑新版后 TraeWork 快照不可达 | 中 | 版本说明明确"本版本起仅支持合并版 Trae CN"；旧版本可回退使用旧池（迁移只移动不改写内容） |

---

## 9. 待实测项 / 开放问题

| # | 问题 | 验证方法 | 影响 |
|---|---|---|---|
| OQ-1 | 合并版是否生成 `%APPDATA%\Trae CN\logs\<session>\dynamicConfig.log`（当前顶层未见 logs/） | 启动合并版客户端登录后观察 logs 目录 | **已定论（2026-10-14 实测）**：合并版**正常生成** `logs\<yyyymmddThhmmss>\dynamicConfig.log`（本机 20261009 起 12+ 会话目录齐全），`detect_live_uid` 日志探测可用——登录客户端启动 ~1s 内即产生带 `uid=` 的 config query；未登录客户端的会话日志恒无 `uid=`（此差异已转化为三态探测，见 §11-①）。§1.2 的「顶层未见 logs/」侦察记录过时 |
| OQ-2 | 合并版 `state.vscdb` 是否仍写 `solo.mobile.allowControl`（ai-agent 模块并入后） | 合并版登录后读 ItemTable | 若仍写且值有效，可作为附加证据源（非必须） |
| OQ-3 | 签到/账号接口是否校验 UA | 代理抓包合并版客户端请求头 | 决定未来是否更新 UA 字面量 |
| OQ-4 | TraeWork 存量快照恢复进合并版的实际可用性 | P1 迁移后抽样恢复 + 启动验证 | **已实测（2026-10-14）**：迁移搬迁本身无损（槽位文件全量迁入），但**存量槽位存在历史污染**——部分槽内装的并非槽名账号的登录态（详见 §11），恢复后客户端登录为其他账号或停在未登录界面。R1 缓解不足以覆盖此形态，需按 §11 指引逐槽修复 |
| OQ-5 | `bin\trae-cn` CLI 能力与 `TraeCLI桥接落地方案.md` 的联动 | 按 pending 桥接方案实测 | 桥接方案目标应用收敛为 Trae CN，方案文本需在实施时同步 |

---

## 10. 参考

- 本机实测：`C:\Users\tianw\AppData\Local\Programs\Trae CN\`（合并版安装）、`%APPDATA%\Trae CN\`（合并版数据目录，icube 布局）、TRAE SOLO CN 两目录残留（2026-10-14）
- 代码盘点：§2 清单（rg 全量扫描 src/ 与 src-tauri/src，2026-10-14 HEAD）
- 关联文档：`docs/pending/TraeCLI桥接落地方案.md`、`AGENT.md`（工程约定与 uid 红线）、`docs/tech-framework.md`、`docs/user-manual.md`、`backlog.md`
- 机制依赖：F-68 全局键合并、F-80 档案化、双 uid 体系红线（AGENT.md §13）

---

## 11. 实施审查补充（2026-10-14，合并版实测与代码修复）

> 对已实施代码的审查结论。当日实测：本机合并版（Trae CN 3.4.2 / buildVersion 2.3.91092）
> 连续切换 4 账号，结合客户端 `logs\*\dynamicConfig.log` 与 `main.log` 逐会话取证。

### 11.1 实测发现

| # | 发现 | 证据 |
|---|---|---|
| ① | **槽位历史污染是「切换后变成别的账号/未登录」的根因**：部分存量槽内装的并非槽名账号的登录态。恢复后客户端用槽内会话静默换登为其他账号，或会话已死停在未登录界面 | main.log：恢复 2328 槽后客户端走 `login via NEED_REFRESH`，ExchangeToken 换回 `userId=1448502780763114`、`tokenReleaseAt=2026-09-30T09:53:58Z`（与 2328 槽保存时间 2026-09-30 18:04 吻合）；恢复 2117 槽后实际登录为 1335；恢复 2803 槽的会话日志（093318）config query **无 uid 参数**（未登录） |
| ② | **未登录现场回写槽位的恶性循环**：客户端未登录时守卫三源（日志探测 None → hybrid 回退桥标记）fail-open 判「一致」，把未登录现场刷回标记槽，槽位被毁——之后「切换」到该账号恒恢复出未登录态 | switcher.log 09:33:40：把 093318 未登录现场备份进 2803 槽（此前内容仅存一代 .bak） |
| ③ | **恢复后按快照打开 IDE 窗口**：合并版启动窗口类型取 `User\globalStorage\storage.json` 的 `runtimeWindow.lastTask.windowType`（枚举 `Agent/IDE/Floating/TraeWork`，缺省回落 Agent——main.js `_8e`）。快照冻结保存时刻的窗口类型（旧快照 = IDE），故切换恢复后开 IDE 窗口；顶部打开读 live 数据（此前为 Agent）故开 Agent 窗口 | live storage.json `runtimeWindow.lastTask={"windowType":"IDE",...}`；main.js `Xe` 枚举与 `_8e` 回落逻辑 |
| ④ | 迁移本身验证通过：kv `trae_path` 已删、`trae_cn_path` 承接正确、`traework_merge_migrated` 置位、旧池仅剩 `current_account*` 运行时标记（设计内保留）；快照白名单 15 项与合并版布局吻合（备份/恢复恒 13 项 = 15 - wal/shm，正常） | aiwork.sqlite kv 实读 + `profiles_trae` 槽位逐项核对 |

### 11.2 代码修复（随本轮实施一并交付）

1. **三态登录探测**（[switcher/icube.rs](file:///d:/code/AIWorkAssistant_main/src-tauri/src/switcher/icube.rs) `LiveUidProbe`）：`Uid`（日志直探权威）/ `SessionNoUid`（会话日志存在但无 uid = 未登录）/ `NoEvidence`（无法判定，维持原 fail-open）。合并版 logs/ 正常生成（OQ-1），该差异可靠。
2. **未登录现场禁止回写槽位**（`switch_flow` stop 后权威重探）：`SessionNoUid` → 跳过标记槽回写并 Warn 专项文案，斩断 ② 的恶性循环。
3. **L2 保存守卫拒未登录**（`save_identity_guard`）：`SessionNoUid` → fail-closed 拒绝保存（还原现场 + 明确指引），防止把未登录现场存进任意账号槽。
4. **切换后登录身份确认**（`confirm_switch_uid`，Trae 专属）：启动后轮询 20s 只认**比 start 前更新的会话**的 uid；`Mismatch`（实际登录为他账号，快照污染）/ `Timeout`（会话失效未登录）均在 done 文案中如实告知，不再静默报「切换成功」。
5. **Agent 窗口修正**（`apply_agent_window_fixup`）：恢复快照后（`AppProfile.force_agent_window`，仅 Trae）与顶部打开前（`open_trae_cn_app`，客户端未运行时）把 `runtimeWindow.lastTask` 补丁为 Agent——所有打开路径恒开 Agent 窗口，与用户预期一致。

### 11.3 存量污染槽位修复指引（本机）

代码修复只阻断**进一步**污染，已污染槽位需按槽修复（三选一）：

1. **重建（推荐）**：在客户端手动登录该账号（勿用「切换」）→「保存当前登录态」覆盖该槽（修复后守卫会做身份校验）；
2. **回滚代际**：`<uid>.bak`/`.bak2` 中若还有污染前的旧快照，可手动整体替换主槽后从 Trae 侧验证；
3. **删除重入池**：删除污染槽位后重新扫描本机账号入池。

已知污染（2026-10-14 实测）：`2328112497170937`（实为 1448 会话）、`2117003799429594`（实为 1335 会话）、`2803094668190106`（09:33:40 被未登录现场覆盖，`2803094668190106.bak` 为污染前内容）；其余槽位建议切换一次并留意切换后的身份确认警告。
