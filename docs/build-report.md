# 构建与验证记录（AIWorkHub v4.0.0）

> 记录本机实际执行的命令与结果，供复现与排障。所有命令在 `E:\study\gitproject\aiwork\AIWorkHub` 下执行。

## 1. 环境

| 项 | 值 |
|---|---|
| 操作系统 | Windows（x64） |
| Node | v24.20.0（npm 11.19.0 / pnpm 10.13.1） |
| Rust | 1.99.0（`rustc 1.99.0 (b940084d7 2026-09-28)`） |
| MSVC | Visual Studio Build Tools 18 · MSVC 14.51.36231 · Windows SDK 10.0.26100 |
| WebView2 | 已安装（Tauri 运行时） |

### ⚠️ 本机两个环境坑（必读）

1. **`.cargo\bin` 下的 `cargo.exe` / `rustc.exe` 是 0 字节坏桩**，直接 `cargo` 会报「不是有效的应用程序」。
   必须把真实工具链前置到 PATH：
   ```powershell
   $env:PATH="C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;$env:PATH"
   ```
2. **链接与 C 依赖需要 MSVC 环境**（`link.exe` / `cl.exe` / `INCLUDE` / `LIB`）。
   `libsodium-sys-stable`（Stronghold 依赖）在缺 MSVC 环境时会放弃源码编译、转而下载预编译 zip，
   下载超时即构建失败。正确做法是用 `vcvars64.bat` 初始化环境后再跑 cargo：
   ```powershell
   cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set PATH=C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;%PATH% && cargo build --release'
   ```
   若仍卡在 libsodium 下载，可预置归档到 crate 源码目录（构建脚本会优先用本地文件并校验 minisign）：
   `…\registry\src\<mirror>\libsodium-sys-stable-1.24.0\libsodium-1.0.22-stable-msvc.zip` 与同名 `.zip.minisig`。

## 2. 前端

| 步骤 | 命令 | 结果 |
|---|---|---|
| 依赖安装 | `npm install` | ✅ 211 包 |
| 类型检查 | `npx tsc --noEmit` | ✅ 0 error（新增 2 页面 + 24 DTO + api 封装） |
| 单测 | `npm run test`（vitest） | ✅ 6 文件 / 59 用例全通过 |
| 产物构建 | `npx vite build` | ✅ `dist/`（JS ~1.39 MB / CSS 63 kB） |

## 3. Rust

| 步骤 | 命令 | 结果 |
|---|---|---|
| 依赖 + 类型检查 | `cargo check` | ✅ 通过（含新增 5 个 api_server 模块 + 2 个 commands 模块） |
| 单测 | `cargo test` | ✅ **987 passed / 0 failed / 11 ignored**（ignored 为既有网络实测项） |

新增模块的单测覆盖（30 个用例）：

| 模块 | 用例数 | 关键断言 |
|---|---|---|
| `commands::agent_import` | 12 | YAML 子集解析（序列/嵌套）、`.env`、Claude env → 上游、Codex `model_providers` 识别、Continue YAML、额外目录网关识别、`config.toml` 合并幂等、导入幂等 + 历史累积、`ck_` Key 复用不重复 |
| `api_server::upstream_pool` | 8 | SWRR 3:1 → 6:2、连续 3 次失败熔断退出候选、401 直压最长熔断、成功清零但保留 402 停放、priority 保序、保存校验/去重、失败窗口重置、402 停放落在 UTC+8 10:00 |
| `api_server::pricing` | 5 | 内容寻址 id 稳定且调价换 id、分档命中最小覆盖档、记账 + 按日聚合、未定价点名、兜底档必须最后 |
| `api_server::activity` | 4 | 白名单投影丢弃未知字段、`sk-`/`ck_`/`Bearer` 脱敏、按键名掩码、环形 500 + JSONL |
| `api_server::modelsdev` | 4 | 最长前缀家族兜底、远程缓存优先、四级回落、过期缓存不命中 |
| `api_server::atrest` | 6 | 往返 + key_id 稳定、口令错快速失败、密文篡改拒绝、`code` 改动拒绝（AAD 绑定）、空口令拒绝、JSON 往返 |
| `commands::hub` | 2 | 非法端点探测不 panic、时间戳单调 |

> 测试期间发现并修掉 3 个真实缺陷：
> ① 分档计价误实现为「最后一个 ≤ 用量的档」，会让常规小请求一路落到最贵档 → 改为「覆盖用量的最小上限档」；
> ②③ 上游池测试共用全局运行态且每例 `reset_runtime()`，并行执行时互相清空中断（SWRR 分布 7:1、熔断未生效）→ 去除测试内的全局清空，按模型名隔离。

## 4. 安装包（Tauri bundle）

| 步骤 | 命令 | 结果 |
|---|---|---|
| 发布构建 + 打包 | `npm run tauri build`（前端 prebuild + cargo release + msi/nsis） | ✅ `Finished release profile in 3m 21s`，2 个 bundle 均产出 |
| 产物归集 + 便携版 | `node scripts/rename_release.mjs` / `node scripts/package_portable.mjs` | ✅ 输出到 `release/` 并生成 SHA256 |

实测产物：

| 文件 | 大小 | SHA256 |
|---|---|---|
| `release/AI Work 助手_4.0.0_x64-setup.exe`（NSIS） | 9.01 MB | 每次构建变更，见 `release/latest.json` |
| `release/AI Work 助手_4.0.0_x64_zh-CN.msi`（MSI） | 12.86 MB | 同上 |
| `release/AI Work 助手_4.0.0_x64_portable.zip`（便携版） | 12.55 MB | 同上 |
| `src-tauri/target/release/ai-work-assistant.exe`（裸可执行） | — | — |
| `dist/`（前端产物，已被 exe 内嵌） | — | — |

> 唯一的构建告警是 linker 的 `LNK4099: 未找到 PDB "libsodium.pdb"`——预编译 libsodium 静态库不带调试符号，
> 属噪声级告警，不影响产物（release 构建 `warning: 1`）。

### 可执行文件冒烟测试

```powershell
& "src-tauri\target\release\ai-work-assistant.exe" --task-run __smoke_invalid__
# → {"error":"未知任务: __smoke_invalid__","ok":false}  （进程正常启动、参数解析与 CLI 任务模式均工作）
```

## 5. 一键复现脚本

```powershell
cd E:\study\gitproject\aiwork\AIWorkHub
npm install
npx tsc --noEmit
npm run test
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set PATH=C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;%PATH% && cargo test --manifest-path src-tauri\Cargo.toml'
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set PATH=C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;%PATH% && npm run tauri build'
node scripts/rename_release.mjs     # 产物统一输出到 release/ 并按中文名重命名
```

---

## 6. 2026-10-18 回归修复（本地账号扫描 / 签到）

### 6.1 现场证据

| 来源 | 观察 |
|---|---|
| 应用日志 `%APPDATA%\AIWorkAssistant\logs\app.log` | `[调度器] Qoder 积分数据同步：{"ok":true,"skipped":"无 Qoder 账号"}`、`Trae 每日签到：成功 0 / 失败 1`、`获取剩余积分失败 [Trae-…4216]: status 401 code 1001` |
| `aiwork.sqlite` | `qoder_accounts` 0 行；`accounts` 里 Trae 账号 `jwt` **长度为 0**（自动发现占位）、无 refresh_token；`checkin_results` 一行 `status=fail` |
| 本机目录 | `%APPDATA%\QoderCN` **不存在**；实际登录态在 `%APPDATA%\com.qodercn.app.stable`（CN）与 `%APPDATA%\com.qoder.app.stable`（INTL），均为 `Local State` + `auth.v1.dat`、**无** `state.vscdb` |
| WB 凭据 | `%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\`：`workbuddy-desktop.info`（CN，`accessToken` 为 `$wbEncrypted` 信封）+ `workbuddy-desktop-ai.info`（INTL，明文 JWT，exp 2027） |

### 6.2 修复后验证

```powershell
# 真机自检（#[ignore] 用例，只读本机登录态、不打印 token）
cargo test qoder_real_scan_smoke -- --ignored --nocapture
# 命中 C:\Users\Administrator\AppData\Roaming\com.qodercn.app.stable
#   channel=auth_v1 realm=cn uid=01a12a3d-… name=HaoDuoYV exp=1794642488000 refresh=true
# test result: ok. 1 passed
```

| 项 | 结果 |
|---|---|
| `cargo test`（全量） | ✅ **988 passed / 0 failed / 12 ignored**（新增 1 用例：`scan_auth_v1` 全链路 + 目录分类） |
| `cargo check` | ✅ |
| `npx tsc --noEmit` | ✅ |
| Qoder 扫描 | ✅ 命中 `com.qodercn.app.stable`（CN，`auth.v1.dat` 通道）；INTL 目录按设计跳过并说明原因 |
| WorkBuddy 扫描 | ✅ 跳过 CN 信封文件，命中 `workbuddy-desktop-ai.info` 明文 JWT（uid `efe51771-…`，有效期至 2027） |
| 签到无登录态账号 | ✅ 由 `fail` 改为 `skip`（`skip_reason=no_jwt`），并写入本轮 warnings |
| 积分查询空 JWT | ✅ 直接返回「需代理捕获或 OAuth 重新登录」，不再拿上游 401 |

> 说明：CN 版 WorkBuddy 的 `accessToken` 是客户端 at-rest 信封，密钥只存在于客户端进程内存
> （`~/.workbuddy/keyblob` 的 `static-v1` 槽用 `sha256(secret)[:8]` 作校验）。离线解密需要
> 现场回收进程内存密钥（hub 的做法），本次未实现——扫描会明确提示并给出三条替代路径，
> 且不妨碍命中同目录的明文凭据。

