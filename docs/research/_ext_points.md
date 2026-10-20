# AI Work 助手 v3.7.13 扩展点手册

> 目标：在 `TraeWorkAssistant/`（Tauri 2 + React 18 + TS + Rust）里**正确新增一个一级页面（前端）**与**一组 tauri command（Rust）**，并复用现有数据层 / 设置层 / 网关层。
> 全部结论均来自只读代码审阅，行号对应当前工作区版本。代码片段为原文摘录（缩进保留）。
> 凡标 **⚠** 的是「照抄前人踩坑」条款。

---

## 0. 新增功能的总清单（8 处改动，缺一即断链）

| # | 文件 | 改什么 | 详见 |
|---|---|---|---|
| 1 | `src/types.ts` | `ViewKey` 联合类型加成员（+ 新 DTO 接口） | §A1 |
| 2 | `src/components/Sidebar.tsx` | 对应平台的 `NAV`/`BUDDY_NAV`/`QODER_NAV`/`DOUBAO_NAV` 数组加一条菜单项 | §A1 |
| 3 | `src/App.tsx` | `import` 新页面 + `switch (view)` 加 `case` | §A1 |
| 4 | `src/lib/tauri.ts` | `export const api = { ... }` 里加命令封装（invoke） | §A2 |
| 5 | `src-tauri/src/commands/mod.rs` | `pub mod <新模块>;` | §B9 |
| 6 | `src-tauri/src/main.rs` | `mod commands;` 已存在 → 只需在 `generate_handler![...]` 内追加 `commands::<模块>::<命令>` | §B8 |
| 7 | `src-tauri/src/commands/<模块>.rs` | 新命令实现（`#[tauri::command]`） | §D 模板 |
| 8 | （可选）`src/store.ts` | 需要跨页面共享的状态 / 后台刷新入口 | §A4 |

无需改 `tsconfig.json`、`vite.config.ts`、路由表（项目**没有 react-router**，见 `AGENT.md:352`「路由：极简 useState，不引 react-router」）。

---

# A. 前端

## A1. 页面如何注册到导航

### 1) 视图 key 的单一真源：`src/types.ts:4`

```ts
export type ViewKey =
  | 'dashboard'
  | 'accounts'
  | 'checkin'
  | 'credits'
  | 'logs'
  | 'api-service'
  | 'settings'
  // 豆包应用页面（侧边栏应用切换 Tab → 豆包）
  | 'doubao-overview'
  ...
```

`src/types.ts:31-40`：

```ts
/** 侧边栏应用切换（左下角 Tab）：Trae 当前菜单 / Buddy / Qoder（F-80）/ 豆包 */
export type AppKey = 'trae' | 'buddy' | 'qoder' | 'doubao';

/** 各应用的默认落地页 */
export const APP_HOME_VIEW: Record<AppKey, ViewKey> = {
  trae: 'dashboard',
  buddy: 'buddy-overview',
  qoder: 'qoder-overview',
  doubao: 'doubao-overview',
};
```

### 2) 菜单项结构：`src/components/Sidebar.tsx:28`

字段名固定三件套 `key / label / icon`，`icon` 是 **lucide-react 组件本身**（不是字符串），类型标注 `typeof Users`：

```ts
const NAV: { key: ViewKey; label: string; icon: typeof Users }[] = [
  { key: 'dashboard', label: '概览', icon: LayoutDashboard },
  { key: 'accounts', label: '账号管理', icon: Users },
  { key: 'checkin', label: '一键签到', icon: PlayCircle },
  { key: 'credits', label: '积分看板', icon: Coins },
  { key: 'api-service', label: '资源调度', icon: Server },
  { key: 'settings', label: '环境配置', icon: Settings },
];
```

**平台分组方式**：每个应用一份独立数组 + 三元选择（`Sidebar.tsx:38-62` 定义 `DOUBAO_NAV` / `BUDDY_NAV` / `QODER_NAV`），第 `108-115` 行按 `activeApp` 选中：

```ts
  const nav =
    activeApp === 'doubao'
      ? DOUBAO_NAV
      : activeApp === 'buddy'
      ? BUDDY_NAV
      : activeApp === 'qoder'
      ? QODER_NAV
      : NAV;
```

渲染（`Sidebar.tsx:133-151`）——图标取用方式 `const Icon = item.icon; ... <Icon size={17} />`：

```tsx
        {nav.map((item) => {
          const Icon = item.icon;
          const active = view === item.key;
          return (
            <button
              key={item.key}
              onClick={() => onNav(item.key)}
              ...
            >
              <Icon size={17} />
              {item.label}
            </button>
```

> 应用切换 Tab（左下角）由 `APP_TABS`（`Sidebar.tsx:65-70`）驱动，`AppKey` 驱动；新增**一级页面**不要动它，新增**整应用**才动它。

### 3) 页面切换逻辑：`src/App.tsx:33`

```tsx
function renderView(view: string) {
  switch (view) {
    case 'dashboard':
      return <Dashboard />;
    ...
    case 'api-service':
      return <ApiService />;
    case 'settings':
      return <Settings />;
    ...
    default:
      return <Dashboard />;
  }
}
```

挂载点 `App.tsx:123-131`（`Sidebar` 受控、`renderView(view)` 落进滚动容器）：

```tsx
      <div className="flex min-h-0 flex-1">
        <Sidebar view={view} onNav={setView} />
        <main className="relative flex min-w-0 flex-1 flex-col">
          <TopBar />
          <div className="relative min-h-0 flex-1 overflow-auto p-5">{renderView(view)}</div>
        </main>
      </div>
```

`view` 的来源是 zustand：`App.tsx:87` `const view = useAppStore((s) => s.view);`，切换器 `src/store.ts:499`：

```ts
  setView: (v) => set({ view: v }),
  setActiveApp: (app) => set({ activeApp: app, view: APP_HOME_VIEW[app] }),
```

> **⚠ 需要「同组件多视图」时给 `key`**：`App.tsx:42-44` 的注释说明，共用同一组件类型而不给 `key`，React 会复用实例、只更新 props，挂载期 `useEffect` 不重跑 → 数据不刷新。
> ```tsx
>       return <CreditsDashboard key="trae" platform="trae" />;
> ```

---

## A2. `src/lib/tauri.ts`：invoke 封装与事件订阅

### 封装主体（全库唯一 invoke 入口）：`tauri.ts:134`

```ts
/** 全库唯一 invoke 入口：api.* 各命令均经此发出（titlebar 窗口控制等非后端命令除外） */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const timeout = Object.prototype.hasOwnProperty.call(INVOKE_TIMEOUT_OVERRIDES, cmd)
    ? INVOKE_TIMEOUT_OVERRIDES[cmd]
    : INVOKE_TIMEOUT_MS;
  try {
    if (timeout === null) return await tauriInvoke<T>(cmd, args);
    ...
  } catch (err) {
    throw translateInvokeError(err, cmd);
  }
}
```

配套：`INVOKE_TIMEOUT_MS = 120_000`（`tauri.ts:104`）、长耗时命令豁免表 `INVOKE_TIMEOUT_OVERRIDES`（`tauri.ts:106-119`，值 `null` = 不设超时）、错误翻译 `translateInvokeError`（`tauri.ts:123-131`，保证 `String(err)` 无 `Error:` 前缀）。

`export const api = {` 在 `tauri.ts:161`，按域分组（`env / cert / proxy / accounts / groups / tags / checkin / misc / profiles / oauth / apiServer / ...`）。

### 真实例子 1：无参（`tauri.ts:677-680`）

```ts
  apiServer: {
    start: () => invoke<ApiServiceStatus>('api_server_start'),
    stop: () => invoke('api_server_stop'),
    status: () => invoke<ApiServiceStatus>('api_server_status'),
```

### 真实例子 2：带可选参数（`tauri.ts:831-832`）

```ts
    // T1：近 N 天 API 用量统计（Trae 模型请求桶；按日聚合，服务未运行也可查）
    usageStats: (days?: number) =>
      invoke<UsageDayView[]>('api_usage_stats', { days: days ?? null }),
```

可选值统一 `?? null`（Rust 侧 `Option<T>` 收 `null`）；嵌套对象字段 **snake_case**（`tauri.ts:287-299`）：

```ts
    logsQuery: (opts: {
      logType?: string;
      date?: string;
      keyword?: string;
      limit?: number;
    }) =>
      invoke<LogLine[]>('logs_query', {
        opts: {
          log_type: opts.logType,
          ...
```

命名规则的**权威注释**在 `tauri.ts:852`：

```ts
    // 顶层参数用 camelCase（availableOnly），嵌套结构体字段保持 snake_case
    unifiedModels: (availableOnly?: boolean) =>
      invoke<UnifiedModel[]>('api_unified_models', { availableOnly: availableOnly ?? null }),
```

### 事件订阅（单个事件）：`tauri.ts:673-675`

```ts
    // 登录完成事件：回环监听器自动落库后发出；ok=false 时提示改用手动粘贴兜底
    onLoginDone: (cb: (e: OAuthLoginDoneEvent) => void): Promise<UnlistenFn> =>
      listen<OAuthLoginDoneEvent>('oauth-login-done', (ev) => cb(ev.payload)),
```

### 事件订阅（批量注册 + 返回卸载函数数组）：`tauri.ts:1005-1013`

```ts
export async function setupListeners(
  handlers: ListenerHandlers,
): Promise<UnlistenFn[]> {
  const unsubs: UnlistenFn[] = [];
  if (handlers.onProxyLog) {
    unsubs.push(
      await listen<string>('proxy-log', (e) => handlers.onProxyLog!(e.payload)),
    );
  }
  ...
  return unsubs;
}
```

订阅者只在 `store.init()` 里调一次（见 §A4），一个事件名对应 `AGENT.md:249-267` 的事件表；新增事件必须同步该表。

---

## A3. DTO 命名 / 序列化约定

**结论：全局 snake_case。Rust 侧基本不写 `rename_all`。**

`src/types.ts:1-2`（文件头契约）：

```ts
// 与 Rust 端 DTO 对齐的类型定义。注意：Tauri 命令参数默认使用 snake_case，
// 嵌套对象（CheckinOpts / LogsOpts / Settings）的字段必须保持 snake_case。
```

`src/types.ts:350`：`// snake_case 必须与 Rust Settings 完全一致`。

Rust 侧结构体例子（`src-tauri/src/api_server/custom_models.rs:22-54`）——**零 `rename_all`**，字段直接 `pub base_url: String`：

```rust
/// 单条自定义模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomModel {
    /// 稳定 id（cm-<12hex>，upsert 时空值自动生成）
    #[serde(default)]
    pub id: String,
    ...
    #[serde(default)]
    pub base_url: String,
```

前端对应（`src/types.ts:645-663`）字段逐一同名：`id / name / base_url / api_key / vendor / enabled / context_length / max_tokens / supports_image / rate / note / updated_at`。

`rename_all` 在本仓库**只有 4 处**，且前两处是**已存在的不一致（别照抄）**：

| 位置 | 用法 | 实况 |
|---|---|---|
| `src-tauri/src/commands/misc.rs:82-84` | `#[serde(rename_all = "camelCase")] pub struct ProxyLogQueryOpts { start_time, end_time, ... }` | 前端 `tauri.ts:324-331` 仍发 `start_time/end_time` → 按 camelCase 反序列化时期望 `startTime/endTime`，**这两个字段实际被静默丢弃成 `None`**（单字段 `keyword/offset/limit` 不受影响） |
| `src-tauri/src/commands/api_server.rs:1322-1332` | 同上，`ApiLogSearchOpts` | 前端 `tauri.ts:798-805` 同样发 `start_time/end_time` → 同样被丢弃；只有 `date/keyword` 生效 |
| `src-tauri/src/commands/ccswitch.rs:27` | `#[serde(rename_all = "camelCase")]` | 返回结构体，前端 `CcSwitchStatus` 按 camelCase 声明，属**正常工作**的特例 |
| `src-tauri/src/api_server/dispatch.rs:85` | `#[serde(rename_all = "lowercase")]` | 枚举值小写（`strategy` 等），正常工作 |

**结论：新代码不要写 `rename_all`。** 只有「前端已经按 camelCase 定义好了整型接口」时才用，且必须两端一起改。

**红线**（`src-tauri/src/commands/doubao.rs:6`）：

```rust
//!   严禁 rename_all = "camelCase"（曾导致导入预览弹框前端必崩）。
```

以及 `AGENT.md:430-431`：

```
- ❌ 修改 Rust 命令嵌套参数（如 `CheckinOpts`）的字段名 → 前端 invoke 载荷与 `--task-run` CLI 序列化均按字段名匹配，改名即断链。
- ❌ 把 Rust 命令顶层参数改为 camelCase → Tauri 用 Rust 函数签名原名匹配。
```

**实操结论（本项目实测行为）**：
- **顶层命令参数**：Rust `snake_case` ↔ JS **camelCase**。证据：`commands/accounts.rs:612` `group_id: Option<String>` ↔ `tauri.ts:191` `{ name, jwt, groupId }`。
- **嵌套结构体 / DTO 字段**：两侧都是 `snake_case`。
- 可选字段一律 `#[serde(default)]`（`models.rs` 里 150+ 处），新增字段必须带 `default`，否则老数据反序列化即炸。

---

## A4. `src/store.ts` 的组织方式

**单一 store（不是 slice 拼装）**，`zustand` v4 `create`，字段 + 方法全部平铺在同一个 `AppState` 里。

`src/store.ts:71-111`（状态接口）＋ `111-181`（方法签名）：

```ts
interface AppState {
  ready: boolean;
  view: ViewKey;
  /** 侧边栏应用切换（trae = 现有菜单；buddy = 后期扩展置灰；doubao = 豆包页） */
  activeApp: AppKey;
  env: EnvStatus | null;
  ...
  init: () => Promise<void>;
  setView: (v: ViewKey) => void;
  /** 切换侧边栏应用 Tab，并跳到该应用默认首页 */
  setActiveApp: (app: AppKey) => void;
  ...
  refreshEnv: () => Promise<void>;
  ...
  saveSettings: (patch: Partial<Settings>) => Promise<void>;
  pushToast: (kind: ToastKind, msg: string) => void;
```

创建与初值：`store.ts:313` `export const useAppStore = create<AppState>((set, get) => ({`

### 初始化流程：`store.ts:346-353`

```ts
  init: async () => {
    // StrictMode 下 effect 会执行两次（dev）：两次 init 同步并发启动，旧的「先注销旧监听」
    // 防护在 setupListeners resolve 前执行时注销不到任何东西，两组监听都会注册成功，
    // 且后者覆盖 unsubs → 第一组永久泄漏、事件双发（proxy-log 重复、captured +2）。
    // 幂等锁：仅首次执行注册，后续调用直接复用（审查修复 P1-16）
    if (initStarted) return;
    initStarted = true;
    unsubs = await setupListeners({
```

由 `App.tsx:93-97` 触发一次：

```tsx
  useEffect(() => {
    void init().catch((err) => {
      console.error('初始化失败:', err);
    });
  }, [init]);
```

### 如何触发后台刷新

两条现成路子，任选其一（新页面**优先用第二条**，别往 store 塞页面私有状态）：

1. **全局刷新函数**：`store.ts:700-775` 的 `refreshEnv / refreshCert / refreshProxy / refreshApiStatus / refreshAccounts / refreshGroups / refreshSettings / ...`，页面里 `const refreshAccounts = useAppStore((s) => s.refreshAccounts); await refreshAccounts();`
2. **事件驱动**：`init()` 内 `setupListeners({...})` 的回调里 `set(...)` + `void get().refreshAccounts()`（`store.ts:356-361`）：

```ts
      onAccountCaptured: (uid) => {
        // 事件驱动累加捕获数（后端 Arc<AtomicI64> 的实时镜像，避免轮询）
        set((s) => ({ proxy: { ...s.proxy, captured: s.proxy.captured + 1 } }));
        get().pushToast('success', `已捕获账号 ${uid}`);
        void get().refreshAccounts();
      },
```

后台/跨页面级的进行态放 store（`store.ts:54-62` 注释即为范例：Qoder 签到进行态 store 化，页面卸载不丢事件）。

---

## A5. 设置面板的持久化模式

**链路：组件本地 `form` → `store.saveSettings(patch)` → `api.misc.settingsSet(patch)` → 命令 `settings_set` → SQLite `kv` 表键 `app_settings`。**

### 前端 A：`src/components/GeneralSettingsPanel.tsx`

拉取（`GeneralSettingsPanel.tsx:44-50`）＋用 store 覆写本地 form（`67-73`）：

```tsx
  useEffect(() => {
    void refreshSettings();
    api.misc
      .autostartStatus()
      .then(setAutostart)
      .catch(() => setAutostart(false));
  }, [refreshSettings]);
```

```tsx
  useEffect(() => {
    if (settings && !form) {
      // 兼容旧主题值：light→graphite、dark→charcoal
      const theme = settings.theme === 'light' ? 'graphite' : settings.theme === 'dark' ? 'charcoal' : settings.theme;
      setForm({ ...settings, theme });
    }
  }, [settings, form]);
```

脏检查 + 提交（`75`、`113-124`）：

```tsx
  const dirty = form != null && settings != null && JSON.stringify(form) !== JSON.stringify(settings);
```

```tsx
  const save = async () => {
    if (!form) return;
    setSaving(true);
    try {
      await withMinDelay(saveSettings(form));
      toast('success', '设置已保存');
    } catch {
      /* toast 已发出 */
    } finally {
      setSaving(false);
    }
  };
```

### 前端 B：`src/pages/Settings.tsx`（额外维护 baseline，避免外部通道写入被回滚）

`Settings.tsx:33-42`、`45-58`、`66-78`：

```tsx
  // 本地表单状态：用户编辑后点击「保存」才持久化，避免每次按键都写文件
  const [form, setForm] = useState<SettingsType | null>(null);
  // form 与 store settings 的同步基线：form 等于基线（用户未编辑）时跟随外部变化回填，
  // 防止挂载时旧快照把系统设置弹框等外部通道刚保存的值覆盖回滚
  const baselineRef = useRef<SettingsType | null>(null);
  ...
  useEffect(() => {
    void refreshSettings();
    void query();
  }, [refreshSettings]);
```

```tsx
  const save = async () => {
    if (!form) return;
    setSaving(true);
    try {
      await withMinDelay(saveSettings(form));
      baselineRef.current = form; // 保存成功后以提交内容为新基线，恢复对外部变化的跟随
      toast('success', '配置已保存');
    } catch {
      /* toast 已发出 */
    } finally {
      setSaving(false);
    }
  };
```

### 中枢：`src/store.ts:1066-1089`

```ts
  saveSettings: async (patch) => {
    // F3：settings_get 失败时 store.settings 只是默认值兜底，此时保存会把默认值
    // 整表写回、覆盖服务端真实配置（数据丢失）——守卫拒绝写入并提示
    if (!get().settingsLoaded) {
      const msg = '设置读取失败，为避免用默认值覆盖真实配置已拒绝保存；请重启应用恢复后重试';
      get().pushToast('error', msg);
      // 以字符串抛出：调用方 String(err) 展示无 "Error: " 前缀
      throw msg;
    }
    const current = get().settings ?? defaultSettings();
    const next = { ...current, ...patch } as Settings;
    set({ settings: next });
    try {
      // 后端 settings_set 为真 patch 语义（仅写出现的字段）——只发 patch，
      // 不发合并后的全量对象：store.settings 可能落后于其他通道（如系统设置弹框）刚保存的值，
      // 全量发送会把那些字段回滚成旧值
      await api.misc.settingsSet(patch);
    } catch (err) {
      set({ settings: current });
```

读取函数 `refreshSettings`（`store.ts:748-775`，`api.misc.settingsGet()` + 脏值归一 + `settingsLoaded` 标记）。

### Rust 端：命令名与存储键

`src-tauri/src/commands/misc.rs:760-763`：

```rust
#[tauri::command]
pub fn settings_get(state: State<AppState>) -> Settings {
    state.settings()
}
```

`src-tauri/src/commands/misc.rs:806-833`（**patch 合并 + 键名 `app_settings`**）：

```rust
#[tauri::command]
pub fn settings_set(state: State<AppState>, patch: serde_json::Value) -> Result<(), String> {
    // SQLite 化（P2）：app_settings 入 kv 文档（patch 合并语义不变）
    let store = crate::store::db(&state.data_dir);
    let mut current: serde_json::Value = store.kv_get("app_settings");
    // 内容为 null 时初始化为空对象，避免 patch 被丢弃
    if !current.is_object() {
        current = serde_json::json!({});
    }
    if let (Some(current_obj), Some(patch_obj)) =
        (current.as_object_mut(), patch.as_object())
    {
        for (k, v) in patch_obj {
            ...
            current_obj.insert(k.clone(), v.clone());
        }
    }
    store.kv_set("app_settings", &current)
}
```

`state.settings()`（`src-tauri/src/state.rs:234-237`）直接读同一个键并回填默认值：

```rust
    pub fn settings(&self) -> Settings {
        // SQLite 化（docs/sqllite-storage-plan.md P2）：kv 文档读取，
        // 缺失/为空/解析失败均回退默认（与原 fs_utils::read_json 语义一致）
        let mut s: Settings = crate::store::db(&self.data_dir).kv_get("app_settings");
```

字段定义在 `src-tauri/src/models.rs:146-147`（`#[derive(Serialize, Deserialize, Default)] pub struct Settings`），每个字段都带 `#[serde(default = "...")]`，例：`state.rs` 同款 `pinned_app` 在 `models.rs:349-353`：

```rust
    #[serde(default = "default_pinned_app")]
    pub pinned_app: String,
    #[serde(default)]
    pub hidden_apps: Vec<String>,
```

> **新增一个设置项的完整动作**：① `models.rs::Settings` 加字段（带 `#[serde(default...)]`）→ ② `types.ts::Settings` 加同名 `snake_case` 字段 → ③ 面板 `update('新字段', v)` → `saveSettings({ 新字段: v })`（**只发 patch**）。无需迁移脚本（`kv_get` 缺失字段走 `default`）。

---

## A6. UI 组件库用法

`src/components/ui.tsx` **只导出 6 个组件**（没有 Button/Card/Input，那些是 `index.css` 的原子类）：

| 组件 | 行号 | props 签名 |
|---|---|---|
| `Badge` | `ui.tsx:44-54` | `{ children: ReactNode; tone?: Tone; className?: string; title?: string }` |
| `Spinner` | `ui.tsx:62` | `{ className?: string }` |
| `Progress` | `ui.tsx:75-83` | `{ value: number; max?: number; className?: string }` |
| `Modal` | `ui.tsx:95-115` | `{ open: boolean; onClose: () => void; title: string; children: ReactNode; footer?: ReactNode; size?: 'lg' \| 'xl' \| '2xl'; widthClass?: string; bodyClass?: string }` |
| `EmptyState` | `ui.tsx:147-155` | `{ icon?: ReactNode; title: string; hint?: string }` |
| `StatCard` | `ui.tsx:169-179` | `{ label: string; value: ReactNode; hint?: string; tone?: Tone }` |

`Tone` 枚举（`ui.tsx:5-12`）：`'slate' | 'brand' | 'green' | 'amber' | 'red' | 'blue' | 'violet'`。

`Modal` 原文（`ui.tsx:95-133`，注意 ESC + 背景点击均走 `onClose`，`open=false` 直接 `return null`）：

```tsx
export function Modal({
  open, onClose, title, children, footer, size, widthClass, bodyClass,
}: { ... }) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    ...
  }, [open, onClose]);

  if (!open) return null;
```

`StatCard` 原文（`ui.tsx:169-190`）：

```tsx
export function StatCard({ label, value, hint, tone = 'slate' }: {...}) {
  return (
    <div className="card relative overflow-hidden p-4">
      <div className={cn('absolute inset-x-0 top-0 h-0.5', accentMap[tone])} />
      ...
```

### 样式原子类（`src/index.css:126-157`，`@layer components`）

`.card` / `.btn` / `.btn-primary` / `.btn-ghost` / `.btn-outline` / `.btn-danger` / `.input` / `.label` / `.chip` / `.card-hover` / `.row-hover`。

### 页面标题：`src/components/PageHeader.tsx:3-14`

```tsx
export default function PageHeader({ title, desc, actions, leftExtra }: {
  title: string; desc?: string; actions?: ReactNode;
  /** 左侧区域附加元素（紧跟在标题/描述文字后面） */
  leftExtra?: ReactNode;
}) {
```

### Toaster 如何调用

Toaster 只是一个渲染器，**业务侧永远只调 `pushToast`**：

`src/components/Toaster.tsx:21-23`（挂载在 `App.tsx:133` `<Toaster />`）：

```tsx
export default function Toaster() {
  const toasts = useAppStore((s) => s.toasts);
  const dismiss = useAppStore((s) => s.dismissToast);
```

调用方（`ApiService.tsx:44` / `GeneralSettingsPanel.tsx:20`）：

```tsx
  const toast = useAppStore((s) => s.pushToast);
  ...
      toast('error', `同步官网模型失败：${String(err).slice(0, 120)}`);
```

实现（`store.ts:1157-1168`，`kind` ∈ `'info'|'success'|'error'|'warn'`，error 驻留 10s、其余 4.2s）：

```ts
  pushToast: (kind, msg) => {
    // 通知方式兼容：仅 'none'/'system' 有特殊语义，其余值（含历史脏数据如中文标签）
    // 一律按应用内 toast 处理，避免非枚举值把提示静默吞掉（表现为「点了没任何反应」）
    const mode = get().settings?.notify ?? 'toast';
    if (mode !== 'none' && mode !== 'system') {
      ...
      setTimeout(() => get().dismissToast(id), kind === 'error' ? 10_000 : 4200);
```

---

## A7. `src/pages/ApiService.tsx` 的页面骨架

### ⚠ 重要事实：这一页**已经没有 Tab 了**

`ApiService.tsx:21-28` 的页头注释即为契约：

```tsx
/**
 * Trae · 资源调度（unified-api-gateway-design §6.1，Phase 3）
 * 页内仅保留本应用资源级内容（自上而下）：积分体系说明 / 池指标行 / 账号池选择 / 模型目录（Trae）。
 * 模型目录数据源为统一目录聚合（api.apiServer.unifiedModels 过滤 Trae 源，§3.2 四层兜底），
 * 行内「编辑」弹框维护 L1 人工覆盖层（trae_model_meta_set/clear），保存后聚合视图即时生效。
 * 网关级功能（启停 / 接口配置 / API Keys / 生态接入 / 用量统计）已全部迁至
 * 全局 API 管理弹窗（左侧栏 KeyRound 图标），页内不再出现网关级内容。
 */
```

页面骨架 = `PageHeader` + 若干 `<div className="mt-5 grid ...">` 区块 + 若干 `Modal` 子弹框：

- 组件入口 / 状态：`ApiService.tsx:41-91`
- 首屏加载：`93-101`（`refreshAccounts / loadPool / refreshStatus / loadGroups / loadTodayUsage / loadModels` 并发 `void`）
- `return` 起点：`420`；`PageHeader`：`422-430`
- 区块 1 积分说明：`432-450`；区块 2 池指标 `StatCard` 三连：`452-472`；区块 3/4 双列卡片：`474`
- 子弹框（**页面内子面板统一用 `Modal` 叠层**）：`1002-1033`（模型元数据编辑）、`1123`（清除冷却确认）、`1154`（清除 L1 人工值确认）

```tsx
  return (
    <div>
      <PageHeader
        title="Trae · 资源调度"
        desc="服务资源池管理 · 账号调度与模型目录 · 资源提供给API网关使用"
        actions={
          <button className="btn-outline" onClick={() => void refreshStatus()}>
            <RefreshCw size={15} /> 刷新
          </button>
        }
      />
```

```tsx
      <div className="mt-5 grid grid-cols-3 gap-3">
        <StatCard
          label="可用账号数"
          value={availableCount ?? '—'}
          tone="green"
          hint={running ? '健康且未冷却，可被调度选中' : 'API 服务未运行，无实时池状态'}
        />
```

```tsx
      {/* 模型元数据编辑弹框（L1 覆盖层，§6.1）：字段全部可留空 = 交由下层自动来源兜底 */}
      <Modal
        open={editing !== null}
        onClose={() => setEditing(null)}
        title={`编辑模型元数据 · ${editing?.id ?? ''}`}
        footer={
          <>
            ...
            <button className="btn-primary flex items-center gap-1" onClick={() => void saveMeta()} disabled={savingMeta}>
              {savingMeta ? <Spinner className="text-white" /> : <Save size={15} />}
```

### 若要新增一个「Tab」，真正要改的是 `src/components/api/ApiManagerModal.tsx`

`ApiManagerModal.tsx:22-32`：

```tsx
type TabKey = 'overview' | 'config' | 'keys' | 'usage' | 'custom' | 'eco' | 'sanitize';

const TABS: { key: TabKey; label: string }[] = [
  { key: 'overview', label: '概览' },
  { key: 'config', label: '接口配置' },
  { key: 'keys', label: 'API Keys 管理' },
  { key: 'usage', label: '用量统计' },
  { key: 'custom', label: '自定义模型' },
  { key: 'eco', label: '生态接入' },
  { key: 'sanitize', label: '指纹清洗' },
];
```

Tab 栏渲染 `ApiManagerModal.tsx:60-75`，内容分发 `86-98`（**新增 Tab 只需三处：`TabKey` 联合、`TABS` 数组、内容区一行**）：

```tsx
      <div className="mt-3 h-[calc(120vh_-_896px)] min-h-[288px] overflow-y-auto pr-0.5">
        {tab === 'overview' && <ResourceSummary />}
        {tab === 'config' && <InterfaceConfig />}
        {tab === 'keys' && (
          <ApiKeysManager onSubModalChange={(v) => { subOpenRef.current = v; }} />
        )}
        {tab === 'usage' && <UsageStatsPanel />}
        {tab === 'custom' && (
          <CustomModelsPanel onSubModalChange={(v) => { subOpenRef.current = v; }} />
        )}
        {tab === 'eco' && <EcoAccess />}
        {tab === 'sanitize' && <FingerprintSanitizePanel />}
      </div>
```

打开整个弹窗：任意位置 `useAppStore.getState().setShowApiManager(true)`（`store.ts` 的 `showApiManager`，入口按钮见 `Sidebar.tsx:188-195`）。

> **两条路线别混**：页面级「一级页面」走 §A1；网关级「一个 Tab」走本节（`ApiManagerModal`）。

---

# B. Rust 端

## B8. `generate_handler!` 注册位置与命令命名风格

注册位置：`src-tauri/src/main.rs:150` 起，到 `398` 的 `])` 结束。

```rust
        .invoke_handler(tauri::generate_handler![
            commands::env::app_locate,
            commands::env::open_doubao_app,
            ...
            commands::api_server::gateway_settings_get,
            commands::api_server::gateway_settings_set,
```

**新增命令 = 在 `main.rs:398` 之前追加一行 `commands::<模块>::<命令>,`**（逗号别漏，顺序无关）。

命名风格：`<域>_<动作>` 全 snake_case，域前缀与模块名一致（`api_server_*` / `pool_*` / `api_keys_*` / `custom_models_*` / `gateway_settings_*` / `ccswitch_*`）。

三个真实签名：

1) 带两个 `State`、返回值非 Result（`commands/api_server.rs:590-594`）：

```rust
#[tauri::command]
pub fn api_server_status(
    state: State<'_, AppState>,
    runtime: State<'_, Mutex<Option<ApiServerRuntime>>>,
) -> ApiServiceStatus {
```

2) 可选参数（`Option<T>`）+ 直接调数据层（`commands/api_server.rs:1443-1447`）：

```rust
#[tauri::command]
pub fn api_usage_stats(state: State<'_, AppState>, days: Option<u32>) -> Vec<crate::api_server::usage::UsageDayView> {
    let days = days.unwrap_or(14).clamp(1, 90);
    crate::api_server::usage::query_recent(&state.data_dir, days, false)
}
```

3) 写操作，`Result<_, String>`（`commands/api_server.rs:1703-1707`）：

```rust
#[tauri::command]
pub fn gateway_settings_set(
    state: State<'_, AppState>,
    settings: GatewaySettingsPatch,
) -> Result<crate::api_server::gateway_settings::GatewaySettings, String> {
```

另有 4) `#[tauri::command(async)]` 用于阻塞耗时命令（`commands/misc.rs:767-768`）：

```rust
#[tauri::command(async)]
pub fn notify_test(state: State<AppState>) -> Result<String, String> {
```

`State<'_, AppState>` 与 `State<AppState>` 两种写法在库里都在用（前者更常见）；同步命令用 `State<'_, T>` 即可，不要为了 `'static` 到处 clone。

---

## B9. `src-tauri/src/commands/mod.rs` 模块声明方式

全文只有 23 行平铺声明（`commands/mod.rs:1-23`）：

```rust
pub mod accounts;
pub mod api_server;
pub mod cert;
pub mod ccswitch;
pub mod checkin;
pub mod doubao;
pub mod env;
pub mod misc;
pub mod oauth;
pub mod oauth_loopback;
pub mod process;
pub mod profile;
pub mod proxy;
pub mod qoder;
pub mod shared_groups;
pub mod switch;
pub mod tags;
pub mod trae_apps;
pub mod updater;
pub mod usage_history;
pub mod wb_config;
pub mod workbuddy;
pub mod workbuddy_stats;
```

**新增一个命令模块需要改 2 处**：
1. `commands/mod.rs` 追加 `pub mod <name>;`
2. `main.rs:150-398` 的 `generate_handler!` 追加 `commands::<name>::<cmd>,`

若是**目录型模块**（像 `qoder` / `workbuddy`），模式是 `commands/<name>/mod.rs` 内逐文件 `mod xxx;` + `pub use xxx::*;`，命令路径仍是 `commands::<name>::<fn>`（`commands/qoder/mod.rs:20-40`）：

```rust
mod accounts;
pub(crate) mod checkin;
mod cli_status;
...
pub use accounts::*;
pub use checkin::*;
pub use cli_status::*;
```

> 顶层模块 `mod commands;` 已在 `main.rs:4`，无需再动。

---

## B10. `src-tauri/src/state.rs`：`AppState` 与路径

结构体（`state.rs:129-154`）：

```rust
/// 应用全局状态。base_dir 指向 %APPDATA%\AIWorkAssistant；
/// 子目录: conf/ (配置), data/ (数据), logs/ (日志)
/// Clone 支持把状态克隆进后台工作线程（签到/任务等直调场景）。
#[derive(Clone)]
pub struct AppState {
    pub data_dir: PathBuf,
    /// JWT 刷新锁：防止多个并发请求同时 ExchangeToken（Arc 共享跨线程）
    pub jwt_refresh_lock: Arc<Mutex<()>>,
    pub qoder_pool_lock: Arc<Mutex<()>>,
    pub wb_pool_lock: Arc<Mutex<()>>,
    pub doubao_pool_lock: Arc<Mutex<()>>,
    pub group_defs_lock: Arc<Mutex<()>>,
}
```

构造与路径（`state.rs:159-232`）：

```rust
    pub fn new() -> Result<Self, String> {
        let appdata = std::env::var("APPDATA")
            .map(PathBuf::from)
            .map_err(|_| "无法读取 APPDATA 环境变量".to_string())?;
        let data_dir = appdata.join(DATA_DIR_NAME);
        ...
        let conf_dir = data_dir.join("conf");
        let data_subdir = data_dir.join("data");
        let logs_dir = data_dir.join("logs");
```

```rust
    /// 配置文件路径：base_dir/conf/name
    pub fn conf_path(&self, name: &str) -> PathBuf {
        let dir = self.data_dir.join("conf");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    /// 数据文件路径：base_dir/data/name
    pub fn data_path(&self, name: &str) -> PathBuf {
```

- 注册：`main.rs:145` `.manage(state)`（`state` 由 `AppState::new()?` 生成，`main.rs` 上部）。
- 命令里拿：`state: State<'_, AppState>`，用 `&state.data_dir`（**唯一权威数据根**）。
- 其余 `manage` 的运行时单例：`main.rs:145-149`（`MainWindowMaximized`、`Mutex<Option<ProxyHandle>>`、`Mutex<Option<ApiServerRuntime>>`、`CheckinGuard`）。**新增全局可变运行时，照这个模式 `.manage(Mutex::new(...))`，不要往 `AppState` 里塞内部可变状态。**

> **⚠ `data_dir` 里已是 SQLite 一统**：`data/aiwork.sqlite`（`AGENT.md:97`），别再新写 `data/xxx.json`。旧路径 API（`data_path` / `conf_path`）只留给「必须落文件」的产物（日志、导出、证书、快照）。

---

## B11. `src-tauri/src/store/` 提供的持久化 API

三层：`schema.rs`（DDL + 白名单）、`mod.rs`（连接 + 原语）、`docs.rs`（类型化 load/save）。

### 取连接：`store/mod.rs:34`

```rust
/// 取（或创建）data_dir 对应的存储连接。库文件：`<data_dir>/data/aiwork.sqlite`。
pub fn db(data_dir: &Path) -> Arc<Store> {
    let db_path = data_dir.join("data").join("aiwork.sqlite");
```

### KV 文档（key → JSON）——最常用

```rust
    pub fn kv_get_raw(&self, key: &str) -> Option<String>          // mod.rs:108
    pub fn kv_get_raw_checked(&self, key: &str) -> Result<Option<String>, String>  // mod.rs:118
    pub fn kv_set_raw(&self, key: &str, content: &str) -> Result<(), String>       // mod.rs:128
    pub fn kv_delete(&self, key: &str) -> Result<(), String>                        // mod.rs:139
    /// 读取 KV 文档（语义对齐 fs_utils::read_json：缺失/损坏回退 Default）
    pub fn kv_get<T: DeserializeOwned + Default>(&self, key: &str) -> T             // mod.rs:147
    /// 写入 KV 文档（紧凑 JSON）
    pub fn kv_set<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String>  // mod.rs:154
```

原文（`mod.rs:146-157`）：

```rust
    /// 读取 KV 文档（语义对齐 fs_utils::read_json：缺失/损坏回退 Default）
    pub fn kv_get<T: DeserializeOwned + Default>(&self, key: &str) -> T {
        self.kv_get_raw(key)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// 写入 KV 文档（紧凑 JSON）
    pub fn kv_set<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let content = serde_json::to_string(value).map_err(|e| format!("序列化失败: {e}"))?;
        self.kv_set_raw(key, &content)
    }
```

### 行文档（`(pk, data JSON)` 表）

```rust
    pub fn rows_all(&self, table: &str) -> Result<Vec<(String, serde_json::Value)>, String>       // mod.rs:162
    pub fn rows_replace(&self, table: &str, rows: &[(String, serde_json::Value)]) -> Result<(), String>  // mod.rs:184
    pub fn row_upsert(&self, table: &str, pk: &str, data: &serde_json::Value) -> Result<(), String>      // mod.rs:217
```

表名白名单（防注入，`mod.rs:163-165` + `schema.rs:15-32`）：

```rust
    pub fn rows_all(&self, table: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
        if !schema::ROW_TABLES.contains(&table) {
            return Err(format!("非法表名: {table}"));
        }
```

```rust
/// 行文档表白名单（rows_* 原语允许操作的表，防表名拼接注入）
pub const ROW_TABLES: &[&str] = &[
    "device_map", "groups", "tags", "account_tags", "remaining_credits",
    "account_cooldowns", "pay_status", "api_keys", "custom_models",
    "doubao_accounts", "wb_accounts", "wb_tokens", "qoder_accounts",
    "qoder_tokens", "qoder_checkin_results", "qoder_credits_history",
];
```

> **新增行表**：`schema.rs` 加 DDL（`DDL` 数组，`34` 起）+ 加进 `ROW_TABLES` + `SCHEMA_VERSION`（`schema.rs:12`，当前 `4`）+ `migrate.rs` 版本闸门；再在 `docs.rs` 写 `xxx_load/xxx_save`。

### 通用出口（自定义 SQL，非 whitelist 场景）

```rust
    /// 在连接上执行（串行化访问的统一出口）
    pub fn with_conn<T>(&self, f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>) -> Result<T, String>  // mod.rs:98
```

### 类型化文档（`docs.rs`，命名 `<域>_load / <域>_save`）

范例（`store/docs.rs:1323-1345`）—— **`rows_*` + `kv` 组合的写法值得直接照抄**：

```rust
pub fn custom_models_load(s: &Store) -> CustomModelsFile {
    let models = s
        .rows_all("custom_models")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(_, data)| serde_json::from_value(data).ok())
        .collect();
    CustomModelsFile {
        models,
        updated_at: s.kv_get_raw("custom_models_updated_at").and_then(|v| v.parse().ok()).unwrap_or(0),
    }
}

pub fn custom_models_save(s: &Store, f: &CustomModelsFile) -> Result<(), String> {
    let rows: Vec<(String, Value)> = f
        .models
        .iter()
        .map(|m| serde_json::to_value(m).map(|v| (m.id.clone(), v)))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("序列化失败: {e}"))?;
    s.rows_replace("custom_models", &rows)?;
    s.kv_set_raw("custom_models_updated_at", &f.updated_at.to_string())
}
```

### 从 commands 里的真实调用

```rust
// commands/misc.rs:756-757（kv 写布尔）
pub fn traework_migration_guide_dismiss(state: State<AppState>) -> Result<(), String> {
    crate::store::db(&state.data_dir).kv_set("traework_merge_guide_dismissed", &true)
}

// commands/misc.rs:748-751（kv 读 Option<T>）
    if store.kv_get::<bool>("traework_merge_guide_dismissed") {
        return Ok(None);
    }
    store.kv_get::<Option<TraeWorkMigrationReport>>("traework_merge_report")

// state.rs:237（读 Settings 文档）
    let mut s: Settings = crate::store::db(&self.data_dir).kv_get("app_settings");
```

> 类型化「流水表」见 `docs.rs` 里的 `api_usage_upsert_day`（`1537`）、`credits_history_append`（`1715`）、`qoder_session_insert`（`1566`）等；高频统计优先走**列化流水表 + 增量 upsert**，别整表读写。
> `config_cache` 兜住读热路径：`custom_models.rs:91` `super::config_cache::get_or_load(...)` / `111` `invalidate(...)`——**写后必须 invalidate**。

---

## B12. `api_server/gateway_settings.rs`：网关设置

结构体（`gateway_settings.rs:13-31`）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySettings {
    /// 网关监听端口（默认 7864，与既有约定一致）
    #[serde(default = "default_port")]
    pub port: u16,
    /// 网关监听地址（审查 P1-2）：默认 127.0.0.1 环回——个人助手场景默认不对外暴露。
    #[serde(default = "default_host")]
    pub host: String,
    /// 默认模型（请求未指定 model 时使用；统一目录内模型）
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default)]
    pub updated_at: i64,
}
```

load / save（`gateway_settings.rs:71`、`104`）：

```rust
pub fn load(data_dir: &Path) -> GatewaySettings {
    let store = crate::store::db(data_dir);
    if let Some(text) = store.kv_get_raw("api_gateway_settings") {
        if let Ok(s) = serde_json::from_str::<GatewaySettings>(&text) {
            return normalized(s);
        }
    }
    ...
```

```rust
/// 保存网关设置（端口/模型合法性由调用方校验后传入；这里兜底端口范围）
pub fn save(data_dir: &Path, s: GatewaySettings) -> Result<(), String> {
    if s.port == 0 {
        return Err("端口无效（1-65535）".into());
    }
    let mut s = normalized(s);
    ...
    crate::store::db(data_dir).kv_set("api_gateway_settings", &s)
```

命令层（`commands/api_server.rs:1684-1720`）——**`patch` 合并语义（`Option` 字段）是写设置的标准手法**：

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GatewaySettingsPatch {
    pub port: Option<u16>,
    pub host: Option<String>,
    pub default_model: Option<String>,
}

/// 保存网关设置（patch 合并：读现值 → 应用 Some 字段 → 落盘；端口改动在下次
/// 启动 API 服务后生效；返回规范化后的生效值）
#[tauri::command]
pub fn gateway_settings_set(
    state: State<'_, AppState>,
    settings: GatewaySettingsPatch,
) -> Result<crate::api_server::gateway_settings::GatewaySettings, String> {
    let mut s = crate::api_server::gateway_settings::load(&state.data_dir);
    if let Some(p) = settings.port { s.port = p; }
    ...
    crate::api_server::gateway_settings::save(&state.data_dir, s)?;
    Ok(crate::api_server::gateway_settings::load(&state.data_dir))
}
```

前端对应：`tauri.ts:860-863`（`gateway_settings_get / gateway_settings_set`），DTO `types.ts:626-632`。
`kv` 键：**`api_gateway_settings`**（历史 `app_settings` 里的 `api_port/api_default_model` 只做一次性迁移，不再读）。

---

## B13. API Key 体系

### 数据结构（`api_keys.rs:243-292`）

```rust
/// API Key 条目
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ApiKeyEntry {
    /// 唯一标识（前端生成）
    pub id: String,
    /// 展示名
    pub name: String,
    /// 实际 Key 值（ck_xxx / sk-...）
    pub key: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 每日请求配额，0 = 不限
    #[serde(default)]
    pub daily_limit: u64,
    /// 创建时间（unix 秒）
    #[serde(default)]
    pub created_at: u64,
    /// 当日用量记账日期（YYYY-MM-DD）
    #[serde(default)]
    pub used_date: String,
    /// 当日已用请求数
    #[serde(default)]
    pub used_today: u64,
    ...
    #[serde(default)]
    pub allowed_accounts: Vec<String>,
    #[serde(default)]
    pub schedule_mode: String,
    #[serde(default)]
    pub dedicated_account: String,
    #[serde(default)]
    pub bind_pool: String,
    #[serde(default)]
    pub daily_stats: Vec<KeyDailyStat>,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
}
```

根结构（`api_keys.rs:298-306`）：

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ApiKeysFile {
    #[serde(default)]
    pub keys: Vec<ApiKeyEntry>,
    /// 显式关闭鉴权：仅当没有任何启用 Key 时生效（true = 无 Key 放行；默认 false = 无 Key 拒绝）
    #[serde(default)]
    pub auth_disabled: bool,
}
```

存储：**Stronghold vault**（`api_keys.rs:31-33`，命名空间 `api_keys` / 记录键 `registry`），不是 SQLite 明文表。

### 读 / 写 / 原子改 / 鉴权记账签名（`api_keys.rs:505-587`）

```rust
pub fn load(data_dir: &Path) -> ApiKeysFile                                                              // 505
pub fn save(data_dir: &Path, f: &ApiKeysFile)                                                            // 515
pub fn modify<F: FnOnce(&mut ApiKeysFile) -> bool>(data_dir: &Path, f: F) -> bool                        // 541
pub fn verify_and_consume_locked(data_dir: &Path, presented: &str, today: &str) -> KeyCheck              // 563
pub fn flush_dirty(data_dir: &Path) -> bool                                                              // 592
pub fn constraints_for(data_dir: &Path, key_id: &str) -> Option<ResolvedKey>                             // 394
pub fn key_name_for(data_dir: &Path, key_id: &str) -> String                                             // 413
pub fn key_snapshot_for(data_dir: &Path, key_id: &str) -> Option<(String, ResolvedKey)>                   // 429
```

```rust
/// 读（缓存优先；命令层展示与 constraints_for 均走此处，不再每调用一次 SQLite 读）
pub fn load(data_dir: &Path) -> ApiKeysFile {
    let mut reg = KEYS_STATE.lock().unwrap_or_else(|e| e.into_inner());
    entry_or_load(&mut reg, data_dir).file.clone()
}
```

**⚠ 热点：不要 `load → 改 → save` 做增量更新**（会整表覆盖）。用 `modify`（`api_keys.rs:536-541` 注释）：

```rust
/// 原子修改（命令层引用清理路径用）：全程持 KEYS_STATE 锁，对内存权威副本执行
/// f（返回是否变更），变更则标脏并锁外落盘。...
pub fn modify<F: FnOnce(&mut ApiKeysFile) -> bool>(data_dir: &Path, f: F) -> bool {
```

命令层范例：`commands/api_server.rs:1481-1509`（`api_keys_list` / `api_keys_save`，注意开鉴权门禁在 `server.rs::ensure_bind_auth_policy`）。

### `ccswitch.rs` 的 `ensure_ccswitch_key`（整函数原文，`commands/ccswitch.rs:303-356`）

```rust
/// CC Switch 注册专用 Key 名（自动创建/复用；与业务 Key 隔离，不受业务每日配额影响）
const CC_SWITCH_KEY_NAME: &str = "CC Switch 专用";

/// 取「CC Switch 专用」Key：已有同名 Key → 复用（若被禁用则恢复启用）；
/// 无 → 新建（ck_ + 128-bit 随机 hex，不限每日配额）。
/// 持久化随 api_keys::save 落盘（内存权威副本同步刷新）；失败返回 Err 由
/// 注册流程中止——宁可报错也不写占位 Key 导致请求全 401/429。
fn ensure_ccswitch_key(data_dir: &std::path::Path) -> Result<String, String> {
    let mut file = crate::api_server::api_keys::load(data_dir);
    let existing = file
        .keys
        .iter()
        .find(|k| k.name == CC_SWITCH_KEY_NAME)
        .map(|e| (e.enabled, e.key.clone()));
    if let Some((enabled, key)) = existing {
        if !enabled {
            // 被手动禁用过：注册是显式使用意图，恢复启用
            if let Some(e) = file
                .keys
                .iter_mut()
                .find(|k| k.name == CC_SWITCH_KEY_NAME)
            {
                e.enabled = true;
            }
            crate::api_server::api_keys::save(data_dir, &file);
        }
        return Ok(key);
    }
    let entry = crate::api_server::api_keys::ApiKeyEntry {
        id: uuid::Uuid::new_v4().to_string(),
        name: CC_SWITCH_KEY_NAME.to_string(),
        // uuid simple() = 32 位小写 hex，与前端生成的 ck_ 子 Key 格式一致（总长 35）
        key: format!("ck_{}", uuid::Uuid::new_v4().simple()),
        enabled: true,
        daily_limit: 0,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        used_date: String::new(),
        used_today: 0,
        allowed_accounts: Vec::new(),
        schedule_mode: String::new(),
        dedicated_account: String::new(),
        bind_pool: String::new(),
        daily_stats: Vec::new(),
        allowed_groups: Vec::new(),
        allowed_tags: Vec::new(),
    };
    let key = entry.key.clone();
    file.keys.push(entry);
    crate::api_server::api_keys::save(data_dir, &file);
    Ok(key)
}
```

key 格式约定：`ck_` + 32 位小写 hex（总长 35），`id` 用 `uuid::Uuid::new_v4()`；旧 `sk-` 前缀继续兼容（`api_keys.rs:16-17`）。

---

## B14. 自定义模型（OpenAI 兼容上游）

结构体（`custom_models.rs:22-54`），字段与前端 `types.ts:645-663` 一一对应：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomModel {
    pub id: String,            // cm-<12hex>，upsert 空值自动生成
    pub name: String,          // 请求模型名（路由键，canonical = trim+lowercase）
    pub base_url: String,      // OpenAI 兼容 base（可含 /v1）
    pub api_key: String,       // Bearer
    pub vendor: String,
    pub enabled: bool,         // #[serde(default = "default_true")]
    pub context_length: u64,
    pub max_tokens: u64,
    pub supports_image: bool,
    pub rate: f64,             // 展示倍率，0 = 免费
    pub note: String,
    pub updated_at: i64,
}
```

CRUD 签名（全部 `data_dir` 第一参）：

```rust
pub fn load(data_dir: &Path) -> Vec<CustomModel>                                      // custom_models.rs:89
pub fn save_list(data_dir: &Path, models: Vec<CustomModel>) -> Result<(), String>     // :97
pub fn upsert(data_dir: &Path, mut m: CustomModel) -> Result<Vec<CustomModel>, String> // :118
pub fn remove(data_dir: &Path, id: &str) -> Result<bool, String>                       // :160
pub fn find_enabled(data_dir: &Path, model: &str) -> Option<CustomModel>               // :173
pub fn chat_url(base_url: &str) -> String                                              // :185
```

`upsert` 校验与 id 生成（`custom_models.rs:118-157` 摘要）：

```rust
    m.name = m.name.trim().to_string();
    m.base_url = m.base_url.trim().trim_end_matches('/').to_string();
    if m.name.is_empty() {
        return Err("模型名称不能为空".into());
    }
    if m.base_url.is_empty()
        || (!m.base_url.starts_with("http://") && !m.base_url.starts_with("https://"))
    {
        return Err("API 地址必须以 http:// 或 https:// 开头".into());
    }
    ...
            if m.id.is_empty() {
                m.id = format!("cm-{:012x}", std::time::SystemTime::now()...);
            }
```

命令层（`commands/api_server.rs:1786-1807`）：

```rust
#[tauri::command]
pub fn custom_models_list(
    state: State<'_, AppState>,
) -> Vec<crate::api_server::custom_models::CustomModel> {
    crate::api_server::custom_models::load(&state.data_dir)
}

#[tauri::command]
pub fn custom_models_save(
    state: State<'_, AppState>,
    model: crate::api_server::custom_models::CustomModel,
) -> Result<Vec<crate::api_server::custom_models::CustomModel>, String> {
    crate::api_server::custom_models::upsert(&state.data_dir, model)
}

#[tauri::command]
pub fn custom_models_remove(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    crate::api_server::custom_models::remove(&state.data_dir, &id)
}
```

---

## B15. REST 路由注册与 handler 风格

**⚠ 路由不在 `api_server/mod.rs`，而在 `api_server/server.rs`。** `api_server/mod.rs:1-30` 只有 `pub mod ...;` 声明 + 常量（`AGENT.md:69` 写的是旧结构，以代码为准）。

`server.rs:173-193`：

```rust
fn build_router(state: Arc<ApiSharedState>) -> Router {
    Router::new()
        .route("/health", get(routes::health))
        .route("/healthz", get(routes::healthz))
        .route("/status", get(routes::status))
        .route("/v1/models", get(routes::models))
        .route("/v1/chat/completions", post(routes::chat_completions))
        .route("/v1/completions", post(routes::completions))
        .route("/v1/embeddings", post(routes::embeddings))
        .route("/v1/messages", post(routes::messages))
        .route("/v1/responses", post(routes::responses_api))
        // T5.4/F-63 生图双端点投影
        .route("/v1/images/generations", post(routes::images_generations))
        .route("/v1/images/edits", post(routes::images_edits))
        .layer(DefaultBodyLimit::max(routes::MAX_BODY_BYTES))
        .layer(from_fn_with_state(state.clone(), auth::bearer_auth))
        .with_state(state)
}
```

handler 风格（`routes.rs:454`）：

```rust
pub async fn health(State(state): State<Arc<ApiSharedState>>) -> impl IntoResponse {
```

```rust
/// /healthz（T2.3/F-32）：三池均无可用账号 → 503，供探活/看门狗。
pub async fn healthz(State(state): State<Arc<ApiSharedState>>) -> Response {
```

**新增一个 REST 端点 = 3 步**：
1. `routes.rs` 里写 `pub async fn my_ep(State(state): State<Arc<ApiSharedState>>, ...) -> Response`（取 `Json`/`Bytes` 用 axum 提取器，注意 `MAX_BODY_BYTES`）；
2. `server.rs:173` 的 `Router::new()` 链上 `.route("/v1/my", post(routes::my_ep))`；
3. 鉴权默认生效（中间件 `auth::bearer_auth` 全局挂载，`auth.rs:25-30`）。**只有 `/health` 且仅环回监听时豁免**：

```rust
pub async fn bearer_auth(
    State(state): State<Arc<ApiSharedState>>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.uri().path() == "/health" {
        // 环回豁免与 bind 物理同源（2026-10-12 审查 M3）：按实际绑定 host 判定，
        ...
        if super::server::is_loopback_host(&host) {
            return next.run(request).await;
        }
    }
```

鉴权通过后，Key 身份被塞进 extensions（`auth.rs:94-98`），handler 可据此做配额/约束：

```rust
        Some(KeyCheck::Ok(rk)) => {
            let id = rk.id.clone();
            request.extensions_mut().insert(rk);
            request.extensions_mut().insert(KeyId(id));
            return next.run(request).await;
        }
```

> 新增**模型路由/上游**类能力不要直接改 `build_router` 的既有端点，走 `custom_route.rs` / `dispatch.rs` / `wb_model_route.rs` 既有分流点。生图与工具代执行见 `AGENT.md:421-422`。

---

# C. 约束

## C16. `Cargo.toml` 现有依赖（可直接 `use`）

文件：`src-tauri/Cargo.toml`（91 行，`rust-version = "1.85"`，`edition = "2021"`）。

| crate | 版本（`Cargo.toml` 行） | 备注 |
|---|---|---|
| `tauri` | `2`（feat `tray-icon`）L12 | |
| `tauri-plugin-shell/dialog/notification/single-instance/autostart/stronghold` | `2` L13-21 | Stronghold = vault（API Key / JWT 权威存储） |
| **`serde`** | `1`（feat `derive`）L22 | ✅ 已有 |
| **`serde_json`** | `1` L23 | ✅ 已有 |
| **`base64`** | `0.22` L24 | ✅ 已有 |
| `chrono` | `0.4`（feat `serde`）L25 | ✅ 已有 |
| **`ureq`** | `2`（feat `json`）L26 | ✅ HTTP 客户端（**没有 reqwest**） |
| `urlencoding` | `2` L27 | |
| `url` | `2` L30 | SSRF host 判定 |
| `axum` | `0.7` L31 | ✅ 网关 REST |
| `tokio` | `1`（sync/io-util/net/rt/rt-multi-thread/time/macros）L32 | |
| `tokio-stream` | `0.1` L33 | SSE |
| `bytes` | `1` L34 | |
| **`sha2`** | `0.10` L35 | ✅ 已有 |
| `argon2` | `0.5` L38 | 口令派生 |
| **`aes-gcm`** | `0.10` L40 | ✅ 已有 |
| `aes` / `cbc` | `0.8` / `0.1` L43-44 | |
| `md-5` | `0.10` L46 | |
| `num-bigint` | `0.4` L47 | |
| `p256` | `0.13` L48 | ECDSA |
| **`rusqlite`** | `0.32`（feat `bundled`）L50 | ✅ 已有，SQLite 存储层 |
| `hyper` / `http-body-util` / `hyper-util` / `hyper-rustls` / `tokio-rustls` / `tokio-tungstenite` / `hyper-tungstenite` | L53-59 | MITM 代理 |
| `rcgen` | `0.14` L62 | 证书 |
| `time` / `futures-util` / `regex` / `flate2` / `tower-service` / `webpki-roots` | L63-69 | |
| `sysinfo` | `=0.33.1` L72 | **锁定版**（0.38+ 需 rustc 1.88） |
| `uuid` | `1`（feat `v4`）L74 | ✅ 已有（`ensure_ccswitch_key` 用） |
| `if-addrs` | `0.15.0` L75 | |
| windows-only：`windows-sys 0.59` / `lnk 0.6` / `windows-registry 0.6` | L78-82 | |

**没有的（需自己加 `[dependencies]`，或改用字符串拼接）**：

- ❌ **`toml`** —— `Cargo.toml` 未声明。`Cargo.lock` 里的 `toml 0.8/0.9/1.1` 全是 `tauri-build` 等 **build/传递依赖**，`use toml` 编译不过。现状做法：手写 `codex config.toml`，用 `commands/ccswitch.rs:396` 的 `fn toml_escape(s: &str) -> String` 自己拼。
- ❌ `reqwest`（用 `ureq`）、❌ 任何 `yaml` / `serde_yaml`。
- ❌ 任何 `tantivy`/`walkdir`/`anyhow`/`thiserror`（错误统一 `Result<_, String>`，见 §C18）。

## C17. 前端依赖（`package.json`，38 行）

```json
  "dependencies": {
    "@tauri-apps/api": "~2.11.1",
    "@tauri-apps/plugin-dialog": "~2.7.3",
    "@tauri-apps/plugin-notification": "~2.4.0",
    "@tauri-apps/plugin-shell": "~2.3.6",
    "clsx": "^2.1.1",
    "lucide-react": "^0.456.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1",
    "recharts": "^2.13.3",
    "zustand": "^4.5.5"
  },
  "devDependencies": {
    "@tauri-apps/cli": "2.11.4", "@types/react": "^18.3.12", "@types/react-dom": "^18.3.1",
    "@vitejs/plugin-react": "^4.3.4", "autoprefixer": "^10.4.20", "postcss": "^8.4.49",
    "tailwindcss": "^3.4.15", "typescript": "^5.6.3", "vite": "^5.4.11", "vitest": "^2.1.9"
  }
```

- **无 `toml` / `yaml` / `js-yaml` / `ini` 解析库** → 配置解析一律走 **Rust 端**（`serde` + 手写拼装），前端只收发 JSON（`invoke` 本身也是 JSON）。
- 无 UI 库（Ant/MUI/shadcn）、无路由库、无 axios —— `AGENT.md:432` 红线「❌ 引入 React Router / Redux / 额外 UI 库 → 保持依赖最小」。
- 图标只用 `lucide-react`；图表只用 `recharts`；`clsx` 经 `lib/cn.ts` 包装（`cn(...)`）。

## C18. 代码风格注意点

1. **中文注释是硬规范**：每个 `pub fn` 上方一段 `///` 说明「做什么 + 为什么这么做 + 关联 issue/文档编号」，`//` 行内注释解释反直觉处（`store/mod.rs:115-117`、`tauri.ts:102-103` 是范例）。模块头用 `//!`（`api_server/custom_models.rs:1-13`、`store/mod.rs:1-9`）。
2. **错误类型统一 `Result<T, String>`**，中文错误文案，`format!` 拼上下文；`?` 前用 `.map_err(|e| format!("xxx失败: {e}"))?`。前端 `catch` 直接 `String(err)` 展示（`tauri.ts:123-131` 会剥 `Error:` 前缀）。**不引入 `anyhow` / `thiserror`**。
3. **命名**：命令 `域_动作`；DTO 字段 `snake_case`；前端 `api.<域>.<动作>` 驼峰方法名（`tauri.ts:161` 起）；Rust 文件 `snake_case.rs`；TS 组件 `PascalCase.tsx`。
4. **前端异步按钮必须 `withMinDelay`**（`AGENT.md:354`）：`await withMinDelay(saveSettings(form))`（`GeneralSettingsPanel.tsx:117`），最少 1s loading，防连点。
5. **禁 `window.confirm()`**（`AGENT.md:434`，`index.css` 里也没有对应样式）：确认类操作一律 `Modal`（`ApiService.tsx:1123` 清除冷却确认弹框）。
6. **面板表单三件套**：`form` 本地态 + `dirty` 判定（`JSON.stringify` 比较）+ `baselineRef` 跟随外部变化（`Settings.tsx:33-58`）。
7. **`#[serde(default)]` 全覆盖**：任何新字段都必须带，否则老 kv/JSON 反序列化失败。
8. **命令注册后才能被调用**，忘了 `generate_handler!` 的报错是运行期 `command xxx not found`（不是编译期）。
9. **凭证红线**：`accessToken`/`PAT`/`api_key` 不进日志、不进 NDJSON、前端掩码（`commands/qoder/mod.rs:4`）。
10. **提交前**：`cargo test`（`AGENT.md:24`）+ `npm run build`（`tsc && vite build`，会做 TS 类型检查）；模块内单测写在同文件 `#[cfg(test)] mod tests`，用 `Fixture` 结构体 + `Drop` 清临时目录（`custom_models.rs:209-232` 模板）。
11. **不要动版本号**（`AGENT.md:372` 红线），除非用户明确要求发版。

---

# D. 可直接照抄的模板

## D1. Rust：`src-tauri/src/commands/extdemo.rs`

```rust
//! 扩展示例命令域（新增一级页面的后端支撑）。
//! ⚠ serde 命名约定：全部 snake_case，与前端 types.ts 严格对齐。
//! 错误统一 Result<_, String>（前端 String(err) 直接展示）。

use tauri::State;

use crate::api_server::gateway_settings;
use crate::state::AppState;

/// 页面状态 DTO（前端 types.ts 同名接口字段必须一致）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ExtDemoStatus {
    /// 网关是否在跑（本例仅演示字段来源）
    pub running: bool,
    /// 网关端口（复用 gateway_settings）
    pub gateway_port: u16,
    /// 默认模型
    pub default_model: String,
    /// 自定义模型数量（复用 custom_models 数据层）
    pub custom_model_count: usize,
}

/// 读状态：复用网关设置层 + 自定义模型数据层，不新增存储
#[tauri::command]
pub fn extdemo_status(state: State<'_, AppState>) -> ExtDemoStatus {
    let gw = gateway_settings::load(&state.data_dir);
    ExtDemoStatus {
        running: false,
        gateway_port: gw.port,
        default_model: gw.default_model,
        custom_model_count: crate::api_server::custom_models::load(&state.data_dir).len(),
    }
}

/// 查询参数（嵌套结构体：字段保持 snake_case；前端 payload 形如 { opts: { keyword, limit } }）
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ExtDemoQueryOpts {
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// 带可选参数的查询
#[tauri::command]
pub fn extdemo_query(
    state: State<'_, AppState>,
    opts: ExtDemoQueryOpts,
) -> Result<Vec<String>, String> {
    let limit = opts.limit.unwrap_or(20).clamp(1, 200);
    let kw = opts.keyword.unwrap_or_default();
    // 示例：读 kv 文档 + 过滤
    let store = crate::store::db(&state.data_dir);
    let raw: serde_json::Value = store.kv_get("extdemo_settings");
    let mut out: Vec<String> = Vec::new();
    if let Some(arr) = raw.get("items").and_then(|v| v.as_array()) {
        for it in arr {
            let s = it.as_str().unwrap_or_default();
            if kw.is_empty() || s.contains(&kw) {
                out.push(s.to_string());
            }
        }
    }
    out.truncate(limit);
    Ok(out)
}

/// 写设置：kv 文档键 `extdemo_settings`（patch 合并语义，只写出现的字段）
#[tauri::command]
pub fn extdemo_settings_set(
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let store = crate::store::db(&state.data_dir);
    let mut cur: serde_json::Value = store.kv_get("extdemo_settings");
    if !cur.is_object() {
        cur = serde_json::json!({});
    }
    if let (Some(c), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            c.insert(k.clone(), v.clone());
        }
    }
    store.kv_set("extdemo_settings", &cur)?;
    Ok(cur)
}
```

注册两步：
```rust
// src-tauri/src/commands/mod.rs
pub mod extdemo;

// src-tauri/src/main.rs（generate_handler! 内，398 行的 ]) 之前）
commands::extdemo::extdemo_status,
commands::extdemo::extdemo_query,
commands::extdemo::extdemo_settings_set,
```

## D2. TS：`src/lib/tauri.ts` 的封装片段

```ts
  extDemo: {
    status: () => invoke<ExtDemoStatus>('extdemo_status'),
    // 顶层 camelCase，嵌套 opts 字段 snake_case
    query: (opts: { keyword?: string; limit?: number }) =>
      invoke<string[]>('extdemo_query', {
        opts: { keyword: opts.keyword ?? null, limit: opts.limit ?? null },
      }),
    settingsSet: (patch: Partial<ExtDemoSettings>) =>
      invoke<ExtDemoSettings>('extdemo_settings_set', { patch }),
  },
```

## D3. TSX：`src/pages/ExtDemo.tsx` 页面骨架

```tsx
import { useCallback, useEffect, useState } from 'react';
import { RefreshCw, Save } from 'lucide-react';
import PageHeader from '../components/PageHeader';
import { Badge, EmptyState, Modal, Spinner, StatCard } from '../components/ui';
import { useAppStore } from '../store';
import { api } from '../lib/tauri';
import { withMinDelay } from '../lib/delay';
import type { ExtDemoStatus } from '../types';

/** 扩展示例页：复用数据层/网关层，不新增存储格式 */
export default function ExtDemo() {
  const toast = useAppStore((s) => s.pushToast);
  const refreshSettings = useAppStore((s) => s.refreshSettings);

  const [status, setStatus] = useState<ExtDemoStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setStatus(await api.extDemo.status());
    } catch (err) {
      toast('error', `加载失败：${String(err)}`);
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    void load();
    void refreshSettings();
  }, [load, refreshSettings]);

  const save = async () => {
    setSaving(true);
    try {
      await withMinDelay(api.extDemo.settingsSet({ enabled: true }));
      toast('success', '已保存');
    } catch (err) {
      toast('error', `保存失败：${String(err)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      <PageHeader
        title="扩展示例"
        desc="新增一级页面模板 · 复用网关设置与自定义模型数据层"
        actions={
          <button className="btn-outline" onClick={() => void load()} disabled={loading}>
            <RefreshCw size={15} /> {loading ? '刷新中…' : '刷新'}
          </button>
        }
      />

      <div className="mt-5 grid grid-cols-3 gap-3">
        <StatCard label="网关端口" value={status?.gateway_port ?? '—'} tone="blue" hint="来源 api_gateway_settings" />
        <StatCard label="默认模型" value={status?.default_model ?? '—'} tone="violet" />
        <StatCard label="自定义模型" value={status?.custom_model_count ?? 0} tone="green" hint="custom_models 表" />
      </div>

      <div className="mt-5 card p-5">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-sm font-semibold">示例区块</h2>
          <Badge tone="brand">v1</Badge>
        </div>
        {status ? (
          <button className="btn-primary flex items-center gap-1" onClick={() => setConfirmOpen(true)}>
            <Save size={15} /> 保存
          </button>
        ) : (
          <EmptyState title="暂无数据" hint="点击右上角刷新" />
        )}
      </div>

      <Modal
        open={confirmOpen}
        onClose={() => setConfirmOpen(false)}
        title="确认保存"
        footer={
          <>
            <button className="btn-ghost" onClick={() => setConfirmOpen(false)} disabled={saving}>
              取消
            </button>
            <button
              className="btn-primary flex items-center gap-1"
              onClick={() => void save().then(() => setConfirmOpen(false))}
              disabled={saving}
            >
              {saving ? <Spinner className="text-white" /> : <Save size={15} />}
              {saving ? '保存中…' : '确认'}
            </button>
          </>
        }
      >
        将写入 <code>extdemo_settings</code>（kv 文档）。
      </Modal>
    </div>
  );
}
```

## D4. 三处导航接线（照抄）

```ts
// src/types.ts:4 —— 追加成员
  | 'ext-demo'
```

```ts
// src/components/Sidebar.tsx:28 —— 对应平台数组追加
  { key: 'ext-demo', label: '扩展示例', icon: Boxes },
```

```tsx
// src/App.tsx —— import + case
import ExtDemo from './pages/ExtDemo';
...
    case 'ext-demo':
      return <ExtDemo />;
```

---

# E. 自检清单（合并前逐项打勾）

- [ ] `types.ts::ViewKey` 有成员 → `Sidebar` 有菜单项 → `App.tsx::renderView` 有 `case`（三处 key 字面量必须完全一致）
- [ ] 新命令在 `main.rs::generate_handler!` 内注册（否则运行期 `command not found`）
- [ ] 新模块在 `commands/mod.rs` 声明（否则编译不过）
- [ ] 所有 DTO 新字段带 `#[serde(default)]`；两侧字段名都是 `snake_case`；顶层参数在前端写 camelCase
- [ ] 读热路径考虑 `config_cache::get_or_load`，写后 `config_cache::invalidate`
- [ ] 增量改动 API Key 用 `api_keys::modify`，不用 `load → save`
- [ ] 新行表：`schema.rs` DDL + `ROW_TABLES` + `SCHEMA_VERSION` + `migrate.rs` 闸门
- [ ] 异步按钮 `withMinDelay`；确认操作 `Modal`（禁 `window.confirm`）
- [ ] 文案/注释中文；错误 `Result<_, String>`；不在日志里落凭证
- [ ] `cargo test` + `npm run build` 通过
