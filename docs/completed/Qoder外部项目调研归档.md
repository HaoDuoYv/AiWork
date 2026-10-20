# F-80 M0 产出物：外部项目调研归档（外部参考实现价值分析）

> **调研日期**: 2026-09-27
> **调研方式**: GitHub Contents API + raw 源码逐文件分析（两轮：价值评估 → 实现级深挖）
> **用途**: ① 多账号并发（设计文档 §5.10，每账号稳定指纹）的实现参考底稿；② 签到/积分/凭证通道的契约互证
> **关联**: `docs/completed/Qoder平台支持设计方案.md` v1.2（R-12 侦察项前置调研）
> **源码副本**: 两轮深挖时已留存 `main.py`（qoder-creator，2347 行）与 `registrar.py`（QoderGateway，765 行）到本地 `.recon_tmp/`（未入库，仅供移植期比对）

---

## 0. 结论速览

| # | 仓库 | 技术栈 | 核心内容 | 价值 | 对 F-80 的用途 |
|---|---|---|---|---|---|
| 1 | okky-x0f/qoder-creator | Python + Playwright | QoderPatcher 指纹伪造 + 桌面端自动登录 | **高**（v1.2 需求变更后由"规避向"升级为多账号并发核心参考） | §5.10 指纹字段算法与写入路径 |
| 2 | bzym2/QoderGateway | Python 异步网关 | COSY 授权链 + 多账号池（每账号稳定 machine_id） | **高** | 凭证/token 生命周期、多账号绑定模型 |
| 3 | caigee-cmd/cli2api | JS (CF Worker) | **CN 域直连签到** | **高**（附加发现） | 签到契约互证（与本项目 R-4/R-6 结论一致） |
| 4 | mmqz/cpa-multi-plugins | Go 插件 | campaigns 契约最全实现 | **高**（附加发现） | claim 幂等/恢复逻辑、上游行为情报 |
| 5 | avaritiachaos/qoder-proxy | Node.js | CLI 包装代理 + 本地安全加固 | 中 | 本地服务安全清单 |
| 6 | foxy1402/qoder-proxy | Node.js | 每请求 spawn CLI 子进程 | 低（反模式） | 仅作架构警示 |

> 检索方法说明：对 `"sash/api/v1/me/campaigns"` 做全局代码搜索（70 条命中）发现 #3、#4 两个直接实现签到的仓库，贴题度反超指定仓库。

---

## 1. okky-x0f/qoder-creator —— QoderPatcher 指纹伪造

### 1.1 定位

Playwright 驱动 **Qoder 桌面客户端（Electron）** 的全自动登录/领 Pro Trial 脚本（单文件 `main.py` 约 109KB）。核心路径：每账号 → 重写本地指纹文件 → 启动 Qoder Desktop → device auth 登录。**不直接调用 Qoder API**（已确认全文无 `Cosy-*` 请求头、无 machine_token 概念）——它负责"造指纹 + 驱动客户端"，让 Qoder Desktop 自己带着本地指纹完成登录。

### 1.2 三平台路径配置（情报价值：桌面端数据目录映射）

```python
# 来源：main.py L58-106
"Windows": {
    "app_path_user": "%LOCALAPPDATA%/Qoder/Qoder.exe",
    "data_dir": "%APPDATA%/Qoder",            # ← 指纹在此（注意不是 %LOCALAPPDATA%）
    "user_dir": "~/.qoder",                    # 用户级数据（三平台一致）
    "cache_dir": "%LOCALAPPDATA%/Qoder/Cache",
    "preferences": "%APPDATA%/Qoder/Preferences",
}
# macOS: data_dir=~/Library/Application Support/Qoder, preferences=com.qoder.Qoder.plist
# Linux: data_dir=~/.config/Qoder
```

### 1.3 指纹字段全清单与生成算法

| 字段 | 生成算法 | 格式 | 适用平台 |
|---|---|---|---|
| `machine_id` | `md5(str(random.randint(1000000, 9999997)))` | 32 位小写 hex，无分隔符 | 三平台 |
| `ms_deviceid` | `str(uuid.uuid4())` | 36 位标准 uuid（含 `-`，小写） | 三平台 |
| `umid` | `str(uuid.uuid4())` | 36 位标准 uuid | 仅 macOS |
| `mac_address` | 6 字节各随机 `:` 分隔两位 hex | `aa:bb:cc:dd:ee:ff`（未设本地管理位） | 仅 macOS |

```python
# 来源：main.py L371-400
def generate_machine_id(self):
    machine_id = hashlib.md5(str(random.randint(1000000, 9999997)).encode()).hexdigest()[:32]
```

### 1.4 写入位置与文件格式（关键）

```python
# 来源：main.py L428-510（patch_qoder_data 节选）
# 先备份整个 data_dir，再删除旧状态文件并重写为「裸文本」：
#   <data_dir>/machineid          ← md5 hex，32 字符
#   <data_dir>/serviceMachineId   ← 与 machineid 同值
#   <data_dir>/ms_deviceid        ← uuid4
#   <data_dir>/state.vscdb        ← 删除（登录态 SQLite，删掉即"未登录"）
#   <data_dir>/patch_info.json    ← JSON 留痕（platform/machine_id/ms_deviceid/patched_at）
```

### 1.5 PKCE / nonce 与 device auth

```python
# 来源：main.py L402-426, L926-960
code_verifier = base64.urlsafe_b64encode(os.urandom(32)).rstrip(b'=').decode()
code_challenge = base64.urlsafe_b64encode(sha256(verifier).digest()).rstrip(b'=').decode()
nonce = uuid.uuid4().hex                       # 32 位小写 hex
# device auth URL:
# https://qoder.com/device/selectAccounts?nonce=&challenge=&challenge_method=S256
#   &redirect_uri=qoder://aicoding.aicoding-agent/login-success&machine_id=<mid>
```

**关键教训（源码注释原文 L1887-1893）**：自造 PKCE URL 不可行——Qoder Desktop 内部保存的是它自己生成的 `code_verifier`，第三方造的 `challenge` 配不上对。脚本因此优先从 Chrome 标签页**捕获 Desktop 自己打开的 device auth URL**，失败才退化到不带 PKCE 的简化 URL。

### 1.6 多账号绑定方式（反面示范，需改造）

- 账号文件仅 `EMAIL|PASSWORD`，不含指纹字段；
- 每账号登录前 `patch_qoder_data()`，且 `launch_qoder` 内还会再 patch 一次——**每账号全新随机指纹、一次登录内重生成两遍**；
- 指纹不与账号持久化绑定、不回写账号文件。
- **结论**：该仓库是"一次性指纹 + 单机串行换号"模式；**每账号稳定指纹**的正确示范在 QoderGateway（§2.6）。

### 1.7 附：重置逻辑佐证

`reset_qoder_deep` 整删 `data_dir / ~/.qoder / cache / preferences / saved_state`；`reset_qoder_completely` 删后重建并立即重播指纹。说明 `~/.qoder` 亦是登录态目录，换号时必须一并清理。

---

## 2. bzym2/QoderGateway —— COSY 授权链与多账号账号池

网关型项目：SQLite 多账号池 + COSY 签名直连 Qoder 后端 API + Device Flow 注册机。**这是"每账号绑定稳定 machine_id"的现成正确实现**。

### 2.1 COSY 会话密钥协商（temp_key 双层加密）

```python
# 来源：src/qoder2api/auth.py
SERVER_PUBKEY_PEM = b"""-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDA8iMH5c02LilrsERw9t6Pv5Nc
4k6Pz1EaDicBMpdpxKduSZu5OANqUq8er4GM95omAGIOPOh+Nx0spthYA2BqGz+l
6HRkPJ7S236FZz73In/KVuLnwI8JJ2CbuJap8kvheCCZpmAWpb/cPx/3Vr/J6I17
XcW+ML9FoCI6AOvOzwIDAQAB
-----END PUBLIC KEY-----"""    # RSA 1024-bit SPKI

# temp_key = uuid4().hex[:16]（16 个 ASCII 字符）
# cosy_key = base64(RSA_PKCS1v15(temp_key))          → 128B 密文 → 172 字符
# info     = base64(AES-128-CBC(身份JSON, key=IV=temp_key) + PKCS7)
# 身份 JSON 内层：name/aid/uid/yx_uid/organization_id/organization_name/
#                user_type/security_oauth_token/refresh_token
# payload  = base64(紧凑 JSON 按 key 排序：cosyVersion="0.1.43", ideVersion="",
#                info, requestId=uuid4(每请求重生成), version="v1")
```

### 2.2 请求签名与完整请求头

```python
# 来源：src/qoder2api/auth.py
def sign_request(payload_b64, cosy_key, cosy_date, body, path_without_algo):
    raw = f"{payload_b64}\n{cosy_key}\n{cosy_date}\n{body}\n{path_without_algo}"
    return hashlib.md5(raw.encode()).hexdigest()
# path 需去掉 /algo 前缀；cosy-date 为 unix 秒字符串
```

请求头全家族：`cosy-data-policy: AGREE`、`cosy-clienttype: "5"`（IDE 型）、`cosy-version: 0.1.43`、`cosy-user: <uid>`、`cosy-key`、`cosy-clientip: 169.254.198.161`（**硬编码**）、`authorization: Bearer COSY.{payload_b64}.{sig}`、`cosy-machineid` / `cosy-machinetoken` / `cosy-machinetype`、`login-version: v2`、`user-agent: Go-http-client/2.0`。

### 2.3 设备身份字段来源（多账号并发的直接答案）

```python
# 来源：src/qoder2api/auth.py
def new_machine():
    machine_id    = str(uuid.uuid4())                                  # 36 字符带连字符
    seed          = (str(uuid.uuid4()) + str(uuid.uuid4()))[:50].encode()
    machine_token = base64.urlsafe_b64encode(seed).decode().rstrip("=") # 68 字符 base64url
    machine_type  = uuid.uuid4().hex[:18]                              # 18 位 hex
```

**取值规则**：`machine_id` **持久稳定**（从账号记录读取）；`machine_token` / `machine_type` **每次会话现场随机生成**——实测服务端对后两者随机即通过，说明 machine_id 与 machinetoken 之间**无服务端强绑定校验**（CN 端需 R-12 复核）。

### 2.4 本地凭据解密（`~/.qoder/.auth/`）

```python
# 来源：src/qoder2api/auth.py load_local_session（节选）
auth_dir = Path.home() / ".qoder" / ".auth"
machine_id = (auth_dir / "id").read_text().strip()      # fallback 文件名: machine_id
cipher = base64.b64decode((auth_dir / "user").read_text().strip())
key = machine_id[:16].encode("ascii")                    # key = IV = machine_id 前 16 字符
plain = AES_128_CBC_decrypt_PKCS7(cipher, key, iv=key)   # 解出身份 JSON（兼容 camel/snake 双命名）
```

- `~/.qoder/.auth/id`：裸文本 machine_id；`~/.qoder/.auth/user`：base64(AES-128-CBC(身份 JSON, key=IV=machine_id[:16]))。
- PKCS7 去填充失败 ⇒ machine_id 截断方式不对（含 `-` 的 uuid 前 16 字符截断是已验证写法）。
- **对本项目的价值**：可用于"导入 CLI/桌面端已有登录态"；注意 CN CLI 认证目录为 `~/.qoderworkcn`（需实测确认）。

### 2.5 token 生命周期

```python
# 来源：src/qoder2api/tokens.py
OPENAPI = "https://openapi.qoder.sh"; UA = "qoder/1.1.16"; REFRESH_INTERVAL = 6 * 3600
# 前缀分派：jrt- → POST /api/v1/jobToken/refresh（token_key="token"）
#           其余  → POST /api/v1/deviceToken/refresh（token_key="device_token"）
# body {"refresh_token": rt}；响应 {token, refresh_token, expires_at}
# 限额：GET /api/v2/quota/usage（Bearer security_oauth_token）
```

后台线程每 6 小时全量刷新 enabled 账号——可对照本项目 ensure_fresh 的惰性刷新做兜底。

### 2.6 账号池与每账号稳定绑定（照抄的模型）

```sql
-- 来源：src/qoder2api/database.py
CREATE TABLE accounts (
    uid TEXT PRIMARY KEY, name TEXT, user_type TEXT,
    security_oauth_token TEXT NOT NULL, refresh_token TEXT NOT NULL,
    machine_id TEXT NOT NULL,            -- ← 设备指纹入库，每账号一条
    enabled INTEGER DEFAULT 1, last_status TEXT, last_error TEXT,
    quota INTEGER, is_quota_exceeded INTEGER, plan TEXT, user_tag TEXT, next_reset_at INTEGER
);
```

- 会话构造：`machine_id` 取库、`machine_token/machine_type` 现场随机（`new_machine()`）；
- 失败账号 `(idx+1) % n` 环形轮换并回写 `last_status/last_error`；
- **敏感字段出库前剥离**：对外 API 列表 pop 掉 `security_oauth_token / refresh_token / machine_id`；
- 批量导入时 machine_id 缺省填 `uuid4`。

### 2.7 algo 端点辅助算法（备选底稿）

- `signature.py`：`APPCODE="cosy"`、`SECRET="d2FyLCB3YXIgbmV2ZXIgY2hhbmdlcw=="`（解码 "war, war never changes"）、`sign = md5("cosy&{SECRET}&{RFC1123 GMT date}")`；
- `encoding.py`（`?Encode=1` 混淆）：标准 base64 → 三段重排（`s[-n:] + s[:len-n] 段交换`）→ 65 字符自定义表映射（`CUSTOM_ALPHABET="_doRTgHZBKcGVjlvpC,@aFSx#DPuNJme&i*MzLOEn)sUrthbf%Y^w.(kIQyXqWA!"`，pad=`$`）；
- jobToken 交换：`POST https://center.qoder.sh/algo/api/v3/user/jobToken?Encode=1`（date 用 RFC1123 GMT，另加 appcode+signature 头；响应映射 `id→aid/uid, name, userType, securityOauthToken, refreshToken`）；status 端点同构（`quota/isQuotaExceeded/plan/userTag/nextResetAt`）。
- ⚠️ 全部面向国际站 `.sh` 域，CN 端移植必须实测。

### 2.8 Device Flow 注册机（registrar.py）

```python
# 来源：src/qoder2api/registrar.py
DEVICE_CLIENT_ID = "e883ade2-e6e3-4d6d-adf7-f92ceff5fdcb"
# verifier: 43-128 字符，字符集 [A-Za-z0-9-._~]（RFC 7636 合规）
# auth_url: https://qoder.com/device/selectAccounts?challenge=&challenge_method=S256
#           &nonce=str(uuid4)&machine_id=<mid>&client_id=<DEVICE_CLIENT_ID>
# poll_url: https://openapi.qoder.sh/api/v1/deviceToken/poll?nonce=&verifier=&challenge_method=S256
#           GET，404=等待授权，200=凭据就绪；1s 间隔，300s 超时
# token 响应字段：token / refresh_token / user_id / expires_at / refresh_token_expires_at
```

- token 响应**不含 machine_id 绑定字段**；入库时 `_save_account` 重新随机 machine_id，与授权 URL 的 mid 无关联校验也成功——再次佐证服务端不校验 machine_id 一致性。
- 本项目 `tasks/qoder_oauth.rs` 已实现的设备流与本节参数一致（CN 端授权页实测为 `qoder.cn` 域）。

---

## 3. 附加发现的高价值参考

### 3.1 caigee-cmd/cli2api —— CN 签到直连（与本方案互证）

```js
// 来源：worker/src/checkin.mjs
const endpoints = { cn: { base: "https://openapi.qoder.com.cn", origin: "https://qoder.com.cn" } };
// headers: Authorization: Bearer ${token}, User-Agent: "Qoder",
//          "Cosy-ClientType": "10", "Cosy-Version": "0.3.4",
//          Origin: "https://qoder.com.cn",
//          Referer: "https://openapi.qoder.com.cn/growth-page/activity-iframe"
// redirect: "manual", timeout 15s
```

流程：`refreshTokenIfNeeded → GET campaigns（404→skipped）→ 过滤 CLAIM_BENEFIT/CLAIMABLE → 逐个 POST claim（campaignId encodeURIComponent）→ status!=="CLAIMED" 抛错`；**失败恢复**：claim 失败后复查列表，已变 CLAIMED 则视为成功；401 → forceRefresh 后重试一次；`forceRefresh=true` 保证列表新鲜；pending 单飞去重防并发重复签到。

**价值**：CN 端点直连，`Cosy-ClientType: "10"`、UA `"Qoder"`、Bearer 直调——与本仓库未提交代码的 R-4/R-6 结论逐字吻合，独立互证正确性。

### 3.2 mmqz/cpa-multi-plugins —— campaigns 契约最全（Go）

```go
// 来源：plugins/qoder/campaign.go
// fetchCampaignStatus: GET {billingBase}/sash/api/v1/me/campaigns?forceRefresh=true，10s 超时
// claimableCampaign: 过滤 actionType=="CLAIM_BENEFIT" && claimStatus=="CLAIMABLE" && 时间窗(startAt/endAt)
// performCampaignCheckin: POST .../campaigns/{id}/claim，body "{}"
//   兼容 裸对象 / {data:} 包裹；status=="CLAIMED" 为成功；replayed==true → ALREADY_CLAIMED
```

**上游行为情报（源码注释，含 2026-09-21 实测）**：
1. 旧签到端点 `/sash/api/v1/me/daily-check-in/{status,claim}` 已被上游**全球禁用**（status 返回 DISABLED、claim 返回 409）；CN 自 v0.12.80、Intl 自 v0.8.18 起统一走 campaigns；
2. **CN 端 campaigns 响应可能缺 `showCampaign`/`claimable` 标志字段，必须以 `claimStatus=="CLAIMABLE"` 行为权威**（本项目代码已如此实现）；
3. 已领取的活动结束后从列表消失；Intl 端无 Pro 升级类活动；
4. 可吸收：**claim 失败后复查列表 CLAIMED 视为成功**的恢复逻辑（本项目尚未实现，建议补）。

---

## 4. avaritiachaos/qoder-proxy —— 本地服务安全清单（中）

`auth.js`（6229B）纯本地安全加固，无 Qoder API 逻辑，可整套移植到 Tauri 本地服务/网关自查：
- **DNS rebinding 防护**：Host 白名单 + 解析校验（`isAllowedHost`）；
- Origin 白名单仅 loopback（`isAllowedOrigin`）；
- API Key 校验 `sha256 + timingSafeEqual`（防时序攻击）；
- 日志脱敏。

## 5. foxy1402/qoder-proxy —— 反模式警示（低）

每请求 spawn 一个 qodercli 子进程转发——高并发下进程创建开销与资源峰值极大。**警示**：本项目若需 CLI 协作应常驻 sidecar 或直连 API（本项目直连即可，无需 CLI）。

---

## 6. Rust 移植要点（§5.10 实现时对照）

### 6.1 可直接翻译的纯算法

| 算法 | Python 源 | Rust 实现 | 注意点 |
|---|---|---|---|
| machine_id md5 hex | `md5(str).hexdigest()[:32]` | `md-5` crate | 输出 32 小写 hex；**原实现种子空间仅 1e7，改 uuid4 做 seed 增熵** |
| uuid4 | `str(uuid.uuid4())` / `.hex` | `uuid` crate | 带连字符 36 字符；`.hex` = `simple()` 32 字符 |
| machine_token | base64url((uuid4+uuid4)[:50]) 去 `=` | `base64::URL_SAFE_NO_PAD` | 50B → 68 字符 |
| machine_type | `uuid4().hex[:18]` | `uuid.simple()[..18]` | 18 hex 字符 |
| PKCE S256 | base64url(sha256(verifier)) 去 `=` | `sha2` + `URL_SAFE_NO_PAD` | verifier 43-128 字符 `[A-Za-z0-9-._~]`；nonce 两仓库写法不同（`uuid4().hex` vs `str(uuid4())`），CN 端实测哪个被接受 |
| AES-128-CBC（key=IV） | `Cipher(AES(key), CBC(key))` + PKCS7 | `aes` + `cbc` crate | 本地解密 key=machine_id 前 16 ASCII 字符（含 `-`）；COSY info 用 temp_key 同字节作 key 与 IV |
| RSA PKCS#1 v1.5 | `load_pem_public_key` + PKCS1v15 | `rsa` crate | 1024-bit SPKI PEM；密文 128B → 标准 base64 172 字符 |
| 请求签名 | `md5("payload\nkey\ndate\nbody\npath")` | `md-5` | 五段 `\n` 拼接；path 去 `/algo` 前缀；cosy-date=unix 秒 |
| appcode 签名 | `md5("cosy&{SECRET}&{RFC1123 date}")` | `md-5` + `chrono` | RFC1123 GMT（`Fri, 26 Sep 2025 12:00:00 GMT`） |
| 自定义 base64 | 三段重排 + 65 字符表映射 | `base64::STANDARD` 后手写翻译表 | 仅 body，不进 URL；decode 为精确逆序 |

### 6.2 需本机实测验证的点（→ R-12）

1. **machine_id"真源"文件**：仓库 1 写 `%APPDATA%\Qoder\machineid`（+`serviceMachineId` 同值）；仓库 2 读 `~/.qoder/.auth/id`——两处语义不同（Electron data 目录 vs CLI 认证目录）。CN 端 Qoder Desktop 启动时读哪个、格式是否裸文本，需本机验证。
2. **服务端是否校验 machine_id ↔ token 绑定**：QoderGateway 证据强烈暗示无强校验（machinetoken 每次随机 + 授权 mid 与入库 mid 不一致均通过），需 CN 端实测"换新 machine_id + 旧 refresh_token"。
3. **cosy-clientip 硬编码 `169.254.198.161`** 是否必须原样、可否每账号不同。
4. **cosy-date 双格式并存**：业务头 unix 秒，jobToken/status 头 RFC1123 GMT。
5. **CN CLI 认证目录**是否为 `~/.qoderworkcn`（影响本地登录态导入路径）。

### 6.3 多账号并发绑定模型（建议照抄）

SQLite/账号记录每账号持久化 `machine_id`（+ device_id/umid/mac）→ 会话构造时 machine_id 取库、machine_token/machine_type 现场随机 → 失败环形轮换回写状态 → 对外列表剥离敏感字段。

---

## 7. 与 F-80 设计的映射总表

| F-80 需求 | 参考实现 | 落点 |
|---|---|---|
| 每账号稳定指纹生成 | qoder-creator §1.3（算法）+ QoderGateway §2.3（字段分工） | `QoderDeviceProfile` 结构 + 生成函数（M3） |
| 指纹本地落盘（IDE 并发/切换） | qoder-creator §1.4（裸文本三文件） | `machine.rs` 思路 + R-12 实测定稿（M3） |
| `Cosy-MachineId` 请求注入 | QoderGateway §2.2/§2.3 | `tasks/qoder_common.rs::build_auth_headers`（已有，改为取 device_profile） |
| 签到主流程 | cli2api §3.1 + cpa-multi-plugins §3.2 | `tasks/qoder_checkin.rs`（已实现；补 claim 失败复查恢复逻辑） |
| token 生命周期 | QoderGateway §2.5 | `tasks/qoder_common.rs::ensure_fresh`（已实现，6h 定时刷新可作兜底参考） |
| 本地登录态导入 | QoderGateway §2.4（AES-CBC key=IV=machine_id[:16]） | `commands/qoder/accounts.rs` 导入通道（M3） |
| 账号池工程 | QoderGateway §2.6 | `store/qoder_accounts` 表（已有）+ 敏感字段出库剥离（待补） |
| COSY 签名链备选 | QoderGateway §2.1/§2.2/§2.7 | 仅 algo 端点门槛升级时启用（文档留档） |
| 本地服务安全 | avaritiachaos §4 | API 网关自查清单 |

## 8. 风险与规避清单

| 风险 | 说明 | 规避 |
|---|---|---|
| 条款灰色 | 多账号指纹伪造属"同一设备"去重的对抗用法（设计文档 §七 风险 1） | 每账号**稳定**指纹（非随机轮换）；UI 固定风险提示；不做批量注册 |
| 旧端点陷阱 | `/sash/api/v1/me/daily-check-in/*` 已全球禁用 | 一律走 campaigns |
| CN 字段不全 | campaigns 响应可能缺 `showCampaign/claimable` 标志位 | 以 `claimStatus=="CLAIMABLE"` 行为唯一权威（已实现） |
| 域差 | QoderGateway/qoder-creator 面向国际站 | 所有端点/算法 CN 端实测后才移植 |
| 指纹质量 | QoderPatcher 的 md5(random) 种子空间仅 1e7 | 移植时 uuid4 增熵 |
| PKCE 配对 | 第三方自造 challenge 与 Desktop 内部 verifier 配不上 | 捕获客户端自产的 auth URL，或走本项目的 PAT/设备流通道 |
