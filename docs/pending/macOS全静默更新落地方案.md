# macOS 全静默更新落地方案

> 状态：**方案设计稿（未实施）**
> 前置：Issue #37（Windows 更新竞态）与 macOS 手动安装「先退出再拖拽」引导已落地（见 `installer-hooks.nsh` / `updater.rs` / `AboutDialog.tsx`，git diff 5 文件 +118/-20）。
> 本文描述在其之上追加「macOS 一键静默更新」的完整改造方案。dmg 手动拖拽流程**全程保留为兜底路径**，不做零点击后台自动更新（见 §2 非目标）。

---

## 1. 背景与动机

### 1.1 现状流程（已修复版）

1. 用户在「关于」弹窗点「下载更新」→ `update_download` 下载 dmg 并校验 SHA-256（latest.json manifest，fail-closed）；
2. 点「打开安装包」→ `update_run_installer` mac 分支执行 `open <dmg>` 挂载并弹 Finder 窗口；
3. 用户按弹窗三步引导操作：**① 点按钮退出本应用（`update_quit_app`）→ ② 拖 .app 入 Applications 替换 → ③ 从启动台重开 + Gatekeeper 放行**；
4. 「重启应用」（`update_restart_app`）作为兜底按钮保留。

### 1.2 痛点

- **手动步骤多**：退出、挂载 dmg、拖拽、重开，4 步人工操作，任何一步走神即失败；
- **Gatekeeper 弹窗**：dmg 经浏览器/本应用下载携带 `com.apple.quarantine` xattr（ad-hoc 签名无公证），首次打开必须走系统弹窗放行引导，体验差且易被误判「应用损坏」；
- **Finder 无法替换运行中的应用**：这是手动流程被迫「先退出」的根因（本次修复加的引导），但根因无法在手动流程里消除。

### 1.3 静默更新的核心收益

| 收益 | 原理 |
| --- | --- |
| 一键完成 | 应用退出后由 helper 脚本自动「备份→替换→重开」，用户零拖拽 |
| **无 Gatekeeper 弹窗** | 应用自己 spawn 下载并解包的文件**不带 quarantine xattr**（不经过 Finder/浏览器下载链路），Gatekeeper 首次校验不介入 |
| 跨版本签名一致 | 替换的是同一条流水线产物，ad-hoc 身份由构建机产出，替换后 `open` 直接激活 |
| 与 Windows 行为对齐 | Windows 已是「下载→点安装→自动重启」两步，mac 对齐后双端体验一致 |

---

## 2. 目标与非目标

**目标**

- 用户在「关于」弹窗一次点击：下载 → 校验 → 解包 → 退出应用 → 自动替换 → 自动重开新版；
- 任意环节失败**不退出当前应用**，报错并自动回退到现有 dmg 手动流程；
- /Applications 可写性差异（管理员安装 vs 用户安装）均能处理。

**非目标（明确不做）**

- ❌ **零点击后台自动更新**：需要开机自启 + 常驻轮询（LaunchAgent），且「应用在用户不知情时重启」违背桌面软件惯例。本方案为**一键式**（用户点确认才执行）；
- ❌ 引入 Tauri 官方 updater 插件：插件强依赖签名公钥体系与自家 latest.json 格式，改造面更大且与现有自研四命令体系冲突；
- ❌ 公证（notarization）：无付费 Developer ID，不在本次范围。

---

## 3. 方案选型

| 候选方案 | 结论 | 理由 |
| --- | --- | --- |
| A. Tauri 官方 updater 插件 | ❌ | 需要引入插件 + 专属签名密钥 + 改造 latest.json 格式，与自研校验体系（`lookup_manifest_hash`）重叠冲突 |
| B. 应用内直接替换（Rust 全做） | ❌ | 替换动作必须在**本进程退出后**执行，Rust 内无法完成后半段；`mv` 运行中 bundle 虽 POSIX 允许，但 dyld 已映射文件句柄在新路径仍指向旧 inode，热替换后本进程状态混乱 |
| **C. helper 脚本方案（推荐）** | ✅ | 下载/校验/解包在应用内做（失败可即时反馈 UI），仅「等 PID 退出 → mv 备份 → mv 替换 → open → 回滚」这一小段交给 detached shell 脚本，~30 行 sh，失败面最小 |

**方案 C 执行时序**：

```
应用进程 (v3.6.2)                          helper.sh (detached)        LaunchServices
─────────────────                          ────────────────────        ──────────────
update_download 下载 .app.tar.gz
  └ sha256 校验（manifest）
update_apply_silent_update:
  ├ 防御校验（路径/版本/空间）
  ├ tar 解包到同卷暂存 .app.new
  ├ 校验 Info.plist 版本 == 目标
  ├ 写 helper.sh + spawn (detached) ───────► 轮询 kill -0 <pid> 等
  ├ app.exit(0)（走清理链）                  旧进程退出（≤30s 超时）
                                            mv <bundle> → <bundle>.bak
                                            mv <bundle>.new → <bundle>
                                            open <bundle> ─────────────► 拉起新版 (v3.6.3)
                                            (失败: mv .bak 回滚)
                                            (成功: 延迟删 .bak)
```

---

## 4. 实施计划（四阶段）

### P1 流水线：产出 `.app.tar.gz` 资产（~50 行）

#### 4.1.1 `build-macos.yml`

三个矩阵任务（aarch64 / x64 / universal）构建命令追加 app bundle 产物：

```yaml
# 原：--bundles dmg
npm run tauri build -- --target ${{ matrix.target }} --bundles app,dmg --config tauri.signing.conf.json
```

- `app` bundle 产出路径：`src-tauri/target/<triple>/release/bundle/macos/<product>.app`；
- dmg 重试逻辑不变（dmg 仍是手动兜底产物）；
- artifact 上传路径追加 `release/mac/*.app.tar.gz`。

#### 4.1.2 `scripts/rename_release.mjs`

新增「打包 + 哈希」段（置于 dmg jobs 之后）：

```js
// macOS 静默更新资产：把 bundle/macos/<product>.app 打成 tar.gz 并计入清单
// （保留符号链接与可执行权限位，必须用 tar 而非 zip）
const appBundleSrcDirs = [
  resolve(SRC_TAURI, 'target/release/bundle/macos'),
  resolve(SRC_TAURI, 'target/aarch64-apple-darwin/release/bundle/macos'),
  resolve(SRC_TAURI, 'target/x86_64-apple-darwin/release/bundle/macos'),
  resolve(SRC_TAURI, 'target/universal-apple-darwin/release/bundle/macos'),
];
const silentJobs = [
  { dir: appBundleSrcDirs, suffix: 'aarch64', requiredBy: 'aarch64' },
  { dir: appBundleSrcDirs, suffix: 'x64',     requiredBy: 'x64' },
  { dir: appBundleSrcDirs, suffix: 'universal', requiredBy: 'universal' },
];
for (const j of silentJobs) {
  if (process.env.TAURI_TARGET && !process.env.TAURI_TARGET.startsWith(j.suffix === 'x64' ? 'x86_64' : j.suffix)
      && !(j.suffix === 'universal' && process.env.TAURI_TARGET === 'universal-apple-darwin')) continue;
  const src = j.dir.map(d => join(d, product)).find(p => existsSync(p));
  const outName = `${product}_${version}_${j.suffix}.app.tar.gz`;
  if (!src) { /* 同现有 WARN/die 策略：本任务必需则 die */ }
  // -h 跟随符号链接？否——.app 内 Frameworks 符号链接必须保留，用默认行为 + 归一 cd 到父目录打包
  execSync(`tar -czf "${join(outDir, outName)}" -C "${dirname(src)}" "${product}.app"`);
}
```

关键点：

- **必须 `tar` 而非 zip**：`.app/Contents/Frameworks/` 内有符号链接与可执行位，zip 不保；
- 打包时 `-C <父目录> <product>.app` 保证解出即得 `XXX.app` 目录，不带绝对路径；
- 命名对齐现有资产惯例：`AI Work 助手_3.6.2_aarch64.app.tar.gz` / `_x64.` / `_universal.`；
- sha256 计入 `latest.json`（现有 assets 循环追加三个 `.app.tar.gz` 键即可，`REQUIRED_ON_THIS_HOST` 按 TAURI_TARGET 同步判定，缺失即 die）；
- 清单合并逻辑（跨平台 previous latest.json 继承）无需改动，自动生效。

#### 4.1.3 产物矩阵

| TAURI_TARGET | 新增资产 | 用途 |
| --- | --- | --- |
| aarch64-apple-darwin | `*_aarch64.app.tar.gz` | Apple Silicon 静默更新 |
| x86_64-apple-darwin | `*_x64.app.tar.gz` | Intel 静默更新 |
| universal-apple-darwin | `*_universal.app.tar.gz` | 双架构兜底（同 dmg 策略） |

---

### P2 后端：`updater.rs` 静默更新命令（~150-200 行）

#### 4.2.1 `asset_suffixes()` mac 分支扩展（[updater.rs#L176-L183](file:///d:/code/AIWorkAssistant/src-tauri/src/commands/updater.rs)）

```rust
#[cfg(target_os = "macos")]
fn asset_suffixes() -> &'static [&'static str] {
    if std::env::consts::ARCH == "x86_64" {
        // .app.tar.gz（静默更新）优先于 dmg（手动兜底）
        &["_x64.app.tar.gz", "_universal.app.tar.gz", "_x64.dmg",
          "_universal.dmg", "_aarch64.dmg"]
    } else {
        &["_aarch64.app.tar.gz", "_universal.app.tar.gz", "_aarch64.dmg",
          "_universal.dmg", "_x64.dmg"]
    }
}
```

- `pick_asset`（L186-210）按 `ends_with` 匹配 + 优先级排序，**无需改动**：`.app.tar.gz` 与 `.dmg` 后缀互不误配（`"_x64.dmg"` 不会被 `"_x64.app.tar.gz"` ends_with 命中，反之亦然）；
- 语义变化：`update_check` 返回的资产默认变为 tar.gz——**前端据此展示「一键更新」入口，若 pick 到 dmg（上游仅发 dmg 的老 Release）自动回落手动流程**。

#### 4.2.2 新命令 `update_apply_silent_update`

签名与防御骨架复用 `update_run_installer`（L653-671）既有模式：

```rust
/// macOS 一键静默更新：解包 .app.tar.gz → 写 helper 脚本 → 退出应用。
/// 任意一步失败都不退出应用（前端回落 dmg 手动流程）。
#[tauri::command]
pub fn update_apply_silent_update(
    app: AppHandle,
    file_path: String,
    asset_name: String,
) -> Result<(), String> {
```

**步骤 1：防御校验**（复用现有三件套，不新造）

| 防御 | 复用/新增 |
| --- | --- |
| 路径必须位于 `temp_dir()/ai-work-assistant-update` | 复用 L660-666 逻辑 |
| 资产版本 > 当前版本 | 复用 L667-671 逻辑 |
| SHA-256 已在 `update_download` 阶段 fail-closed 校验 | 无需重复 |
| 磁盘剩余空间 ≥ 解包体积 × 2（tar.gz size 已知，估 ×3 for .app） | 新增 `fs2`-free 简查：`statfs` 不可用则跳过（防御性，不阻塞主流程） |

**步骤 2：解包到「同卷暂存目录」**

```rust
// 目标替换目录 = 当前 bundle 父目录（通常 /Applications，但兼容 ~/Applications
// 与自定义拖放位置）——从 current_exe 反推，与 update_restart_app L740-744 同源
let bundle = std::env::current_exe().ok()
    .and_then(|e| e.ancestors().nth(3).map(|a| a.to_path_buf()))
    .filter(|b| b.extension().map(|x| x == "app").unwrap_or(false))
    .ok_or("无法定位当前应用 bundle")?;
let install_dir = bundle.parent().ok_or("无法定位安装目录")?;

// 同卷暂存：跨卷 mv 是 copy+delete（非原子，中断=半成品），必须先判定 st_dev
// 同卷 → 解包到 install_dir/.update-staging-<pid>/
// 跨卷 → 落 install_dir 需要写权限，多半不可写，直接走提权路径（见步骤 4）
let staging = install_dir.join(format!(".aiwa-update-staging-{}", std::process::id()));
std::fs::create_dir_all(&staging).map_err(|e| {
    // /Applications 不可写（管理员安装场景）→ 交给提权 helper 整体处理
    format!("暂存目录创建失败（可能无写权限）: {e}")
})?;
crate::platform::cmd::sys_command("tar")
    .args(["-xzf", &file_path, "-C"])
    .arg(&staging)
    .status().and_then(|s| s.exit_ok())
    .map_err(|e| { let _ = std::fs::remove_dir_all(&staging); format!("解包失败: {e}") })?;
```

**步骤 3：解包产物校验**

```rust
// 解出结构必须是 <staging>/<product>.app/Contents/{MacOS,Info.plist}
let new_app = staging.join(product_name()); // productName 从 current_exe bundle name 取，不硬编码
let plist_ver = read_info_plist_value(&new_app, "CFBundleShortVersionString")
    .ok_or("解包产物缺少 Info.plist 版本")?;
if cmp_version(parse_version(&plist_ver)?, current) != Ordering::Greater {
    let _ = std::fs::remove_dir_all(&staging);
    return Err("解包产物版本异常，已中止".into());
}
// 可执行文件存在性：Contents/MacOS/<exe>（exe 名 = current_exe().file_name()）
```

- `read_info_plist_value` 复用 [switcher::locate](file:///d:/code/AIWorkAssistant/src-tauri/src/switcher/locate.rs) 现有实现（`update_restart_app` 已在用）；
- 失败一律清暂存 + 返回 Err，**不退出应用**。

**步骤 4：写 helper 脚本并 detached spawn**

```sh
#!/bin/sh
# aiwa-silent-update helper：本进程退出后由它完成替换（失败自动回滚）
OLD="<bundle 路径>"
NEW="<staging>/<product>.app"
PID=<应用 pid>

# 1. 等应用退出（kill -0 探活，120 次 × 250ms = 30s 超时后强制继续）
i=0
while kill -0 "$PID" 2>/dev/null && [ $i -lt 120 ]; do
  sleep 0.25; i=$((i+1))
done

# 2. 备份旧版 → 替换 → 校验 → 重开（任一步失败回滚）
if mv "$OLD" "$OLD.bak" && mv "$NEW" "$OLD"; then
  sleep 1
  if open "$OLD"; then
    # 重开成功：延迟清理备份（新版正常拉起后旧版无用）
    (sleep 60 && rm -rf "$OLD.bak") &
    exit 0
  fi
  # open 失败 → 回滚
  mv "$OLD" "$NEW"; mv "$OLD.bak" "$OLD"
fi
# 回滚/失败：清暂存，旧版未动，用户下次仍可走手动流程
rm -rf "$NEW" "$OLD.bak" 2>/dev/null
```

```rust
// spawn：完全脱离父进程生命周期
// setsid 需 libc；简化：sh -c 'nohup sh helper.sh >/dev/null 2>&1 &'
crate::platform::cmd::sys_command("sh")
    .arg("-c").arg(format!("nohup sh {:?} >/dev/null 2>&1 &", helper_path))
    .spawn().map_err(|e| { let _ = std::fs::remove_dir_all(&staging); format!("helper 启动失败: {e}") })?;
let _ = app.emit("update-installing", asset_name);
// 走 RunEvent::Exit 清理链（停代理/停 API 服务/还原系统代理），绝不用 std::process::exit
app.exit(0);
```

**/Applications 不可写分支（提权兜底）**：步骤 2 暂存目录创建失败时，不报错终止，改走：

- helper 脚本整体内容改为经 `osascript -e 'do shell script "…" with administrator privileges'` 执行；
- **注入防御**：脚本内所有路径变量必须单引号包裹 + 内部单引号转义（`'` → `'\''`），或改用 base64 编码脚本体传递（推荐后者，零转义面）：
  ```sh
  osascript -e "do shell script \"echo <b64> | base64 -d | sh\" with administrator privileges"
  ```
- 提权弹密码框失败（用户取消）→ 返回 Err，前端回落手动流程。

**步骤 5：残留自愈（下次启动时）**

在应用启动路径（`main.rs` setup 或 `update_check` 入口）加一段 ~15 行的清理：

```rust
// 自愈：上次静默更新可能残留 .bak / staging（断电、helper 被杀等）
// 规则：当前 bundle 旁若存在 <bundle>.bak 且当前磁盘版本 == 运行版本 → 替换已回滚或未完成，删 .bak
//       若 .aiwa-update-staging-* 存在 → 直接删（失败遗留）
```

#### 4.2.3 命令注册

- [main.rs](file:///d:/code/AIWorkAssistant/src-tauri/src/main.rs) invoke_handler 追加 `update_apply_silent_update`；
- [tauri.ts](file:///d:/code/AIWorkAssistant/src/lib/tauri.ts) `api.updater` 追加 `applySilentUpdate: (p: string, a: string) => invoke('update_apply_silent_update', {...})`。

---

### P3 前端：`AboutDialog.tsx` 一键更新（~80 行）

现 mac 视图状态机：`downloading → downloaded → installing（三步引导卡片）`，改为：

```
downloading → downloaded
   ├ 资产是 .app.tar.gz（新 Release）→ [一键更新] 主按钮
   │    点击 → applySilentUpdate()
   │    ├ Ok  → 「正在完成更新…应用将自动重启」（应用很快退出，无需后续 UI）
   │    └ Err → toast 报错 + 降级到 installing 三步引导（把 dmg 流程补跑：
   │            资产是 tar.gz 时提示「已回退手动模式，请重新下载 dmg 版」或
   │            直接 pick dmg 资产重新 download——推荐后者，改 2 处状态即可）
   └ 资产是 .dmg（上游未发 tar.gz 的历史 Release）→ 现有「打开安装包」三步引导，原样保留
```

- 三步引导卡片（退出→拖拽→重开 + Gatekeeper 指引）**全部保留**，作为 dmg 兜底 UI；
- `quitting` 状态、`quitForInstall`、`update_quit_app` 不动（dmg 流程仍依赖）。

---

### P4 真机验证矩阵（合入前必须全绿）

| # | 场景 | 环境 | 预期 |
| --- | --- | --- | --- |
| M-1 | 标准路径一键更新 | aarch64 真机，应用装于 /Applications，用户管理员 | 一键后应用退出→自动重开新版本，全程无 Gatekeeper 弹窗 |
| M-2 | Intel 真机 / universal 包 | x64 或 universal 产物 | 同 M-1 |
| M-3 | 标准用户权限（/Applications 不可写） | 应用由管理员装，当前用户标准权限 | osascript 提权弹密码 → 输入后完成更新；取消则回落手动 |
| M-4 | 自定义安装路径 | 应用拖到 ~/Applications 或桌面 | 替换发生在当前 bundle 所在目录，非硬编码 /Applications |
| M-5 | 替换中途失败回滚 | kill helper 于 mv 替换后、open 前 | 旧版 .bak 恢复或下次启动自愈清理，应用仍可打开 |
| M-6 | 下载中断重试 | 断网重试 | 现有断点/重下逻辑不受影响 |
| M-7 | dmg 兜底路径 | 上游仅发 dmg 的历史 Release | 三步引导原样可用 |
| M-8 | LS 激活时序 | M-1 基础上重复 10 次 | `open` 稳定拉起新版（对应 update_restart_app 注释中遗留的时序竞争观察点） |
| M-9 | 符号链接完整性 | 更新后 `codesign -v --deep` + Frameworks 功能 | tar.gz 解包不破坏符号链接与签名结构 |

---

## 5. 风险登记

| # | 风险 | 等级 | 缓解 |
| --- | --- | --- | --- |
| R1 | **跨卷 mv 非原子**：temp_dir 与 /Applications 可能跨卷（外接盘/特殊分区），copy+delete 中断留半成品 | 高 | P2 步骤 2 强制同卷暂存（staging 建在 install_dir 旁），并校验 st_dev |
| R2 | **osascript 提权注入**：路径含空格/引号拼进 shell | 高 | 脚本体 base64 编码传递，零拼接转义面 |
| R3 | **替换中断电/helper 被杀**：.bak 与新版并存 | 中 | helper 内回滚逻辑 + 应用启动自愈清理（P2 步骤 5） |
| R4 | **LS 激活时序竞争**：`open` 提交时旧进程尚未完全注销，LS 可能激活旧实例 | 中 | helper `sleep 1` 后再 open（kill -0 已确认退出）；M-8 观察项 |
| R5 | **tar 解包破坏符号链接/权限** | 中 | 用系统 `tar`（bsdtar 原生支持）；M-9 验证 |
| R6 | **磁盘空间不足** | 低 | 下载前简单检查；不足时报错不退出 |
| R7 | **无 quarantine 的安全性权衡**：静默更新绕过 Gatekeeper 首检 | 说明项 | 资产 SHA-256 fail-closed 校验（现有 manifest 体系）承担完整性；ad-hoc 签名下 Gatekeeper 本就拦不住有意者，净收益为正 |
| R8 | **新流对老版本不可用**：v3.6.2 及以前只认 .dmg | 说明项 | 无需兼容——老版本继续走 dmg 引导；新版本起 `asset_suffixes` 才含 .app.tar.gz |

---

## 6. 工作量与涉及文件汇总

| 阶段 | 文件 | 预估改动 |
| --- | --- | --- |
| P1 | `.github/workflows/build-macos.yml` | ~10 行（bundles 追加 app、artifact 路径） |
| P1 | `scripts/rename_release.mjs` | ~40 行（tar.gz 打包 + 3 个哈希键 + 必需性判定） |
| P2 | `src-tauri/src/commands/updater.rs` | ~150-200 行（suffixes 扩展 + apply_silent_update + 提权兜底 + 自愈） |
| P2 | `src-tauri/src/main.rs` | +1 行注册 |
| P3 | `src/lib/tauri.ts` | +3 行 |
| P3 | `src/components/AboutDialog.tsx` | ~80 行（一键按钮 + 回退分支） |
| P4 | 真机验证 | M-1 ~ M-9 |

**合计 ~300-330 行**，无新依赖（`tar`/`sh`/`osascript` 均为 macOS 系统自带），成本大头在防御路径与 P4 真机矩阵。

---

## 7. 与既有修复的关系

- 本方案**不改动**已落地的 Issue #37 修复（`installer-hooks.nsh` UpdateMode 竞态、`CREATE_BREAKAWAY_FROM_JOB` 加固）——那是 Windows 侧逻辑；
- macOS 现有三步手动引导（`update_quit_app` + AboutDialog 卡片）**保留为兜底**，`update_restart_app` 两道防御原样复用；
- `update_run_installer` mac 分支（L677-685）不动，仅前端在 tar.gz 资产场景不再调用它。
