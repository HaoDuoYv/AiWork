# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 先读 AGENT.md

`AGENT.md`（约 110KB）是本仓库的事实真相源：完整目录地图、Tauri 命令契约（§5）、事件表（§6）、数据文件布局（§7）、切换器/switcher 约定（§8）、前端约定（§10）、升版与发版流程（§11.1）、安全红线（§12）、禁止事项（§13）、含中文源码编辑红线（§13.1）、已知约束（§14）、Git 引用静默丢失坑（§15）、远端同步规范（§16）。**任何会改契约的提交须同步更新 AGENT.md**。本文件只给出上手所需的高层骨架，细节去 AGENT.md 对应小节查，不要另起一份重复说明。

## 常用命令

```bash
npm install
npm run tauri dev        # 开发：scripts/dev-tauri.mjs 在 5000-6000 随机选空闲端口，写 tauri.dev.conf.json 覆盖 devUrl
npm run tauri build      # 打包 msi + nsis 到 src-tauri/target/release/bundle/
npm run build            # 仅前端：tsc && vite build（tsc 同时承担类型检查，无单独 lint 命令）
npm run test             # 前端单测 vitest run
npm run lint             # 前端 ESLint（eslint.config.js，0 error/0 warning 基线）
npx tsc --noEmit         # 类型检查（npm run build 也会跑 tsc）
npx vitest run src/lib/format.test.ts          # 跑单个测试文件
npx vitest run -t "测试名关键字"                 # 按用例名过滤
cd src-tauri && cargo test                     # Rust 单测（测试用例内置于各模块）
cargo test <test_name> -p ai-work-assistant    # 只跑匹配的 Rust 测试
cargo clippy --all-targets -- -D warnings      # Rust lint 门禁（基线见 src-tauri/Cargo.toml [lints.clippy]）
cargo check -p ai-work-assistant               # 快速编译检查（网关大改前建议先跑）
node scripts/rename_release.mjs                # 安装包统一输出到 release/，中文命名
node scripts/package_portable.mjs              # 便携版 zip
npm run set-version <x.y.z>                    # 仅在用户明确要求发版时执行
```

- 前置：仅 Windows；Node 18+、Rust 1.85（MSRV，`Cargo.toml` rust-version）、VS Build Tools C++、WebView2。
- **本机 cargo/rustc 坑**：`~/.cargo/bin` 下的可能是 0 字节坏桩，构建时需把 `C:\Users\Administrator\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin` 前置到 PATH。
- **不要用 `npm run dev` 单独跑 Vite**——必须走 `npm run tauri dev`（见 AGENT.md §13）。
- 升版仅在用户明确说"升级版本"时做；日常提交不动版本号（AGENT.md §11.1）。

## 架构总览

Tauri 2 桌面应用：React 18 前端（`src/`）+ Rust 后端（`src-tauri/src/`）。职责上 Rust 是**全部业务逻辑与 I/O** 的执行方，前端是**状态与展示**层。

### 前端 (`src/`)
- **单一真相**：`store.ts`（Zustand）聚合全部状态，`App.tsx` 启动时调一次 `init()`。签到/切换/保存登录等进度由 Rust 事件驱动，其中 Qoder 签到的事件监听与归约放在 store 层（而非页面），切页不丢事件。
- **唯一 invoke 入口**：`lib/tauri.ts` 的 `api.*`（内部统一 `invoke` 封装，含超时与错误文案翻译）。新增命令须同时在 `src-tauri/src/main.rs` 的 `generate_handler!` 注册、在 `lib/tauri.ts` 加封装、在 `types.ts` 定义 DTO。
- **跨语言契约**：字段名一律 **snake_case**，`types.ts` 与 Rust DTO 严格对齐。Rust 命令的顶层参数名按函数签名原名匹配（不要改 camelCase），嵌套参数结构体字段名改名即断链（前端 invoke 载荷与 `--task-run` CLI 序列化的 JSON 都依赖字段名）。
- **路由**：极简 `useState` 切页，不用 react-router。导航分「应用 Tab」（Trae / Buddy / Qoder / 豆包 …）与不随 Tab 切换的全局页（`agent-hub` 智能体接入、`hub-center` 枢纽增强）。
- **样式**：Tailwind 3 + `darkMode:'class'`，6 套主题由 `data-theme` 驱动；`lib/themes.ts` 与 `index.css` 覆盖块必须两处同步新增。
- 弹窗用自研 `Modal`（不支持 `window.confirm()`）；异步按钮用 `withMinDelay(promise, 1000)`。

### 后端 (`src-tauri/src/`)
按「领域」而非「技术层」组织的大模块：

| 模块 | 职责 |
|---|---|
| `api_server/` | 内嵌 API 网关（axum + ureq）：OpenAI `/v1/chat/completions`、Anthropic `/v1/messages`、Codex `/v1/responses` 三协议，四池调度（trae / buddy / qoder / custom），SSE 双向转换，模型目录、计价、上游池熔断、`ck_` 子 Key；轻量端点（health/healthz/usage）在 `routes_light.rs`，协议大块在 `routes.rs` |
| `commands/` | `#[tauri::command]` 层（薄）；`workbuddy/`、`qoder/` 为按应用拆分的目录模块；`hub.rs`、`agent_import.rs` 为枢纽增强入口 |
| `tasks/` | 后台业务直调（签到/保活/额度巡检/调度器），不经 Tauri 命令；`--task-run <name>` CLI 模式供 schtasks 直调主 exe |
| `switcher/` | 登录态切换器：各应用快照布局档案（profile.rs）、进程三级关闭、设备标识重置、快照复制与单代回滚 |
| `device_proxy/` | 自建 MITM 代理：CA、头改写、凭据捕获、WS/SSE 记录、上游隧道 |
| `store/` | SQLite 存储层（WAL，`data/aiwork.sqlite`），统一经 `store::db(data_dir)` 串行访问；启动迁移器把旧 JSON 导入 `data/backup/` |
| `state.rs` / `models.rs` | `AppState`（数据目录固定在 `%APPDATA%\AIWorkAssistant`）与 DTO |
| `vault.rs` / `jwt.rs` / `fs_utils.rs` | 凭证保险库（DPAPI）、JWT 解析刷新、原子 JSON 读写与路径安全 |

### 网关请求链路（改网关前必读）
`docs/gateway-dispatch.md` 是排障与调参的权威说明。要点：模型命中自定义模型 → custom 池；否则查统一目录（trae + buddy + qoder 双区 + 自定义）并按 `wb_model_route` 的别名/系列/后缀规则映射到池；池内 `pool.rs` 做 过滤（禁用/冷却/积分耗尽）→ 排序 → 会话粘性 → 试跑 → 失败轮换；响应侧按协议走 `sse.rs` / `wb_sse.rs`。各池在 `api_pool` 配置中有独立开关与调度参数（`trae_enabled` / `wb_enabled` / qoder / custom）。

### 数据与运行模型
- 所有数据只在本地 `%APPDATA%\AIWorkAssistant\`（`conf/` 配置与 vault，`data/` SQLite + 快照目录 + 证书 + 导出，`logs/`）。
- AGENT.md 中提到的 JSON 状态文件名均为**逻辑名**（SQLite kv 键）；UI 配置、vault、日志、导出仍走 `fs_utils::write_json` 原子写。
- 桌面应用外的变体：`macos_main`、`docker_main` 分支是独立产品线，不要在本分支上混入其改动。

## 本仓库特有的操作红线

1. **改含中文的源码文件只用 Edit/Write 工具**，或先写 Node 脚本（UTF-8 读写）批量替换。**禁止** PowerShell `Get-Content`/`Set-Content`/`WriteAllText`——曾两次吞掉数百处换行并损坏中文（AGENT.md §13.1）。改完核对 `\uFFFD` 计数为 0、行数合理；损坏先 `git checkout -- <file>`，不要尝试逆向修编码。
2. **删除文件用 Python `os.remove`，不用裸 `rm`**（本机 bash `rm` shim 曾损坏）。
3. **git**：提交前 `git fetch --all --tags --prune` 并处理落后；嵌套分支提交后立即比对 `git rev-parse HEAD` 与分支 ref（本环境存在 ref 静默回滚坑，修复只能原位改 `packed-refs`）；不跑 `git gc`/`git prune`/`git pack-refs`；推送被拒不用 `--force`（AGENT.md §15、§16）。
4. **凭证**：列表接口对 jwt / session 一律掩码下发；完整值走专用 `*_get_*` 命令按需获取；user_id 作路径段必须过 `ensure_uid_safe`。凭证不出 Rust 进程、不入日志。
5. **依赖最小化**：不引入 React Router / Redux / 额外 UI 库；Rust 侧新增依赖注意 MSRV 1.85（例如 sysinfo 锁定 0.33）。
6. **lint 通过才提交**：`cargo clippy --all-targets -- -D warnings`、`npm run lint`、`npx tsc --noEmit`、两侧测试必须全绿（CI `.github/workflows/ci.yml` 按此阻断）。不要用 `#[allow(...)]` 或 `eslint-disable` 压新警告——确属误报或有意设计时，在 `src-tauri/Cargo.toml [lints.clippy]` / `eslint.config.js` 写理由并就地注释。
7. **已知未接线**：`tasks::qoder_realm::resolve_pool_tag` 与 `api_server::api_keys::pool_realm_of` 已标 `#[allow(dead_code)]`——Qoder 子 Key 绑区（`bind_pool=qoder_intl`）当前**不生效**，定区只认模型前缀。不要误当作可用能力，接线见 `docs/backlog.md` F-83。

## 文档索引

- `AGENT.md` — 项目速查手册（事实真相源，改契约须同步）
- `docs/gateway-dispatch.md` — 网关调度与排障
- `docs/tech-framework.md` / `docs/product-design.md` / `docs/user-manual.md` — 技术框架 / 产品设计 / 用户手册
- `docs/merge-plan.md` — v4.0.0 三项目合并方案与来源映射
- `docs/backlog.md` — 唯一待办来源；`docs/pending/`、`docs/completed/`、`docs/archived/` 为方案归档
- `CHANGELOG.md` — 版本变更记录（发版时手动新增条目）
