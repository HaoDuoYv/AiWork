//! 静态加密信封（合并增强：源自 `workbuddy2api-hub / wb_atrest.py`）
//!
//! ## 用途
//! 「智能体接入」导出包、成本/活动导出、以及未来任何需要落到**用户可见文件**的
//! 载荷，都通过本模块封装成自描述信封，而不是裸 JSON：
//! - 密钥派生：`key = sha256(seed)`（32 字节），`key_id = hex(sha256(key))[..16]`，
//!   信封只带 `key_id`，解密端先校验 `key_id` 再解密（错误口令快速失败）；
//! - 算法：AES-256-GCM，12 字节随机 nonce，认证标签随密文；
//! - **AAD 逐字段绑定**：版本/信封类型/派生方案/代数/`key_id`/业务码全部进 AAD，
//!   任何字段被改写（例如把 `code` 从 `AGENT_BUNDLE` 改成 `ACCOUNT_EXPORT`）都会
//!   解密失败，而不是"换个壳照样能读"。
//!
//! ## 与 hub 的对应关系
//! hub 的 AAD 为 `"WB-AAD\0"|0x01|LP("WBEF1")|LP("sym-v1")|u32(1)|LP(keyId)|code|00|00`；
//! 本模块改为 `"AWAAD\0"` / `AWEF1` 前缀（跨产品不得互通，避免一份密文被另一产品
//! 误读），其余编码规则（4 字节大端长度前缀、双 `00` 尾巴）保持一致，便于对照审计。
//!
//! ## 红线
//! - 口令/种子**不落盘**；只有调用方持有的 `seed` 能解开；
//! - 解密失败不区分「口令错」与「密文被改」以外的细节，避免 oracle 泄露。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

/// 信封魔数（绑定产品，跨产品不互通）
const MAGIC: &[u8] = b"AWAAD\0";
/// 信封类型
const ENV_KIND: &str = "AWEF1";
/// 派生方案
const KDF_NAME: &str = "sym-v1";
/// 信封代数
const GEN: u32 = 1;
/// nonce 长度
const NONCE_LEN: usize = 12;

/// 加密信封（可直接 JSON 落盘/传给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub alg: String,
    pub kdf: String,
    pub key_id: String,
    /// 业务码（进 AAD，用于隔离不同用途的密文）
    pub code: String,
    /// base64 标准编码 nonce
    pub nonce: String,
    /// base64 标准编码密文（含 GCM tag）
    pub ct: String,
    /// 人读提示
    #[serde(default)]
    pub hint: String,
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn derive_key(seed: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

fn key_id_of(key: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(key);
    let hex = format!("{:x}", h.finalize());
    hex.chars().take(16).collect()
}

/// 4 字节大端长度前缀
fn lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// 构造 AAD（与 hub 同构：魔数 | 0x01 | LP(kind) | LP(kdf) | u32(gen) | LP(key_id) | code | 00 | 00）
fn build_aad(key_id: &str, code: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(MAGIC);
    out.push(0x01);
    lp(&mut out, ENV_KIND.as_bytes());
    lp(&mut out, KDF_NAME.as_bytes());
    out.extend_from_slice(&GEN.to_be_bytes());
    lp(&mut out, key_id.as_bytes());
    out.extend_from_slice(code.as_bytes());
    out.push(0x00);
    out.push(0x00);
    out
}

fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn unb64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| format!("base64 解码失败: {e}"))
}

/// 加密：返回信封（`code` 建议用大写下划线常量，如 `AGENT_BUNDLE`）
pub fn seal(plaintext: &str, seed: &str, code: &str) -> Result<Envelope, String> {
    if seed.trim().is_empty() {
        return Err("加密种子不能为空".to_string());
    }
    let key = derive_key(seed);
    let key_id = key_id_of(&key);
    let code = if code.trim().is_empty() {
        "GENERIC"
    } else {
        code.trim()
    };
    // nonce：uuid v4 前 12 字节（uuid 由 OS CSPRNG 提供）
    let uuid_bytes = uuid::Uuid::new_v4().into_bytes();
    let nonce_bytes: [u8; NONCE_LEN] = uuid_bytes[..NONCE_LEN]
        .try_into()
        .map_err(|_| "nonce 生成失败".to_string())?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("加密器初始化失败: {e}"))?;
    let aad = build_aad(&key_id, code);
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| "加密失败（AES-GCM 拒绝）".to_string())?;
    Ok(Envelope {
        v: GEN,
        alg: "AES-256-GCM".to_string(),
        kdf: KDF_NAME.to_string(),
        key_id,
        code: code.to_string(),
        nonce: b64(&nonce_bytes),
        ct: b64(&ct),
        hint: format!("sealed@{}", now_ts()),
    })
}

/// 解密：`key_id` 不匹配（口令错）或 AAD/密文被改 → Err
pub fn open(env: &Envelope, seed: &str) -> Result<String, String> {
    if env.v != GEN {
        return Err(format!("信封代数不支持：v={}（本端仅支持 v={GEN}）", env.v));
    }
    if env.alg != "AES-256-GCM" || env.kdf != KDF_NAME {
        return Err(format!("信封算法不支持：alg={} kdf={}", env.alg, env.kdf));
    }
    if seed.trim().is_empty() {
        return Err("解密种子不能为空".to_string());
    }
    let key = derive_key(seed);
    let expect = key_id_of(&key);
    if env.key_id != expect {
        return Err("口令不匹配（key_id 校验失败）".to_string());
    }
    let nonce_bytes = unb64(&env.nonce)?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err("nonce 长度非法".to_string());
    }
    let ct = unb64(&env.ct)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("解密器初始化失败: {e}"))?;
    let aad = build_aad(&env.key_id, &env.code);
    let pt = cipher
        .decrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: &ct,
                aad: &aad,
            },
        )
        .map_err(|_| "解密失败（密文或元数据被篡改）".to_string())?;
    String::from_utf8(pt).map_err(|e| format!("明文不是合法 UTF-8: {e}"))
}

/// 口令强度提示（不阻断，仅 UI 提示；口令人为设置时给出最低要求）
pub fn seed_strength(seed: &str) -> &'static str {
    let n = seed.chars().count();
    if n < 8 {
        "weak"
    } else if n < 16 {
        "ok"
    } else {
        "strong"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_key_id_stable() {
        let env = seal(r#"{"a":1,"中文":"值"}"#, "correct horse battery", "AGENT_BUNDLE").unwrap();
        assert_eq!(env.key_id.len(), 16);
        assert_eq!(env.alg, "AES-256-GCM");
        let pt = open(&env, "correct horse battery").unwrap();
        assert!(pt.contains("中文"));
        // 同种子同 code → key_id 稳定
        let env2 = seal("x", "correct horse battery", "AGENT_BUNDLE").unwrap();
        assert_eq!(env.key_id, env2.key_id);
        // nonce 随机
        assert_ne!(env.nonce, env2.nonce);
    }

    #[test]
    fn wrong_seed_fails_fast() {
        let env = seal("secret", "seed-one-1234567", "AGENT_BUNDLE").unwrap();
        let err = open(&env, "seed-two-1234567").unwrap_err();
        assert!(err.contains("口令不匹配"), "{err}");
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut env = seal("secret", "seed-one-1234567", "AGENT_BUNDLE").unwrap();
        let mut bytes = unb64(&env.ct).unwrap();
        bytes[0] ^= 0xff;
        env.ct = b64(&bytes);
        assert!(open(&env, "seed-one-1234567").is_err());
    }

    #[test]
    fn code_is_bound_by_aad() {
        let mut env = seal("secret", "seed-one-1234567", "AGENT_BUNDLE").unwrap();
        env.code = "ACCOUNT_EXPORT".to_string();
        let err = open(&env, "seed-one-1234567").unwrap_err();
        assert!(err.contains("篡改"), "换 code 必须解密失败：{err}");
    }

    #[test]
    fn empty_seed_rejected() {
        assert!(seal("x", "  ", "C").is_err());
        let env = seal("x", "ok-seed-1234", "C").unwrap();
        assert!(open(&env, "").is_err());
    }

    #[test]
    fn envelope_serializes_as_json() {
        let env = seal("x", "ok-seed-1234", "C").unwrap();
        let text = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&text).unwrap();
        assert_eq!(open(&back, "ok-seed-1234").unwrap(), "x");
    }
}
