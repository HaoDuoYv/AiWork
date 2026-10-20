//! WorkBuddy 桌面端 at-rest 加密凭据解密（回收自 `wb_atrest.py`，对齐客户端
//! `packages/at-rest-crypto`）。
//!
//! ## 为什么需要
//! 桌面客户端自 2026-09-24 起把 `.info` 里的 `accessToken` / `refreshToken`
//! （国内版连 `nickname` / `phoneNumber`）从明文改成
//! `{"$wbEncrypted":1,"envelope":"<base64(JSON)>"}` 信封。扫码导入只剩明文那份
//! （`workbuddy-desktop-ai.info` = 国际版），国区账号（`workbuddy-desktop.info`）
//! 因此**永远识别不到**——「国际区和国区是两个账户，都要可以识别」的根因就在这里。
//!
//! ## 方案（逐字节对齐参考实现，任一环节错位 GCM 校验必然失败）
//! ```text
//! key       = sha256(atRestSecretKey_base64 字符串)   // 客户端构建载荷派生
//! keyId     = sha256(key).hex[0..16]
//! AAD       = "WB-AAD\0" | 0x01 | LP(framingMagic) | LP("sym-v1") | u32(suite)
//!             | LP(keyId) | framingCode | 0x00 | 0x00          (LP = u32be 长度 + utf8)
//! envelope  = base64(JSON{suite,keyId,nonce,authTag,ciphertext})   AES-256-GCM
//! framing   : 文件 → WBEF1/1（keyblob），字段 → WBEV1/2（.info 里的字段）
//! ```
//! 静态密钥编译在客户端原生模块里，磁盘上拿不到明文，只能从**运行中的**客户端
//! 进程内存里找回来（`recover_key`）：判据是 `sha256(候选)[..16]` 等于 keyblob
//! `static-v1` 槽的 `protectorKeyId`，因此客户端升级换钥后自动失效、不会用错钥。
//!
//! ## 密钥来源（按序，命中即止）
//! 1. 环境变量 `AIWORK_WB_ATREST_KEY` / `WORKBUDDY_ATREST_KEY`（base64 或 hex，人工兜底）；
//! 2. 数据目录 `wb_atrest.key`（用户手工粘贴一次，仅本机）；
//! 3. 客户端进程内存扫描（Windows；客户端需正在运行）。
//!
//! 密钥只留在进程内存（`CACHE`），不落日志、不进账号池；keyblob 每次使用前复核，
//! 客户端换钥后内存里的旧钥自动作废。

use std::path::PathBuf;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// 信封字段名
pub const ENVELOPE_KEY: &str = "$wbEncrypted";
const SCHEME: &str = "sym-v1";
const DEFAULT_SUITE: u32 = 1;
const KEYBLOB_NAME: &str = "keyblob";

/// 信封载体：AAD 里的 magic / framingCode 必须与载体一一对应
#[allow(dead_code)] // Record/Stream 为客户端其余帧形态保留（当前只用到 File / Field）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// keyblob 里的 wrapped 槽（WBEF1 / 1）
    File,
    /// `.info` 里的字段（WBEV1 / 2）
    Field,
    /// 记录（WBER1 / 3）
    Record,
    /// 流（WBES1 / 4）
    Stream,
}

impl Framing {
    fn magic(self) -> &'static [u8] {
        match self {
            Framing::File => b"WBEF1",
            Framing::Field => b"WBEV1",
            Framing::Record => b"WBER1",
            Framing::Stream => b"WBES1",
        }
    }

    fn code(self) -> u8 {
        match self {
            Framing::File => 1,
            Framing::Field => 2,
            Framing::Record => 3,
            Framing::Stream => 4,
        }
    }
}

fn lp(text: &str) -> Vec<u8> {
    let raw = text.as_bytes();
    let mut out = Vec::with_capacity(4 + raw.len());
    out.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    out.extend_from_slice(raw);
    out
}

/// GCM AAD（逐字节对齐客户端）
pub fn aad(key_id: &str, suite: u32, framing: Framing) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"WB-AAD\0");
    out.push(0x01);
    out.extend_from_slice(&lp(std::str::from_utf8(framing.magic()).unwrap_or("")));
    out.extend_from_slice(&lp(SCHEME));
    out.extend_from_slice(&suite.to_be_bytes());
    out.extend_from_slice(&lp(key_id));
    out.push(framing.code());
    out.push(0x00);
    out.push(0x00);
    out
}

/// `protectorKeyId` / `envelope.keyId`：sha256(key) 的前 16 个 hex
pub fn derive_key_id(key: &[u8]) -> String {
    let digest = Sha256::digest(key);
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// 是不是一个 `$wbEncrypted` 信封（而不是明文 token）
pub fn is_envelope(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.get("envelope").is_some_and(Value::is_string) && m.contains_key(ENVELOPE_KEY),
        Value::String(s) => {
            let t = s.trim_start();
            t.starts_with('{') && t.chars().take(64).collect::<String>().contains(ENVELOPE_KEY)
        }
        _ => false,
    }
}

struct ParsedEnvelope {
    suite: u32,
    key_id: String,
    nonce: Vec<u8>,
    tag: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn b64_field(v: Option<&Value>, name: &str) -> Result<Vec<u8>, String> {
    let s = v
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("envelope 缺字段 {name}"))?;
    B64.decode(s.trim()).map_err(|e| format!("envelope {name} 非法 base64: {e}"))
}

fn parse_envelope(v: &Value) -> Result<ParsedEnvelope, String> {
    // dict 形态 → 取 envelope 字符串；字符串形态 → 直接 base64(JSON)
    let encoded = match v {
        Value::Object(m) => m
            .get("envelope")
            .and_then(Value::as_str)
            .ok_or("envelope 字段缺失")?
            .to_string(),
        Value::String(s) => s.trim().to_string(),
        _ => return Err("不是加密信封".into()),
    };
    let raw = B64
        .decode(encoded.trim())
        .map_err(|e| format!("envelope 不是 base64(JSON): {e}"))?;
    let env: Value = serde_json::from_slice(&raw).map_err(|e| format!("envelope JSON 解析失败: {e}"))?;
    let suite = env
        .get("suite")
        .and_then(Value::as_u64)
        .map(|n| n as u32)
        .unwrap_or(DEFAULT_SUITE);
    let key_id = env
        .get("keyId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(ParsedEnvelope {
        suite,
        key_id,
        nonce: b64_field(env.get("nonce"), "nonce")?,
        tag: b64_field(env.get("authTag"), "authTag")?,
        ciphertext: b64_field(env.get("ciphertext"), "ciphertext")?,
    })
}

/// 用静态 at-rest 密钥解开一个信封 → 明文字节
pub fn open_envelope(key: &[u8], value: &Value, framing: Framing) -> Result<Vec<u8>, String> {
    let env = parse_envelope(value)?;
    let expect = derive_key_id(key);
    if !env.key_id.is_empty() && env.key_id.to_ascii_lowercase() != expect {
        return Err(format!(
            "envelope keyId {} 与当前密钥（{expect}）不匹配",
            env.key_id
        ));
    }
    if key.len() != 32 {
        return Err(format!("at-rest 密钥长度应为 32 字节，实际 {}", key.len()));
    }
    if env.nonce.len() != 12 {
        return Err(format!("nonce 长度应为 12 字节，实际 {}", env.nonce.len()));
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("GCM 初始化失败: {e}"))?;
    let mut msg = env.ciphertext;
    msg.extend_from_slice(&env.tag);
    let aad = aad(&(env.key_id.to_ascii_lowercase()), env.suite, framing);
    cipher
        .decrypt(Nonce::from_slice(&env.nonce), Payload { msg: &msg, aad: &aad })
        .map_err(|_| "信封解密失败（密钥不匹配或密文被篡改）".to_string())
}

/// 加密封装（仅单测与自检用；客户端写入路径不经过本工具）
#[cfg_attr(not(test), allow(dead_code))]
pub fn seal(key: &[u8], plaintext: &[u8], framing: Framing, nonce: [u8; 12]) -> Result<Value, String> {
    if key.len() != 32 {
        return Err("密钥长度应为 32 字节".into());
    }
    let key_id = derive_key_id(key);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("GCM 初始化失败: {e}"))?;
    let aad = aad(&key_id, DEFAULT_SUITE, framing);
    let out = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad })
        .map_err(|_| "信封加密失败".to_string())?;
    if out.len() < 16 {
        return Err("GCM 输出异常".into());
    }
    let (ct, tag) = out.split_at(out.len() - 16);
    let env = serde_json::json!({
        "suite": DEFAULT_SUITE,
        "keyId": key_id,
        "nonce": B64.encode(nonce),
        "authTag": B64.encode(tag),
        "ciphertext": B64.encode(ct),
    });
    Ok(serde_json::json!({
        ENVELOPE_KEY: 1,
        "envelope": B64.encode(serde_json::to_vec(&env).unwrap_or_default()),
    }))
}

/// 解密一个「可能加密、也可能本来就是明文」的字段。
/// 空值 → Some("")；非信封的非字符串值 → None（调用方自行回退）。
pub fn decrypt_field(key: &[u8], v: &Value) -> Option<String> {
    match v {
        Value::Null => Some(String::new()),
        Value::String(s) if s.is_empty() => Some(String::new()),
        other if is_envelope(other) => open_envelope(key, other, Framing::Field)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

// ── keyblob ────────────────────────────────────────────────────────────────

/// 客户端配置目录（keyblob 所在）：`~/.workbuddy-ai` / `~/.workbuddy`
pub fn config_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for var in ["WORKBUDDY_CONFIG_DIR", "CODEBUDDY_CONFIG_DIR"] {
        if let Ok(p) = std::env::var(var) {
            let p = p.trim();
            if !p.is_empty() {
                out.push(PathBuf::from(p));
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        for name in [".workbuddy-ai", ".workbuddy"] {
            out.push(PathBuf::from(&home).join(name));
        }
    }
    out
}

/// keyblob 路径（固定文件名，钉在配置目录内）
pub fn keyblob_path() -> Option<PathBuf> {
    config_dirs()
        .into_iter()
        .map(|d| d.join(KEYBLOB_NAME))
        .find(|p| p.is_file())
}

fn load_keyblob() -> Option<Value> {
    let p = keyblob_path()?;
    let raw = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&raw).ok()
}

/// keyblob 里 `static-v1` 槽的 protectorKeyId（内存搜钥的判据）
pub fn expected_key_id() -> Option<String> {
    let blob = load_keyblob()?;
    blob.get("slots")?.as_array()?.iter().find_map(|slot| {
        if slot.get("type").and_then(Value::as_str) == Some("static-v1") {
            slot.get("protectorKeyId")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_ascii_lowercase())
        } else {
            None
        }
    })
}

/// 自检：密钥与 keyblob 对得上，且能解出 32 字节主密钥。
///
/// 注意 `wrapped` 的形态（2026-10-18 实测真实 keyblob）：它是**裸 base64(JSON) 字符串**
/// `{"suite":1,"keyId":"…","nonce":"…","authTag":"…","ciphertext":"…"}`，
/// **不带** `$wbEncrypted` 外壳。早期实现按 `is_envelope()` 过滤，会把真实 keyblob 判成
/// 「没有 static-v1 槽」（内存里明明找到了密钥却全部校验失败）。这里接受 string / object
/// 两种形态，交给 `open_envelope` 统一解析。
pub fn validate_key(key: &[u8]) -> Result<(), String> {
    let blob = load_keyblob().ok_or("keyblob 不存在（客户端未安装/未登录过？）")?;
    validate_key_against_blob(key, &blob)
}

/// 纯函数版本（便于单测；`blob` = 客户端 `keyblob` 解析后的 JSON）
fn validate_key_against_blob(key: &[u8], blob: &Value) -> Result<(), String> {
    let slots = blob
        .get("slots")
        .and_then(Value::as_array)
        .ok_or("keyblob 缺 slots")?;
    let expect_id = derive_key_id(key);
    let mut seen_slot = false;
    let mut mismatch = false;
    let mut last_err: Option<String> = None;
    for slot in slots {
        if slot.get("type").and_then(Value::as_str) != Some("static-v1") {
            continue;
        }
        let Some(wrapped) = slot.get("wrapped") else { continue };
        // 裸 base64 字符串 或 {"$wbEncrypted":1,"envelope":…} 均可
        let usable = match wrapped {
            Value::String(s) => !s.trim().is_empty(),
            Value::Object(_) => is_envelope(wrapped),
            _ => false,
        };
        if !usable {
            continue;
        }
        seen_slot = true;
        let key_id = slot
            .get("protectorKeyId")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase());
        if key_id.as_deref() != Some(expect_id.as_str()) {
            // 客户端可能保留多个槽（轮换后的新旧钥共存）：继续试下一个槽，
            // 而不是在第一个不匹配的槽上直接判死
            mismatch = true;
            continue;
        }
        let master = match open_envelope(key, wrapped, Framing::File) {
            Ok(m) => m,
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
        if master.len() != 32 {
            last_err = Some(format!("keyblob 解出的主密钥长度异常（{}）", master.len()));
            continue;
        }
        if let Some(expect) = blob.get("keyId").and_then(Value::as_str) {
            if derive_key_id(&master) != expect.trim().to_ascii_lowercase() {
                last_err = Some("keyblob 解出的主密钥校验失败".into());
                continue;
            }
        }
        return Ok(());
    }
    Err(if let Some(e) = last_err {
        e
    } else if mismatch {
        "密钥与 keyblob 的 protectorKeyId 不匹配".into()
    } else if seen_slot {
        "keyblob 里没有可用的 static-v1 槽".into()
    } else {
        "keyblob 里没有 static-v1 槽（结构可能已升级）".into()
    })
}

// ── 密钥获取 ───────────────────────────────────────────────────────────────

static CACHE: Mutex<Option<Vec<u8>>> = Mutex::new(None);

fn parse_key_text(text: &str) -> Option<Vec<u8>> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let t = t.strip_prefix("0x").unwrap_or(t);
    // hex（32 字节 = 64 字符）
    if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut out = Vec::with_capacity(32);
        for i in (0..64).step_by(2) {
            out.push(u8::from_str_radix(&t[i..i + 2], 16).ok()?);
        }
        return Some(out);
    }
    // base64（32 字节 = 44 字符，末位 '='）
    B64.decode(t).ok().filter(|b| b.len() == 32)
}

/// 人工粘贴的密钥 / 环境变量（两处「离线可用」来源）
pub fn manual_key(state_data_dir: Option<&std::path::Path>) -> Option<Vec<u8>> {
    for var in ["AIWORK_WB_ATREST_KEY", "WORKBUDDY_ATREST_KEY"] {
        if let Ok(v) = std::env::var(var) {
            if let Some(k) = parse_key_text(&v) {
                return Some(k);
            }
        }
    }
    let dir = state_data_dir?;
    let path = dir.join("wb_atrest.key");
    let text = std::fs::read_to_string(path).ok()?;
    parse_key_text(&text)
}

fn cached_key() -> Option<Vec<u8>> {
    let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.clone().filter(|k| validate_key(k).is_ok())
}

fn remember(key: Vec<u8>) {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(key);
}

/// 失败记忆（同一 keyId 60 秒内不重复扫进程内存）：一次扫描会读多份凭据文件，
/// 密钥拿不到时若每份都触发整段内存扫描，界面会长时间无响应（实测日志里
/// 连续 4 条「未通过 keyblob 自检」即此放大）。
static FAIL_MEMO: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);

/// 取 at-rest 密钥：缓存 → 人工来源 → 进程内存扫描。失败返回可读原因。
pub fn recover_key(
    state_data_dir: Option<&std::path::Path>,
    log: &dyn Fn(&str),
) -> Result<Vec<u8>, String> {
    if let Some(k) = cached_key() {
        return Ok(k);
    }
    let key_id = expected_key_id();
    // 失败记忆命中：直接给「稍后再试」而不是再扫一遍
    if let Some(kid) = key_id.as_deref() {
        let memo = FAIL_MEMO
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        if let Some((id, at)) = memo {
            if id == kid && at.elapsed() < std::time::Duration::from_secs(60) {
                return Err(
                    "无法取得桌面端 at-rest 密钥：最近一次尝试未成功（60 秒内不重复扫描）——\
                     请先启动 WorkBuddy / CodeBuddy 桌面端并保持运行后重试"
                        .to_string(),
                );
            }
        }
    }
    for (label, candidate) in [
        ("环境变量/本地密钥文件", manual_key(state_data_dir)),
        ("客户端进程内存", scan_client_memory(key_id.as_deref(), log)),
    ] {
        let Some(candidate) = candidate else { continue };
        if let Err(e) = validate_key(&candidate) {
            log(&format!("[wb-atrest] {label} 的密钥未通过 keyblob 自检：{e}"));
            continue;
        }
        log(&format!(
            "[wb-atrest] 已取到桌面端 at-rest 密钥（来源：{label}，keyId={}）",
            derive_key_id(&candidate)
        ));
        remember(candidate.clone());
        if let Ok(mut memo) = FAIL_MEMO.lock() {
            *memo = None;
        }
        return Ok(candidate);
    }
    if let (Some(kid), Ok(mut memo)) = (key_id.clone(), FAIL_MEMO.lock()) {
        *memo = Some((kid, std::time::Instant::now()));
    }
    let hint = if keyblob_path().is_none() {
        "未找到客户端 keyblob（WorkBuddy 桌面端未安装/未登录过）".to_string()
    } else if cfg!(windows) {
        "请先启动 WorkBuddy / CodeBuddy 桌面端并保持运行，再重试扫描（密钥只在客户端进程内存里）"
            .to_string()
    } else {
        "非 Windows 无法从进程内存回收密钥，请在数据目录放 wb_atrest.key（hex/base64）".to_string()
    };
    Err(format!("无法取得桌面端 at-rest 密钥：{hint}"))
}

/// 进程内已有可用密钥？（扫描预览用：有钥即可直接解密，无需再点一次）
#[cfg_attr(not(test), allow(dead_code))]
pub fn has_cached_key() -> bool {
    cached_key().is_some()
}

// ── 客户端进程内存扫描（Windows）───────────────────────────────────────────

#[cfg(windows)]
fn scan_client_memory(key_id: Option<&str>, log: &dyn Fn(&str)) -> Option<Vec<u8>> {
    let key_id = key_id?;
    let pids = client_pids();
    if pids.is_empty() {
        log("[wb-atrest] 未发现运行中的 WorkBuddy/CodeBuddy 桌面端进程");
        return None;
    }
    // 时间预算：V8 主堆可达数百 MB ~ 1GB，逐块读取 + 扫描在慢盘上可能数十秒；
    // 这是用户手动触发的操作，但仍给一个上限，避免界面长时间无响应
    //（实测 release 单进程 1.2GB 约 8s 命中，45s 覆盖慢盘/多进程）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    for pid in &pids {
        if let Some(key) = scan_pid(*pid, key_id, deadline) {
            return Some(key);
        }
        if std::time::Instant::now() >= deadline {
            log("[wb-atrest] 内存扫描已达时间上限（45s），未命中；可关闭客户端后重试或手工提供密钥");
            return None;
        }
    }
    log(&format!(
        "[wb-atrest] 已在 {} 个客户端进程中搜索 at-rest 密钥，未命中（客户端版本可能更换了密钥载荷）",
        pids.len()
    ));
    None
}

#[cfg(not(windows))]
fn scan_client_memory(_key_id: Option<&str>, _log: &dyn Fn(&str)) -> Option<Vec<u8>> {
    None
}

/// 客户端主进程 PID（按工作集倒序，只取同名进程）
#[cfg(windows)]
fn client_pids() -> Vec<u32> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut v: Vec<(u64, u32)> = sys
        .processes()
        .iter()
        .filter_map(|(pid, p)| {
            let name = p.name().to_ascii_lowercase();
            if name == "workbuddy.exe" || name == "workbuddyai.exe" {
                Some((p.memory(), pid.as_u32()))
            } else {
                None
            }
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, pid)| pid).take(3).collect()
}

#[cfg(windows)]
fn scan_pid(pid: u32, key_id: &str, deadline: std::time::Instant) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Memory::{
        VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_READONLY,
        PAGE_READWRITE, PAGE_WRITECOPY, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
        PAGE_EXECUTE_WRITECOPY,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    const CHUNK: usize = 4 << 20;
    const OVERLAP: usize = 64;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }
        let readable = PAGE_READONLY
            | PAGE_READWRITE
            | PAGE_WRITECOPY
            | PAGE_EXECUTE_READ
            | PAGE_EXECUTE_READWRITE
            | PAGE_EXECUTE_WRITECOPY;
        let mut addr: usize = 0;
        let mut carry: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            if std::time::Instant::now() >= deadline {
                break;
            }
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            let got = VirtualQueryEx(
                handle,
                addr as *const core::ffi::c_void,
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            );
            if got == 0 {
                break;
            }
            let base = mbi.BaseAddress as usize;
            let size = mbi.RegionSize;
            if size == 0 {
                break;
            }
            let ok = mbi.State == MEM_COMMIT && (mbi.Protect & readable) != 0 && (mbi.Protect & PAGE_GUARD) == 0;
            if ok {
                let mut off = 0usize;
                while off < size {
                    let want = CHUNK.min(size - off);
                    let mut read = 0usize;
                    let dst = buf.as_mut_ptr() as *mut core::ffi::c_void;
                    let rc = ReadProcessMemory(
                        handle,
                        (base + off) as *const core::ffi::c_void,
                        dst,
                        want,
                        &mut read,
                    );
                    if rc != 0 && read > 0 {
                        carry.extend_from_slice(&buf[..read]);
                        if let Some(key) = find_key_in_bytes(&carry, key_id) {
                            CloseHandle(handle);
                            return Some(key);
                        }
                        // 只保留尾部一段防跨块截断（token 44 字符，UTF-16 翻倍）
                        if carry.len() > OVERLAP {
                            let keep = carry.len() - OVERLAP;
                            carry.drain(..keep);
                        }
                    } else if read == 0 {
                        // 整块不可读：跳过（不中断整段扫描）
                        carry.clear();
                    }
                    off += want;
                }
            }
            let next = base.saturating_add(size);
            if next <= addr {
                break;
            }
            addr = next;
        }
        CloseHandle(handle);
    }
    None
}

/// 在内存字节里找「44 字符规范 base64」候选并验证（ASCII 与 UTF-16LE 两种排布）
fn find_key_in_bytes(bytes: &[u8], key_id: &str) -> Option<Vec<u8>> {
    const N: usize = 44;
    fn is_b64(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
    }
    // ASCII 排布
    let mut i = 0;
    while i + N <= bytes.len() {
        if bytes[i + N - 1] == b'=' && bytes[i..i + N - 1].iter().all(|b| is_b64(*b)) {
            let prev_ok = i == 0 || !is_b64(bytes[i - 1]);
            let next_ok = i + N >= bytes.len() || !is_b64(bytes[i + N]);
            if prev_ok && next_ok {
                if let Ok(cand) = B64.decode(&bytes[i..i + N]) {
                    if cand.len() == 32 && derive_key_id(&cand) == key_id {
                        return Some(cand);
                    }
                }
            }
            i += N;
        } else {
            i += 1;
        }
    }
    // UTF-16LE 排布（JS 字符串）
    let mut j = 0;
    while j + N * 2 <= bytes.len() {
        let eq = bytes[j + (N - 1) * 2] == b'=' && bytes[j + (N - 1) * 2 + 1] == 0;
        let body_ok = (0..N - 1).all(|k| is_b64(bytes[j + k * 2]) && bytes[j + k * 2 + 1] == 0);
        if eq && body_ok {
            let mut raw = Vec::with_capacity(N);
            for k in 0..N {
                raw.push(bytes[j + k * 2]);
            }
            if let Ok(cand) = B64.decode(&raw) {
                if cand.len() == 32 && derive_key_id(&cand) == key_id {
                    return Some(cand);
                }
            }
            j += N * 2;
        } else {
            j += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> Vec<u8> {
        (0..32u8).map(|i| i.wrapping_mul(7).wrapping_add(n)).collect()
    }

    #[test]
    fn key_id_is_sha256_prefix16() {
        let k = key(1);
        let id = derive_key_id(&k);
        assert_eq!(id.len(), 16);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn aad_layout_matches_client_contract() {
        let a = aad("0123456789abcdef", 1, Framing::Field);
        // "WB-AAD\0" | 0x01 | LP(WBEV1) | LP(sym-v1) | u32(1) | LP(keyId) | 2 | 0 | 0
        assert_eq!(&a[..7], b"WB-AAD\0");
        assert_eq!(a[7], 0x01);
        assert_eq!(&a[8..12], &5u32.to_be_bytes());
        assert_eq!(&a[12..17], b"WBEV1");
        assert_eq!(&a[17..21], &6u32.to_be_bytes());
        assert_eq!(&a[21..27], b"sym-v1");
        assert_eq!(&a[27..31], &1u32.to_be_bytes());
        assert_eq!(&a[31..35], &16u32.to_be_bytes());
        assert_eq!(&a[35..51], b"0123456789abcdef");
        assert_eq!(a[51], 2);
        assert_eq!(&a[52..], &[0u8, 0u8]);
    }

    #[test]
    fn seal_open_roundtrip_field_and_file() {
        let k = key(3);
        for framing in [Framing::Field, Framing::File, Framing::Record, Framing::Stream] {
            let env = seal(&k, br#"{"accessToken":"dt-abc"}"#, framing, [7u8; 12]).unwrap();
            assert!(is_envelope(&env));
            let out = open_envelope(&k, &env, framing).unwrap();
            assert_eq!(out, br#"{"accessToken":"dt-abc"}"#);
        }
    }

    #[test]
    fn open_rejects_wrong_key_and_wrong_framing() {
        let k = key(4);
        let env = seal(&k, b"token", Framing::Field, [1u8; 12]).unwrap();
        assert!(open_envelope(&key(5), &env, Framing::Field).is_err());
        assert!(open_envelope(&k, &env, Framing::Record).is_err());
    }

    #[test]
    fn decrypt_field_passthrough_plaintext() {
        let k = key(6);
        assert_eq!(decrypt_field(&k, &Value::String("dt-plain".into())), Some("dt-plain".into()));
        assert_eq!(decrypt_field(&k, &Value::Null), Some(String::new()));
        let env = seal(&k, "好多愚".as_bytes(), Framing::Field, [9u8; 12]).unwrap();
        assert_eq!(decrypt_field(&k, &env), Some("好多愚".into()));
    }

    #[test]
    fn plaintext_token_is_not_envelope() {
        assert!(!is_envelope(&Value::String("eyJhbGciOiJSUzI1NiJ9.abc".into())));
        assert!(!is_envelope(&Value::Number(1.into())));
        assert!(is_envelope(&serde_json::json!({"$wbEncrypted": 1, "envelope": "abc"})));
    }

    /// 跨实现对齐（参考实现 `workbuddy2api-hub/wb-proxy/wb_atrest.py` 生成的真实信封）：
    /// AAD 逐字节对齐才有意义——Rust↔Rust 自洽不能证明与客户端一致，这里用 Python 侧
    /// `seal()` 产出的信封反向验证我们的 `open_envelope`。
    #[test]
    fn opens_envelope_sealed_by_reference_python() {
        let key_hex = "0b121920272e353c434a51585f666d747b828990979ea5acb3bac1c8cfd6dde4";
        let key: Vec<u8> = (0..key_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&key_hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(derive_key_id(&key), "626c4ed5cb6893c7");

        // framing=field：`{"accessToken": {...}}` 这种字段级信封
        let field = serde_json::json!({
            "$wbEncrypted": 1,
            "envelope": "eyJzdWl0ZSI6MSwia2V5SWQiOiI2MjZjNGVkNWNiNjg5M2M3Iiwibm9uY2UiOiJBQUVDQXdRRkJnY0lDUW9MIiwiYXV0aFRhZyI6InhhVE5DNk5YTG5VSGhyU1F2TzdwcUE9PSIsImNpcGhlcnRleHQiOiJnNi9iSWEwdGdPRDhyRnNGbFY5TG5ycz0ifQ=="
        });
        assert_eq!(
            decrypt_field(&key, &field).as_deref(),
            Some("dt-intl-token-xyz")
        );

        // framing=file：keyblob 里 `wrapped` 槽的同构信封
        let file_env = serde_json::json!({
            "$wbEncrypted": 1,
            "envelope": "eyJzdWl0ZSI6MSwia2V5SWQiOiI2MjZjNGVkNWNiNjg5M2M3Iiwibm9uY2UiOiJEQTBPRHhBUkVoTVVGUllYIiwiYXV0aFRhZyI6IjJVZTdEd3dnZ2ZPc3hBTTJTTzNleHc9PSIsImNpcGhlcnRleHQiOiI3eWJaODNUNzROcEpmZ0trbXR4RkN2VTZiOUZDeFB6M1pNbVo4RlRSZFVnPSJ9"
        });
        let plain = open_envelope(&key, &file_env, Framing::File).unwrap();
        assert_eq!(plain, b"master-key-32-bytes-padding-pad!");
    }

    /// 真实 keyblob 形态（2026-10-18 实测）：`slots[].wrapped` 是**裸 base64(JSON)**，
    /// 不带 `$wbEncrypted` 外壳。早期按 is_envelope 过滤会把真 keyblob 判成「无 static-v1 槽」，
    /// 导致内存里明明找到密钥却全部自检失败（用户日志中的根因）。
    #[test]
    fn validate_key_accepts_bare_base64_wrapped_slot() {
        let at_rest = key(21);
        let master = key(22);
        // 客户端写入形态：envelope 的 base64 直接作为 wrapped 字符串
        let env = seal(&at_rest, &master, Framing::File, [5u8; 12]).unwrap();
        let bare = env
            .get("envelope")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let blob = serde_json::json!({
            "version": 1,
            "keyId": derive_key_id(&master),
            "slots": [{
                "type": "static-v1",
                "protectorKeyId": derive_key_id(&at_rest),
                "wrapped": bare,
            }]
        });
        assert!(validate_key_against_blob(&at_rest, &blob).is_ok(), "裸 base64 wrapped 必须能自检");
        // 错误密钥：明确报 protectorKeyId 不匹配（而不是「没有槽」）
        let err = validate_key_against_blob(&key(23), &blob).unwrap_err();
        assert!(err.contains("protectorKeyId"), "{err}");

        // 对象形态（$wbEncrypted 外壳）同样接受
        let blob2 = serde_json::json!({
            "keyId": derive_key_id(&master),
            "slots": [{
                "type": "static-v1",
                "protectorKeyId": derive_key_id(&at_rest),
                "wrapped": env.clone(),
            }]
        });
        assert!(validate_key_against_blob(&at_rest, &blob2).is_ok());

        // 多槽（客户端轮换后新旧钥共存）：第一个槽对不上也不判死，继续试后面的槽
        let blob3 = serde_json::json!({
            "keyId": derive_key_id(&master),
            "slots": [
                {"type": "static-v1", "protectorKeyId": derive_key_id(&key(99)), "wrapped": bare},
                {"type": "static-v1", "protectorKeyId": derive_key_id(&at_rest),
                 "wrapped": env.get("envelope").cloned().unwrap_or(Value::Null)},
            ]
        });
        assert!(validate_key_against_blob(&at_rest, &blob3).is_ok());
    }

    #[test]
    fn validate_key_reports_missing_slot_shape() {
        let err = validate_key_against_blob(&key(1), &serde_json::json!({"slots": []}))
            .unwrap_err();
        assert!(err.contains("没有 static-v1 槽"), "{err}");
    }

    /// 真机探针（默认忽略）：`cargo test probe_recover_key_and_decrypt -- --ignored --nocapture`
    /// 需要 WorkBuddy 桌面端**正在运行**。验证「进程内存回收密钥 → keyblob 自检 → 解开
    /// 真实 `.info` 的 accessToken / nickname」整链（token 只打印长度与前 6 位）。
    #[test]
    #[ignore]
    fn probe_recover_key_and_decrypt() {
        let dir = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
            .join("CodeBuddyExtension")
            .join("Data")
            .join("Public")
            .join("auth");
        let path = dir.join("workbuddy-desktop.info");
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let key = recover_key(None, &|m| println!("{m}")).expect("应能从运行中的客户端取到密钥");
        println!("keyId={}", derive_key_id(&key));
        let token = fs_decrypt(&key, &raw, &["accessToken"]);
        println!("tokenLen={} prefix={:?}", token.len(), token.chars().take(6).collect::<String>());
        assert!(token.starts_with("eyJ"), "accessToken 应解出 JWT");
        let nick = fs_decrypt(&key, &raw, &["nickname"]);
        println!("nickname={nick:?}");
        assert!(!nick.is_empty());

        // 顺带验证「国区积分能不能取到 / 是多少」：用解出的真凭证直连计费端点，
        // 余额以服务端 TotalCredits 为准（与 wb_credits::balance_from_summary 同口径）
        let uid = crate::fs_utils::dig(&raw, &["uid"])
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let domain = crate::fs_utils::dig(&raw, &["domain"])
            .and_then(Value::as_str)
            .unwrap_or("www.workbuddy.cn")
            .to_string();
        let creds = crate::tasks::wb_common::Creds {
            access_token: token,
            uid,
            domain: domain.clone(),
            ..Default::default()
        };
        let headers = crate::tasks::wb_common::build_auth_headers(&creds, true);
        let base = crate::tasks::wb_common::region_billing_base(&domain);
        let url = format!("{base}/billing/meter/get-user-resource-summary");
        let agent = crate::tasks::http_agent(30);
        let (status, body) = crate::tasks::wb_common::post_json(&agent, &url, &headers, &serde_json::json!({}));
        println!("billing summary status={status}");
        if let Some(b) = body {
            let total = b
                .get("data")
                .and_then(|d| d.get("TotalCredits"))
                .and_then(Value::as_str)
                .unwrap_or("(缺 TotalCredits)");
            println!("TotalCredits={total}");
        }
    }

    /// 探针用：按 dig 的键序取一个字段并解密（与 `commands::workbuddy::common` 同口径）
    #[cfg(test)]
    fn fs_decrypt(key: &[u8], raw: &Value, keys: &[&str]) -> String {
        let v = crate::fs_utils::dig(raw, keys);
        match v {
            Some(v) => decrypt_field(key, v).unwrap_or_default(),
            None => String::new(),
        }
    }

    #[test]
    fn find_key_in_ascii_and_utf16_memory() {        let k = key(8);
        let b64 = B64.encode(&k);
        assert_eq!(b64.len(), 44);
        let id = derive_key_id(&k);
        let mut mem = b"junk junk junk ".to_vec();
        mem.extend_from_slice(b64.as_bytes());
        mem.extend_from_slice(b" tail");
        assert_eq!(find_key_in_bytes(&mem, &id), Some(k.clone()));

        let mut utf16 = b"xx".to_vec();
        for ch in b64.as_bytes() {
            utf16.push(*ch);
            utf16.push(0);
        }
        assert_eq!(find_key_in_bytes(&utf16, &id), Some(k));
    }
}
