//! 一键导入其他智能体配置（三项目合并新增强制能力）
//!
//! ## 定位
//! 原 TraeWorkAssistant 只有「把本网关注册进 CC Switch」的单向能力
//! （`commands/ccswitch.rs`）；本模块补齐**反向**能力：扫描本机其他
//! AI 编码智能体 / 客户端 / 兄弟网关的配置，把其中的
//! 「OpenAI 兼容上游（base_url + api_key + 模型名）」与「MCP server 清单」
//! 一键并入本应用资源池（`api_server::custom_models` + `api_server::api_keys`
//! + kv `agent_mcp_registry`），并把本网关**写回**目标智能体配置。
//!
//! ## 覆盖对象（内置注册表 `AGENTS`）
//! Claude Code / Codex CLI / Gemini CLI / Qwen Code / OpenCode / Continue /
//! Cline / Roo Code / Cursor / Windsurf / Cherry Studio / Chatbox / Aider，
//! 另支持「额外目录」扫描（自动识别 qoder2api-hub / workbuddy2api-hub 这类
//! 兄弟网关的 settings / 模型目录 / 账号文件）。
//!
//! ## 红线
//! - 只读识别 + **写前备份**（`<file>.bak_aiwork_<ts>`），绝不静默覆盖；
//! - API Key 落库后前端只回显掩码（`sk-****abcd`），完整值仅网关内部使用；
//! - 同名模型 canonical 冲突默认跳过（`overwrite=false`），不做隐式改道；
//! - 导入历史写 kv `agent_import_history`（cap 50），可回溯。
//!
//! ## 与兄弟项目的关系（合并说明）
//! - qoder2api-hub 的 `accounts/*.json` / `qoder_catalog_cn.json`、
//!   workbuddy2api-hub 的 `settings.json` / `catalog/*.json` 均可在「额外目录」
//!   扫描中体识别为「兄弟网关上游 + 模型目录」，直接并入自定义模型池；
//! - 两个 hub 的账号池（PAT / cookie）不在本模块直接接管，避免与
//!   `commands/qoder/*`、`commands/workbuddy/*` 的账号体系产生两份真源。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api_server::{api_keys, custom_models};
use crate::state::AppState;

// ==================== 常量 ====================

/// 导入历史 kv 键（cap 50）
const KV_HISTORY: &str = "agent_import_history";
/// 统一 MCP 注册表 kv 键（跨智能体合并去重）
const KV_MCP: &str = "agent_mcp_registry";
/// 历史保留条数
const HISTORY_CAP: usize = 50;
/// 写回时使用的 provider id（固定值，便于幂等覆盖自身条目）
const GW_PROVIDER_ID: &str = "aiwork";
/// 写回时使用的 provider 展示名
const GW_PROVIDER_NAME: &str = "AI Work 中枢网关";
/// 写回专用 API Key 名（复用或新建，不限每日配额）
const GW_KEY_NAME: &str = "智能体接入专用";

// ==================== 小工具 ====================

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .filter(|h| !h.trim().is_empty())
        .map(PathBuf::from)
}

fn appdata_dir() -> Option<PathBuf> {
    std::env::var("APPDATA")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .map(PathBuf::from)
}

fn localappdata_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .map(PathBuf::from)
}

/// 解析注册表里的路径模板：`~/x`、`%APPDATA%/x`、`%LOCALAPPDATA%/x`
fn resolve_path(spec: &str) -> Option<PathBuf> {
    let spec = spec.trim();
    let (base, rel) = if let Some(rest) = spec.strip_prefix("~/") {
        (home_dir()?, rest)
    } else if let Some(rest) = spec.strip_prefix("%APPDATA%/") {
        (appdata_dir()?, rest)
    } else if let Some(rest) = spec.strip_prefix("%LOCALAPPDATA%/") {
        (localappdata_dir()?, rest)
    } else if let Some(rest) = spec.strip_prefix("%USERPROFILE%/") {
        (home_dir()?, rest)
    } else {
        (PathBuf::from(""), spec)
    };
    Some(base.join(rel))
}

fn read_text(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn read_json(p: &Path) -> Option<Value> {
    let text = read_text(p)?;
    serde_json::from_str::<Value>(&text).ok()
}

/// Key 掩码：保留前 4 / 后 4，长度不足 10 时全掩码
fn mask_key(k: &str) -> String {
    let k = k.trim();
    if k.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = k.chars().collect();
    if chars.len() <= 10 {
        return "****".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}****{tail}")
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    let s = s.trim();
    if !s.is_empty() && !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

/// 路径转展示串（统一正斜杠，避免 Windows 反斜杠在 UI 里被当转义）
fn pshow(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// 字符串值取出（trim 后非空才算有值）
fn sval<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(Value::as_str) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

/// 键名不区分大小写/下划线的模糊取字符串（各家配置键风格不一）
fn sval_fuzzy(v: &Value, needles: &[&str]) -> Option<String> {
    let obj = v.as_object()?;
    let norm = |s: &str| s.to_ascii_lowercase().replace(['_', '-'], "");
    for (k, val) in obj {
        let nk = norm(k);
        for needle in needles {
            if nk == norm(needle) {
                if let Some(s) = val.as_str() {
                    let t = s.trim();
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

// ==================== YAML 极简解析 ====================
//
// Continue / Aider 的配置有 YAML 形态。引入完整 YAML 依赖不划算（体积 +
// 供应链），这里实现**本模块够用**的子集：缩进映射、`- ` 序列、`k: v` 标量、
// 引号剥离、true/false/数字/null 归一。不支持锚点/多行块/流式写法——遇到
// 这些形态按标量原样保留，调用方另有 JSON 兜底。

struct YLine {
    indent: usize,
    content: String,
}

fn split_kv(s: &str) -> Option<(String, String)> {
    let bytes: Vec<char> = s.chars().collect();
    let mut in_s = false;
    let mut in_d = false;
    for (i, c) in bytes.iter().enumerate() {
        match c {
            '\'' => in_s = !in_s,
            '"' => in_d = !in_d,
            ':' if !in_s && !in_d => {
                let k: String = bytes[..i].iter().collect();
                let v: String = bytes[i + 1..].iter().collect();
                let k = k.trim().trim_matches('"').trim_matches('\'').to_string();
                if k.is_empty() {
                    return None;
                }
                return Some((k, v.trim().to_string()));
            }
            _ => {}
        }
    }
    None
}

fn yaml_scalar(s: &str) -> Value {
    let t = s.trim();
    if t.is_empty() {
        return Value::Null;
    }
    let unq = t.trim_matches('"').trim_matches('\'');
    match unq {
        "true" | "True" | "yes" => return Value::Bool(true),
        "false" | "False" | "no" => return Value::Bool(false),
        "null" | "~" | "None" => return Value::Null,
        _ => {}
    }
    if let Ok(i) = unq.parse::<i64>() {
        return Value::from(i);
    }
    if let Ok(f) = unq.parse::<f64>() {
        if unq.contains('.') {
            return serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null);
        }
    }
    Value::String(unq.to_string())
}

fn yaml_parse_block(lines: &[YLine], idx: &mut usize, indent: usize) -> Value {
    let is_seq = lines
        .get(*idx)
        .map(|l| l.indent == indent && l.content.starts_with('-'))
        .unwrap_or(false);
    if is_seq {
        let mut out: Vec<Value> = Vec::new();
        while let Some(l) = lines.get(*idx) {
            if l.indent != indent || !l.content.starts_with('-') {
                break;
            }
            let rest = l.content[1..].trim().to_string();
            *idx += 1;
            if rest.is_empty() {
                let child = lines.get(*idx).map(|n| n.indent).unwrap_or(indent + 2);
                out.push(yaml_parse_block(lines, idx, child));
                continue;
            }
            if let Some((k, v)) = split_kv(&rest) {
                let mut map = Map::new();
                if v.is_empty() {
                    let child = lines.get(*idx).map(|n| n.indent).unwrap_or(indent + 2);
                    if lines.get(*idx).map(|n| n.indent > indent).unwrap_or(false) {
                        map.insert(k, yaml_parse_block(lines, idx, child));
                    } else {
                        map.insert(k, Value::Null);
                    }
                } else {
                    map.insert(k, yaml_scalar(&v));
                }
                // `- k: v` 之后同级缩进更深的行并入同一 item
                if let Some(n) = lines.get(*idx) {
                    if n.indent > indent && !n.content.starts_with('-') {
                        let child = n.indent;
                        if let Value::Object(more) = yaml_parse_block(lines, idx, child) {
                            for (k2, v2) in more {
                                map.insert(k2, v2);
                            }
                        }
                    }
                }
                out.push(Value::Object(map));
            } else {
                out.push(yaml_scalar(&rest));
            }
        }
        return Value::Array(out);
    }
    let mut map = Map::new();
    while let Some(l) = lines.get(*idx) {
        if l.indent != indent {
            break;
        }
        let (k, v) = match split_kv(&l.content) {
            Some(x) => x,
            None => {
                *idx += 1;
                continue;
            }
        };
        *idx += 1;
        if v.is_empty() {
            let has_child = lines.get(*idx).map(|n| n.indent > indent).unwrap_or(false);
            if has_child {
                let child = lines[*idx].indent;
                map.insert(k, yaml_parse_block(lines, idx, child));
            } else {
                map.insert(k, Value::Null);
            }
        } else {
            map.insert(k, yaml_scalar(&v));
        }
    }
    Value::Object(map)
}

/// YAML 文本 → Value（失败返回 Null）
fn yaml_lite(text: &str) -> Value {
    let lines: Vec<YLine> = text
        .lines()
        .filter(|raw| {
            let t = raw.trim();
            !t.is_empty() && !t.starts_with('#') && !t.starts_with("---")
        })
        .map(|raw| {
            let trimmed = raw.trim_end();
            let indent = trimmed.len() - trimmed.trim_start().len();
            YLine {
                indent,
                content: trimmed.trim_start().to_string(),
            }
        })
        .collect();
    if lines.is_empty() {
        return Value::Null;
    }
    let mut idx = 0usize;
    let indent = lines[0].indent;
    yaml_parse_block(&lines, &mut idx, indent)
}

/// 配置文件解析：优先 JSON，失败回落 YAML 子集
fn read_config_value(p: &Path) -> Option<Value> {
    let text = read_text(p)?;
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        return Some(v);
    }
    let y = yaml_lite(&text);
    if y.is_null() {
        None
    } else {
        Some(y)
    }
}

// ==================== 注册表 ====================

/// 写回形态（决定「一键接入本网关」能否自动化落盘）
#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectKind {
    /// `~/.claude/settings.json` 的 `env` 段
    ClaudeSettings,
    /// `~/.codex/config.toml` 的 `model_providers.*`
    CodexToml,
    /// `~/.config/opencode/opencode.json` 的 `provider.*`
    OpenCodeJson,
    /// `~/.continue/config.json` 的 `models[]`
    ContinueJson,
    /// `~/.aider.conf.yml` 的扁平键
    AiderYaml,
    /// 仅给出可复制的配置片段（凭据存于系统密钥环，落盘会被覆盖）
    SnippetOnly,
}

struct AgentSpec {
    key: &'static str,
    name: &'static str,
    vendor: &'static str,
    protocol: &'static str,
    homepage: &'static str,
    paths: &'static [&'static str],
    connect: ConnectKind,
}

/// 内置智能体注册表。路径按「Windows 常见安装 + 跨平台 home」排列；
/// 任一命中即视为已安装，全部缺失则不展示为「可导入」。
const AGENTS: &[AgentSpec] = &[
    AgentSpec {
        key: "claude-code",
        name: "Claude Code",
        vendor: "Anthropic",
        protocol: "anthropic",
        homepage: "https://claude.com/claude-code",
        paths: &[
            "~/.claude/settings.json",
            "~/.claude.json",
            "%APPDATA%/Claude/claude_desktop_config.json",
        ],
        connect: ConnectKind::ClaudeSettings,
    },
    AgentSpec {
        key: "codex",
        name: "Codex CLI",
        vendor: "OpenAI",
        protocol: "openai",
        homepage: "https://github.com/openai/codex",
        paths: &["~/.codex/config.toml", "~/.codex/auth.json"],
        connect: ConnectKind::CodexToml,
    },
    AgentSpec {
        key: "gemini-cli",
        name: "Gemini CLI",
        vendor: "Google",
        protocol: "openai",
        homepage: "https://github.com/google-gemini/gemini-cli",
        paths: &["~/.gemini/settings.json", "~/.gemini/.env"],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "qwen-code",
        name: "Qwen Code",
        vendor: "Alibaba",
        protocol: "openai",
        homepage: "https://github.com/QwenLM/qwen-code",
        paths: &["~/.qwen/settings.json", "~/.qwen/.env"],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "opencode",
        name: "OpenCode",
        vendor: "SST",
        protocol: "openai",
        homepage: "https://opencode.ai",
        paths: &["~/.config/opencode/opencode.json", "~/.opencode.json"],
        connect: ConnectKind::OpenCodeJson,
    },
    AgentSpec {
        key: "continue",
        name: "Continue",
        vendor: "Continue",
        protocol: "openai",
        homepage: "https://continue.dev",
        paths: &["~/.continue/config.json", "~/.continue/config.yaml"],
        connect: ConnectKind::ContinueJson,
    },
    AgentSpec {
        key: "cline",
        name: "Cline",
        vendor: "Cline",
        protocol: "openai",
        homepage: "https://cline.bot",
        paths: &[
            "%APPDATA%/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
            "%APPDATA%/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_settings.json",
        ],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "roo-code",
        name: "Roo Code",
        vendor: "Roo",
        protocol: "openai",
        homepage: "https://roocode.com",
        paths: &[
            "%APPDATA%/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json",
            "%APPDATA%/Code/User/globalStorage/rooveterinaryinc.roo-cline/settings/roo_settings.json",
        ],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "cursor",
        name: "Cursor",
        vendor: "Anysphere",
        protocol: "openai",
        homepage: "https://cursor.com",
        paths: &[
            "~/.cursor/mcp.json",
            "%APPDATA%/Cursor/User/mcp.json",
            "%APPDATA%/Cursor/User/settings.json",
        ],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "windsurf",
        name: "Windsurf",
        vendor: "Codeium",
        protocol: "openai",
        homepage: "https://windsurf.com",
        paths: &["~/.codeium/windsurf/mcp_config.json"],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "cherry-studio",
        name: "Cherry Studio",
        vendor: "Cherry",
        protocol: "openai",
        homepage: "https://cherry-ai.com",
        paths: &["%APPDATA%/CherryStudio/config.json"],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "chatbox",
        name: "Chatbox",
        vendor: "Chatbox",
        protocol: "openai",
        homepage: "https://chatboxai.app",
        paths: &["%APPDATA%/xyz.chatboxapp.app/config.json"],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "aider",
        name: "Aider",
        vendor: "Aider",
        protocol: "openai",
        homepage: "https://aider.chat",
        paths: &["~/.aider.conf.yml", "~/.aider.conf.yaml"],
        connect: ConnectKind::AiderYaml,
    },
    // DSH = DeepSeek Harness（两个兄弟项目的 agent 注册表里都列为 `dsh`）：
    // 配置在 `~/.dsh/settings.yaml`（`llm-pi-ai.providers.<id>: {inline JSON}`）
    // 与 `~/.dsh/.credentials.yaml`（顶层 `refs:` 块）。自动写回不安全（客户端会重写
    // 该文件），故只导入 + 给片段。
    AgentSpec {
        key: "dsh",
        name: "DeepSeek Harness",
        vendor: "DeepSeek",
        protocol: "openai",
        homepage: "https://github.com/deepseek-ai",
        paths: &["~/.dsh/settings.yaml", "~/.dsh/.credentials.yaml"],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "crush",
        name: "Crush",
        vendor: "Charm",
        protocol: "openai",
        homepage: "https://github.com/charmbracelet/crush",
        paths: &["~/.config/crush/crush.json"],
        connect: ConnectKind::SnippetOnly,
    },
    AgentSpec {
        key: "zcode",
        name: "ZCode",
        vendor: "ZCode",
        protocol: "openai",
        homepage: "",
        paths: &["~/.zcode/v2/provider_config.json"],
        connect: ConnectKind::SnippetOnly,
    },
];

fn spec_of(key: &str) -> Option<&'static AgentSpec> {
    AGENTS.iter().find(|a| a.key == key)
}

// ==================== DTO ====================

/// 单条可导入上游（来自某智能体配置）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentProvider {
    /// 稳定 id：`<agent>::<来源定位>`，跨扫描不变，供前端勾选
    pub id: String,
    pub agent: String,
    pub agent_name: String,
    /// 展示名（如「Anthropic 官方」「中转站 A」「Qoder 网关」）
    pub label: String,
    pub base_url: String,
    /// 完整 Key（仅在 Rust 内存/落库流转；前端只用 key_masked 展示）
    #[serde(default)]
    pub api_key: String,
    pub key_masked: String,
    /// 默认模型
    pub model: String,
    /// 声明的模型清单（模型目录内列出的名字）
    pub models: Vec<String>,
    /// openai | anthropic | gemini | custom
    pub protocol: String,
    /// 来源文件（展示用）
    pub source: String,
    /// 该条是否可作为自定义模型上游导入
    pub importable: bool,
    /// 不可导入原因 / 提示
    pub note: String,
}

/// 单条 MCP server（跨智能体统一形态）
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AgentMcp {
    pub name: String,
    pub agent: String,
    /// stdio：command + args；http/sse：url
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    pub transport: String,
    pub source: String,
}

/// 单个智能体的探测结果
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentProbe {
    pub key: String,
    pub name: String,
    pub vendor: String,
    pub protocol: String,
    pub homepage: String,
    pub installed: bool,
    pub connect_supported: bool,
    pub config_paths: Vec<String>,
    pub providers: Vec<AgentProvider>,
    pub mcp_servers: Vec<AgentMcp>,
    pub warnings: Vec<String>,
    pub scanned_at: i64,
}

/// 全量扫描结果
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentScanResult {
    pub probes: Vec<AgentProbe>,
    /// 额外目录里识别出的「兄弟网关 / 外部智能体」
    pub extras: Vec<AgentProbe>,
    pub scanned_at: i64,
    pub warnings: Vec<String>,
}

/// 导入计划（预览）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentImportPlan {
    pub models: Vec<PlannedModel>,
    pub keys: Vec<PlannedKey>,
    pub mcp: Vec<PlannedMcp>,
    pub skipped: Vec<PlanSkip>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlannedModel {
    pub provider_id: String,
    pub name: String,
    pub base_url: String,
    pub vendor: String,
    pub key_masked: String,
    pub enabled: bool,
    /// 已存在同名模型（不同上游）时的说明
    pub conflict: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlannedKey {
    pub name: String,
    pub existing: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlannedMcp {
    pub name: String,
    pub agent: String,
    pub already: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlanSkip {
    pub provider_id: String,
    pub agent_name: String,
    pub label: String,
    pub reason: String,
}

/// 导入选项（前端勾选项）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentImportOptions {
    /// 建自定义模型条目（默认开）
    #[serde(default = "default_true")]
    pub create_models: bool,
    /// 每个来源智能体建一个专用 ck_ Key（默认关）
    #[serde(default)]
    pub create_keys: bool,
    /// 新模型条目是否启用（默认开）
    #[serde(default = "default_true")]
    pub enable_models: bool,
    /// 同名模型覆盖既有条目（默认关：只跳过并提示）
    #[serde(default)]
    pub overwrite: bool,
    /// 合并 MCP server 到统一注册表（默认开）
    #[serde(default = "default_true")]
    pub register_mcp: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AgentImportOptions {
    fn default() -> Self {
        Self {
            create_models: true,
            create_keys: false,
            enable_models: true,
            overwrite: false,
            register_mcp: true,
        }
    }
}

/// 导入结果
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentImportReport {
    pub models_added: usize,
    pub models_skipped: usize,
    pub keys_added: usize,
    pub keys_reused: usize,
    pub mcp_added: usize,
    pub details: Vec<String>,
    pub warnings: Vec<String>,
    pub imported_at: i64,
}

/// 导入历史记录（kv `agent_import_history`）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentImportRecord {
    pub at: i64,
    pub provider_ids: Vec<String>,
    pub models_added: usize,
    pub models_skipped: usize,
    pub keys_added: usize,
    pub mcp_added: usize,
}

/// 写回参数
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentConnectOptions {
    /// 网关地址（缺省用 gateway_settings 的 host:port）
    #[serde(default)]
    pub base_url: Option<String>,
    /// 目标智能体使用的模型（缺省用网关默认模型）
    #[serde(default)]
    pub model: Option<String>,
    /// 网关 API Key（缺省自动复用/新建「智能体接入专用」）
    #[serde(default)]
    pub api_key: Option<String>,
}

/// 写回结果
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentConnectReport {
    pub target: String,
    pub target_name: String,
    /// 已写入的文件
    pub written: Vec<String>,
    /// 备份文件
    pub backups: Vec<String>,
    /// 无法自动落盘时给出的可复制片段
    pub snippet: String,
    pub notes: Vec<String>,
}

// ==================== 解析器 ====================

#[derive(Default)]
struct Parsed {
    providers: Vec<AgentProvider>,
    mcp: Vec<AgentMcp>,
    warnings: Vec<String>,
}

impl Parsed {
    #[allow(clippy::too_many_arguments)]
    fn provider(
        &mut self,
        agent: &AgentSpec,
        id_suffix: &str,
        label: &str,
        base_url: &str,
        api_key: &str,
        model: &str,
        models: Vec<String>,
        protocol: &str,
        source: &Path,
    ) {
        let id = format!("{}::{}", agent.key, id_suffix);
        if self.providers.iter().any(|p| p.id == id) {
            return;
        }
        let base = base_url.trim().trim_end_matches('/').to_string();
        let key = api_key.trim().to_string();
        let note = if base.is_empty() {
            "配置内未找到 base_url，无法作为上游导入".to_string()
        } else if !(base.starts_with("http://") || base.starts_with("https://")) {
            format!("base_url 非 http(s)：{base}")
        } else {
            String::new()
        };
        self.providers.push(AgentProvider {
            id,
            agent: agent.key.to_string(),
            agent_name: agent.name.to_string(),
            label: if label.trim().is_empty() {
                agent.name.to_string()
            } else {
                label.trim().to_string()
            },
            importable: note.is_empty(),
            note,
            base_url: base,
            key_masked: mask_key(&key),
            api_key: key,
            model: model.trim().to_string(),
            models,
            protocol: if protocol.is_empty() {
                agent.protocol.to_string()
            } else {
                protocol.to_string()
            },
            source: pshow(source),
        });
    }

    fn mcp_server(
        &mut self,
        agent: &AgentSpec,
        name: &str,
        command: &str,
        args: Vec<String>,
        url: &str,
        transport: &str,
        source: &Path,
    ) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if self
            .mcp
            .iter()
            .any(|m| m.name == name && m.command == command && m.url == url)
        {
            return;
        }
        self.mcp.push(AgentMcp {
            name: name.to_string(),
            agent: agent.key.to_string(),
            command: command.trim().to_string(),
            args,
            url: url.trim().to_string(),
            transport: transport.to_string(),
            source: pshow(source),
        });
    }

    fn warn(&mut self, msg: impl Into<String>) {
        push_unique(&mut self.warnings, &msg.into());
    }
}

/// 通用 `mcpServers` 表解析（Claude Desktop / Cursor / Cline / Roo / Windsurf /
/// Gemini CLI / Cherry / Chatbox 同构）
fn parse_mcp_map(agent: &AgentSpec, v: &Value, source: &Path, out: &mut Parsed) {
    let Some(map) = v.get("mcpServers").and_then(Value::as_object) else {
        return;
    };
    for (name, cfg) in map {
        if !cfg.is_object() {
            continue;
        }
        let command = cfg.get("command").and_then(Value::as_str).unwrap_or("");
        let args: Vec<String> = cfg
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let url = sval(cfg, &["url", "serverUrl", "server_url", "endpoint"]).unwrap_or("");
        let transport = cfg
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| cfg.get("transport").and_then(Value::as_str))
            .unwrap_or_else(|| if url.is_empty() { "stdio" } else { "http" });
        out.mcp_server(agent, name, command, args, url, transport, source);
    }
}

/// `.env` 形态解析（KEY=VALUE，忽略注释）
fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let l = l.strip_prefix("export ").unwrap_or(l);
        if let Some((k, v)) = l.split_once('=') {
            let k = k.trim().to_string();
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            if !k.is_empty() {
                out.push((k, v));
            }
        }
    }
    out
}

fn env_get(env: &[(String, String)], keys: &[&str]) -> Option<String> {
    for (k, v) in env {
        for want in keys {
            if k.eq_ignore_ascii_case(want) && !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Claude Code：`settings.json` 的 `env` 段（官方与 cc-switch 一致）
fn parse_claude(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    for f in files {
        let Some(v) = read_json(f) else {
            out.warn(format!("{} 解析失败（非合法 JSON）", pshow(f)));
            continue;
        };
        if let Some(env) = v.get("env").and_then(Value::as_object) {
            let base = env
                .get("ANTHROPIC_BASE_URL")
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = env
                .get("ANTHROPIC_AUTH_TOKEN")
                .and_then(Value::as_str)
                .or_else(|| env.get("ANTHROPIC_API_KEY").and_then(Value::as_str))
                .unwrap_or("");
            let model = env
                .get("ANTHROPIC_MODEL")
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut models = Vec::new();
            for k in [
                "ANTHROPIC_MODEL",
                "ANTHROPIC_DEFAULT_OPUS_MODEL",
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                "CLAUDE_CODE_SUBAGENT_MODEL",
            ] {
                if let Some(m) = env.get(k).and_then(Value::as_str) {
                    push_unique(&mut models, m);
                }
            }
            if !base.trim().is_empty() {
                out.provider(agent, "settings.json#env", "Claude Code env", base, key, model, models, "anthropic", f);
            }
        }
        // 顶层扁平写法（部分第三方工具生成）
        if let Some(base) = sval(&v, &["ANTHROPIC_BASE_URL", "anthropic_base_url"]) {
            let key = sval(&v, &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]).unwrap_or("");
            out.provider(agent, "settings.json#flat", "Claude Code 顶层", base, key, "", Vec::new(), "anthropic", f);
        }
        // `primaryApiKey`（官方登录态，直连 Anthropic 官方，无可导入上游）
        if v.get("primaryApiKey").is_some() {
            out.warn("检测到 Anthropic 官方登录态（primaryApiKey）：官方端点无自定义上游可导入");
        }
        parse_mcp_map(agent, &v, f, out);
    }
}

/// Codex：`config.toml` 的 `[model_providers.*]` + `auth.json`
fn parse_codex(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    for f in files {
        let is_toml = f.extension().map(|e| e.eq_ignore_ascii_case("toml")).unwrap_or(false);
        if is_toml {
            let Some(text) = read_text(f) else { continue };
            let Ok(doc) = toml::from_str::<toml::Table>(&text) else {
                out.warn(format!("{} 解析失败（TOML 语法错误）", pshow(f)));
                continue;
            };
            let default_model = doc
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let active = doc
                .get("model_provider")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(providers) = doc.get("model_providers").and_then(|v| v.as_table()) {
                for (pid, val) in providers {
                    let Some(t) = val.as_table() else { continue };
                    let base = t.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
                    // Key 只认显式 api_key；env_key 是环境变量名，进程内取到真值才算命中
                    let key = t
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_string)
                        .or_else(|| {
                            let name = t.get("env_key").and_then(|v| v.as_str())?;
                            std::env::var(name).ok()
                        })
                        .unwrap_or_default();
                    let name = t.get("name").and_then(|v| v.as_str()).unwrap_or(pid);
                    let label = if active == *pid {
                        format!("Codex {name}（当前）")
                    } else {
                        format!("Codex {name}")
                    };
                    out.provider(agent, &format!("config.toml#model_providers.{pid}"), &label, base, &key, &default_model, Vec::new(), "openai", f);
                    if let Some(env_key) = t.get("env_key").and_then(|v| v.as_str()) {
                        out.warn(format!(
                            "provider {pid} 使用 env_key={env_key}：Key 存于环境变量，导入后需在「自定义模型」补填"
                        ));
                    }
                }
            }
            if doc.get("model_providers").is_none() {
                out.warn("config.toml 未声明 model_providers（可能使用官方登录态）");
            }
        } else if f.file_name().map(|n| n == "auth.json").unwrap_or(false) {
            if let Some(v) = read_json(f) {
                if let Some(k) = sval(&v, &["OPENAI_API_KEY", "openai_api_key"]) {
                    // Key 有但 base 缺：官方端点，不构成本地可导入上游，仅提示
                    out.warn(format!(
                        "auth.json 含 OPENAI_API_KEY（{}）：官方端点无自定义 base_url",
                        mask_key(k)
                    ));
                }
            }
        }
    }
}

/// `.env` + settings.json 型（Gemini CLI / Qwen Code）
fn parse_env_agent(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    let mut base = String::new();
    let mut key = String::new();
    let mut model = String::new();
    let mut models: Vec<String> = Vec::new();
    let mut src: Option<PathBuf> = None;
    for f in files {
        if f.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) {
            let Some(text) = read_text(f) else { continue };
            let env = parse_dotenv(&text);
            for k in ["OPENAI_BASE_URL", "GEMINI_BASE_URL", "BASE_URL", "OPENAI_API_BASE"] {
                if let Some(v) = env_get(&env, &[k]) {
                    base = v;
                    src = Some(f.clone());
                    break;
                }
            }
            for k in ["OPENAI_API_KEY", "GEMINI_API_KEY", "API_KEY"] {
                if let Some(v) = env_get(&env, &[k]) {
                    key = v;
                    src = src.clone().or_else(|| Some(f.clone()));
                    break;
                }
            }
            for k in ["OPENAI_MODEL", "GEMINI_MODEL", "MODEL"] {
                if let Some(v) = env_get(&env, &[k]) {
                    push_unique(&mut models, &v);
                    if model.is_empty() {
                        model = v;
                    }
                }
            }
        } else if let Some(v) = read_config_value(f) {
            if base.is_empty() {
                if let Some(b) = sval_fuzzy(&v, &["baseURL", "base_url", "apiBase", "openaiBaseUrl"]) {
                    base = b;
                    src = src.clone().or_else(|| Some(f.clone()));
                }
            }
            if key.is_empty() {
                if let Some(k) = sval_fuzzy(&v, &["apiKey", "api_key", "token"]) {
                    key = k;
                    src = src.clone().or_else(|| Some(f.clone()));
                }
            }
            if let Some(m) = sval_fuzzy(&v, &["model", "defaultModel"]) {
                push_unique(&mut models, &m);
                if model.is_empty() {
                    model = m;
                }
            }
            parse_mcp_map(agent, &v, f, out);
        }
    }
    if !base.trim().is_empty() {
        let source = src.unwrap_or_else(|| PathBuf::from(agent.name));
        out.provider(agent, "env#gateway", &format!("{} 上游", agent.name), &base, &key, &model, models, "openai", &source);
    } else if !key.is_empty() {
        out.warn(format!(
            "{} 有 API Key（{}）但无自定义 base_url：官方端点无法作为本地上游导入",
            agent.name,
            mask_key(&key)
        ));
    }
}

/// OpenCode：`provider.<id>.options.baseURL / apiKey`
fn parse_opencode(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    for f in files {
        let Some(v) = read_config_value(f) else { continue };
        let default_model = sval_fuzzy(&v, &["model", "defaultModel"]).unwrap_or_default();
        if let Some(providers) = v.get("provider").and_then(Value::as_object) {
            for (pid, pv) in providers {
                let opts = pv.get("options").unwrap_or(pv);
                let base = sval(opts, &["baseURL", "baseUrl", "base_url"]).unwrap_or("");
                let key = sval(opts, &["apiKey", "api_key"]).unwrap_or("");
                let mut models: Vec<String> = pv
                    .get("models")
                    .and_then(Value::as_object)
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                if !default_model.is_empty() {
                    push_unique(&mut models, &default_model);
                }
                out.provider(agent, &format!("opencode.json#provider.{pid}"), &format!("OpenCode {pid}"), base, key, &default_model, models, "openai", f);
            }
        }
        parse_mcp_map(agent, &v, f, out);
        if let Some(mcp) = v.get("mcp").and_then(Value::as_object) {
            for (name, cfg) in mcp {
                let command = cfg
                    .get("command")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args: Vec<String> = cfg
                    .get("command")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .skip(1)
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let url = sval(cfg, &["url"]).unwrap_or("");
                out.mcp_server(agent, name, command, args, url, "stdio", f);
            }
        }
    }
}

/// Continue：`models[]`（JSON 或 YAML）
fn parse_continue(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    for f in files {
        let Some(v) = read_config_value(f) else {
            out.warn(format!("{} 解析失败", pshow(f)));
            continue;
        };
        let models = v.get("models").and_then(Value::as_array).cloned().unwrap_or_default();
        for (i, m) in models.iter().enumerate() {
            let provider = m.get("provider").and_then(Value::as_str).unwrap_or("");
            let base = sval(m, &["apiBase", "api_base", "baseUrl", "base_url"]).unwrap_or("");
            let key = sval(m, &["apiKey", "api_key"]).unwrap_or("");
            let model = m.get("model").and_then(Value::as_str).unwrap_or("");
            let title = m.get("title").and_then(Value::as_str).unwrap_or("");
            let label = if title.is_empty() {
                format!("Continue {}#{i}", if provider.is_empty() { "model" } else { provider })
            } else {
                format!("Continue {title}")
            };
            if base.is_empty() && !key.is_empty() {
                out.warn(format!(
                    "Continue 模型 {label} 使用 {provider} 内置端点（无 apiBase），不可作为本地上游导入"
                ));
            }
            out.provider(agent, &format!("continue#models[{i}]"), &label, base, key, model, Vec::new(), "openai", f);
        }
        parse_mcp_map(agent, &v, f, out);
        if let Some(mcp) = v.get("mcpServers").and_then(Value::as_object) {
            for (name, cfg) in mcp {
                let command = cfg.get("command").and_then(Value::as_str).unwrap_or("");
                let args: Vec<String> = cfg
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                out.mcp_server(agent, name, command, args, "", "stdio", f);
            }
        }
    }
}

/// 通用 MCP-only 客户端（Cursor / Cline / Roo / Windsurf / Cherry / Chatbox）
fn parse_mcp_only(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    for f in files {
        let Some(v) = read_config_value(f) else {
            out.warn(format!("{} 解析失败", pshow(f)));
            continue;
        };
        parse_mcp_map(agent, &v, f, out);
        // 少数客户端把地址放在 `providers` / `apiProviders`
        for key in ["providers", "apiProviders", "models"] {
            let arr = v.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
            for (i, p) in arr.iter().enumerate() {
                let base = sval_fuzzy(p, &["baseUrl", "apiBase", "endpoint", "host"]).unwrap_or_default();
                let k = sval_fuzzy(p, &["apiKey", "key", "token"]).unwrap_or_default();
                let name = sval_fuzzy(p, &["name", "label", "title"]).unwrap_or_else(|| format!("{key}#{i}"));
                let model = sval_fuzzy(p, &["model", "defaultModel"]).unwrap_or_default();
                if !base.is_empty() {
                    out.provider(agent, &format!("{key}#{i}"), &name, &base, &k, &model, Vec::new(), "openai", f);
                }
            }
        }
    }
}

/// Aider：`~/.aider.conf.yml` 扁平键
/// DSH（DeepSeek Harness）解析：`~/.dsh/settings.yaml` 的
/// `llm-pi-ai.providers.<id>: {inline JSON}`（`baseURL` / `displayName` / `apiKeyEnv` / `models[]`）
/// + `~/.dsh/.credentials.yaml` 顶层 `refs:` 块里的 `<apiKeyEnv>: <key>`。
///
/// 说明：hub（`wb_agents._apply_dsh`）写的是 **inline-flow JSON**，所以按行扫
/// 「`<id>: {` 开头的行 → serde_json 解析」，再独立解析 refs 块；不追求通用 YAML。
fn parse_dsh(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    let mut refs: Vec<(String, String)> = Vec::new();
    let mut settings: Option<&PathBuf> = None;
    for f in files {
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name.contains("credential") {
            let Some(text) = read_text(f) else { continue };
            let mut in_refs = false;
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let indent = line.len() - line.trim_start().len();
                if indent == 0 {
                    in_refs = trimmed == "refs:";
                    continue;
                }
                if in_refs {
                    if let Some((k, v)) = trimmed.split_once(':') {
                        let v = v.trim().trim_matches('"').trim_matches('\'');
                        if !k.trim().is_empty() && !v.is_empty() {
                            refs.push((k.trim().to_string(), v.to_string()));
                        }
                    }
                }
            }
        } else if name.ends_with("settings.yaml") {
            settings = Some(f);
        }
    }
    let Some(path) = settings else { return };
    let Some(text) = read_text(path) else { return };
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(": {") else { continue };
        let key = key.trim().trim_matches('"');
        let json_text = format!("{{{}", rest);
        let Ok(v) = serde_json::from_str::<Value>(&json_text) else { continue };
        let base = sval(&v, &["baseURL", "baseUrl", "base_url"]).unwrap_or("");
        if base.trim().is_empty() {
            continue;
        }
        let label = sval(&v, &["displayName", "name"]).unwrap_or(key);
        let key_ref = sval(&v, &["apiKeyEnv", "apiKeyRef", "credential"]).unwrap_or("");
        let api_key = refs
            .iter()
            .find(|(k, _)| k == key_ref)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let models: Vec<String> = v
            .get("models")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let default_model = models.first().cloned().unwrap_or_default();
        out.provider(
            agent,
            &format!("settings.yaml#providers.{key}"),
            label,
            base,
            &api_key,
            &default_model,
            models,
            "openai",
            path,
        );
        if !key_ref.is_empty() && api_key.is_empty() {
            out.warn(format!(
                "DSH provider {key} 的 Key 引用 {key_ref} 未在 .credentials.yaml 找到（可能尚未登录）"
            ));
        }
    }
}

fn parse_aider(agent: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    for f in files {
        let Some(text) = read_text(f) else { continue };
        let v = yaml_lite(&text);
        let base = sval_fuzzy(&v, &["openai-api-base", "openaiApiBase", "api-base"]).unwrap_or_default();
        let key = sval_fuzzy(&v, &["openai-api-key", "openaiApiKey", "api-key"]).unwrap_or_default();
        let model = sval_fuzzy(&v, &["model", "weak-model"]).unwrap_or_default();
        if base.is_empty() && key.is_empty() {
            out.warn("aider.conf.yml 未配置 openai-api-base / openai-api-key（可能用环境变量）");
        }
        out.provider(agent, "aider.conf#openai", "Aider OpenAI 兼容", &base, &key, &model, Vec::new(), "openai", f);
    }
}

/// 按 agent key 分派解析
fn parse_agent(spec: &AgentSpec, files: &[PathBuf], out: &mut Parsed) {
    match spec.key {
        "claude-code" => parse_claude(spec, files, out),
        "codex" => parse_codex(spec, files, out),
        "gemini-cli" | "qwen-code" => parse_env_agent(spec, files, out),
        "opencode" => parse_opencode(spec, files, out),
        "continue" => parse_continue(spec, files, out),
        "aider" => parse_aider(spec, files, out),
        "dsh" => parse_dsh(spec, files, out),
        _ => parse_mcp_only(spec, files, out),
    }
}

fn scan_one(spec: &AgentSpec) -> AgentProbe {
    let mut probe = AgentProbe {
        key: spec.key.to_string(),
        name: spec.name.to_string(),
        vendor: spec.vendor.to_string(),
        protocol: spec.protocol.to_string(),
        homepage: spec.homepage.to_string(),
        installed: false,
        connect_supported: spec.connect != ConnectKind::SnippetOnly,
        scanned_at: now_ts(),
        ..Default::default()
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for p in spec.paths {
        if let Some(path) = resolve_path(p) {
            if path.is_file() {
                probe.config_paths.push(pshow(&path));
                files.push(path);
            }
        }
    }
    probe.installed = !files.is_empty();
    if !probe.installed {
        probe.warnings.push("未检测到配置文件".to_string());
        return probe;
    }
    let mut parsed = Parsed::default();
    parse_agent(spec, &files, &mut parsed);
    probe.providers = parsed.providers;
    probe.mcp_servers = parsed.mcp;
    probe.warnings.extend(parsed.warnings);
    if probe.providers.is_empty() && probe.mcp_servers.is_empty() {
        probe
            .warnings
            .push("配置里没有可导入的 OpenAI 兼容上游或 MCP server".to_string());
    }
    probe
}

// ==================== 额外目录扫描（兄弟网关 / 外部智能体） ====================

/// 额外目录扫描：识别 qoder2api-hub / workbuddy2api-hub 这类兄弟网关。
///
/// 识别规则（只读、启发式，命中即生成一条「网关上游」）：
/// - `settings.json` / `qoder_settings.json` 里带 `port`（+ 可选 `api_key`/`token`）
///   → `http://127.0.0.1:<port>/v1`；
/// - 模型目录 `*catalog*.json` / `catalog/*.json`：取模型 id 清单并入 models；
/// - `accounts/*.json`：仅统计数量并提示（账号池不在此处接管，避免双真源）。
fn scan_extra_dir(dir: &Path) -> Option<AgentProbe> {
    if !dir.is_dir() {
        return None;
    }
    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| pshow(dir));
    let lower = dir_name.to_ascii_lowercase();
    let (key, name) = if lower.contains("qoder") {
        ("extra-qoder", "Qoder 兄弟网关")
    } else if lower.contains("workbuddy") || lower.contains("wb-proxy") {
        ("extra-workbuddy", "WorkBuddy 兄弟网关")
    } else {
        ("extra-agent", "外部智能体目录")
    };
    let mut probe = AgentProbe {
        key: key.to_string(),
        name: format!("{name}（{dir_name}）"),
        vendor: "本地目录".to_string(),
        protocol: "openai".to_string(),
        homepage: String::new(),
        installed: true,
        connect_supported: false,
        scanned_at: now_ts(),
        ..Default::default()
    };
    probe.config_paths.push(pshow(dir));

    let mut models: Vec<String> = Vec::new();
    let mut port: Option<u16> = None;
    let mut api_key = String::new();
    let mut account_files = 0usize;
    let mut scanned: Vec<PathBuf> = Vec::new();

    // 1) 目录根：settings / catalog
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let fname = p
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if fname.ends_with(".json") && (fname.contains("settings") || fname == "settings.json") {
                if let Some(v) = read_json(&p) {
                    if let Some(pt) = v.get("port").and_then(Value::as_u64) {
                        if pt > 0 && pt <= u64::from(u16::MAX) {
                            port = Some(pt as u16);
                        }
                    }
                    for k in ["api_key", "apikey", "token", "access_key"] {
                        if let Some(s) = v.get(k).and_then(Value::as_str) {
                            if !s.trim().is_empty() {
                                api_key = s.trim().to_string();
                                break;
                            }
                        }
                    }
                }
                scanned.push(p);
            } else if fname.ends_with(".json") && fname.contains("catalog") {
                if let Some(v) = read_json(&p) {
                    collect_model_names(&v, &mut models, 0);
                }
                scanned.push(p);
            }
        }
    }
    // 2) catalog/ 子目录
    if let Ok(entries) = std::fs::read_dir(dir.join("catalog")) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().map(|x| x.eq_ignore_ascii_case("json")).unwrap_or(false) {
                if let Some(v) = read_json(&p) {
                    collect_model_names(&v, &mut models, 0);
                }
                scanned.push(p);
            }
        }
    }
    // 3) accounts/ 账号数量统计（仅提示）
    if let Ok(entries) = std::fs::read_dir(dir.join("accounts")) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().map(|x| x.eq_ignore_ascii_case("json")).unwrap_or(false) {
                account_files += 1;
            }
        }
    }

    if let Some(port) = port {
        let spec = AgentSpec {
            key: "extra",
            name: "兄弟网关",
            vendor: "本地目录",
            protocol: "openai",
            homepage: "",
            paths: &[],
            connect: ConnectKind::SnippetOnly,
        };
        let mut parsed = Parsed::default();
        let src = scanned
            .first()
            .cloned()
            .unwrap_or_else(|| dir.to_path_buf());
        let base = format!("http://127.0.0.1:{port}/v1");
        let default_model = models.first().cloned().unwrap_or_default();
        parsed.provider(
            &spec,
            "gateway",
            &format!("{name} :{port}"),
            &base,
            &api_key,
            &default_model,
            models.clone(),
            "openai",
            &src,
        );
        // id 前缀改成真实 key，便于前端按 agent 归组
        for p in &mut parsed.providers {
            p.id = format!("{key}::gateway");
            p.agent = key.to_string();
            p.agent_name = probe.name.clone();
            // 供应商展示名沿用探测名（AgentProvider 无独立 vendor 字段）
            p.label = format!("{} · 本地网关", probe.name);
        }
        probe.providers = parsed.providers;
        probe.warnings.extend(parsed.warnings);
    } else if !models.is_empty() {
        probe
            .warnings
            .push("发现模型目录但未找到可用端口（settings 缺失）：无法生成网关上游".to_string());
    }
    if account_files > 0 {
        probe.warnings.push(format!(
            "目录内 {account_files} 个账号配置文件：账号池请走各自应用页（Qoder / Buddy）导入，避免双真源"
        ));
    }
    if probe.providers.is_empty() && probe.warnings.is_empty() {
        probe.warnings.push("未识别出网关或模型目录".to_string());
    }
    Some(probe)
}

/// 递归收集模型名（catalog 结构各家不同：取 `id` / `name` / `model` / 及数组元素字符串）
fn collect_model_names(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 6 || out.len() > 500 {
        return;
    }
    match v {
        Value::Array(arr) => {
            for item in arr {
                collect_model_names(item, out, depth + 1);
            }
        }
        Value::Object(map) => {
            let mut has_name = false;
            for k in ["id", "name", "model", "model_id", "config_name"] {
                if let Some(s) = map.get(k).and_then(Value::as_str) {
                    let s = s.trim();
                    if !s.is_empty() && s.len() < 80 {
                        push_unique(out, s);
                        has_name = true;
                    }
                }
            }
            if !has_name {
                for (k, val) in map {
                    if val.is_object() || val.is_array() {
                        collect_model_names(val, out, depth + 1);
                    } else if let Some(s) = val.as_str() {
                        if s.len() < 80 {
                            push_unique(out, s);
                        }
                    } else {
                        push_unique(out, k);
                    }
                }
            }
        }
        _ => {}
    }
}

// ==================== 计划 / 应用 ====================

fn collect_selected(
    provider_ids: &[String],
    extra_roots: &[String],
) -> (Vec<AgentProvider>, Vec<AgentMcp>, Vec<String>) {
    let mut providers = Vec::new();
    let mut mcp = Vec::new();
    let mut warnings = Vec::new();
    for spec in AGENTS {
        // 仅扫描被选中的智能体，避免无谓 IO
        let wanted_prefix = format!("{}::", spec.key);
        if !provider_ids.iter().any(|id| id.starts_with(&wanted_prefix)) {
            continue;
        }
        let probe = scan_one(spec);
        for p in probe.providers {
            if provider_ids.iter().any(|id| id == &p.id) {
                providers.push(p);
            }
        }
        mcp.extend(probe.mcp_servers);
        warnings.extend(probe.warnings);
    }
    for root in extra_roots {
        if let Some(probe) = scan_extra_dir(Path::new(root)) {
            for p in probe.providers {
                if provider_ids.iter().any(|id| id == &p.id) {
                    providers.push(p);
                }
            }
            warnings.extend(probe.warnings);
        }
    }
    (providers, mcp, warnings)
}

/// 生成导入计划（只读，不落库）；`data_dir` 用于与既有模型/Key 比对
fn build_plan(
    data_dir: &Path,
    provider_ids: &[String],
    extra_roots: &[String],
    opts: &AgentImportOptions,
) -> AgentImportPlan {
    let (providers, _mcp, warnings) = collect_selected(provider_ids, extra_roots);
    let mut plan = AgentImportPlan {
        warnings,
        ..Default::default()
    };
    let existing = custom_models::load(data_dir);
    let keys = api_keys::load(data_dir);
    for p in providers {
        if !p.importable {
            plan.skipped.push(PlanSkip {
                provider_id: p.id.clone(),
                agent_name: p.agent_name.clone(),
                label: p.label.clone(),
                reason: if p.note.is_empty() {
                    "缺少可用 base_url".to_string()
                } else {
                    p.note.clone()
                },
            });
            continue;
        }
        let name = model_name_for(&p);
        let canonical = crate::api_server::unified_catalog::canonical_id(&name);
        let conflict = match existing.iter().find(|e| {
            crate::api_server::unified_catalog::canonical_id(&e.name) == canonical
        }) {
            Some(hit) => {
                if hit.base_url.trim_end_matches('/') == p.base_url.trim_end_matches('/') {
                    "已存在（同一上游），导入时跳过".to_string()
                } else {
                    format!("已存在同名模型，当前上游为 {}（导入时跳过）", hit.base_url)
                }
            }
            None => String::new(),
        };
        plan.models.push(PlannedModel {
            provider_id: p.id.clone(),
            name,
            base_url: p.base_url.clone(),
            vendor: vendor_for(&p),
            key_masked: p.key_masked.clone(),
            enabled: opts.enable_models,
            conflict,
        });
        if opts.create_keys {
            let key_name = key_name_for(&p);
            if !plan.keys.iter().any(|k| k.name == key_name) {
                plan.keys.push(PlannedKey {
                    existing: keys.keys.iter().any(|e| e.name == key_name),
                    name: key_name,
                });
            }
        }
    }
    plan
}

/// 导入后的模型名：优先 provider 声明的默认模型，否则用 `<agent>-<label>` 归一化
fn model_name_for(p: &AgentProvider) -> String {
    if !p.model.trim().is_empty() {
        return p.model.trim().to_string();
    }
    if let Some(first) = p.models.first() {
        if !first.trim().is_empty() {
            return first.trim().to_string();
        }
    }
    let slug = p
        .label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase();
    let tail = if slug.is_empty() {
        "upstream".to_string()
    } else {
        slug
    };
    format!("{}-{}", p.agent, tail)
}

fn vendor_for(p: &AgentProvider) -> String {
    if !p.agent_name.trim().is_empty() {
        p.agent_name.clone()
    } else {
        p.agent.clone()
    }
}

fn key_name_for(p: &AgentProvider) -> String {
    format!("{} 接入", p.agent_name)
}

/// 取/建「智能体接入专用」Key（复用同名，被禁用则恢复启用）
fn ensure_gateway_key(data_dir: &Path, name: &str) -> Result<String, String> {
    let mut out: Option<String> = None;
    api_keys::modify(data_dir, |file| {
        if let Some(e) = file.keys.iter_mut().find(|k| k.name == name) {
            let mut changed = false;
            if !e.enabled {
                e.enabled = true;
                changed = true;
            }
            out = Some(e.key.clone());
            return changed;
        }
        let entry = api_keys::ApiKeyEntry {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            key: format!("ck_{}", uuid::Uuid::new_v4().simple()),
            enabled: true,
            daily_limit: 0,
            created_at: now_ts() as u64,
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
        out = Some(entry.key.clone());
        file.keys.push(entry);
        true
    });
    out.ok_or_else(|| "创建接入专用 Key 失败".to_string())
}

/// 应用导入计划（落库：自定义模型 + 专用 Key + MCP 注册表 + 历史）
fn apply_plan(
    data_dir: &Path,
    plan: &AgentImportPlan,
    providers: &[AgentProvider],
    mcp: &[AgentMcp],
    opts: &AgentImportOptions,
) -> AgentImportReport {
    let mut report = AgentImportReport {
        imported_at: now_ts(),
        warnings: plan.warnings.clone(),
        ..Default::default()
    };
    if opts.create_models {
        for m in &plan.models {
            let Some(src) = providers.iter().find(|p| p.id == m.provider_id) else {
                continue;
            };
            if !m.conflict.is_empty() && !opts.overwrite {
                report.models_skipped += 1;
                report.details.push(format!("跳过 {}：{}", m.name, m.conflict));
                continue;
            }
            let entry = custom_models::CustomModel {
                id: String::new(),
                name: m.name.clone(),
                base_url: m.base_url.clone(),
                api_key: src.api_key.clone(),
                vendor: m.vendor.clone(),
                enabled: opts.enable_models,
                context_length: 0,
                max_tokens: 0,
                supports_image: false,
                rate: 0.0,
                note: format!("导入自 {}（{}）", src.agent_name, src.source),
                updated_at: 0,
            };
            // 覆盖既有同名条目：先删后建（upsert 的 canonical 唯一性校验会拦同名）
            if !m.conflict.is_empty() && opts.overwrite {
                if let Some(hit) = custom_models::load(data_dir).into_iter().find(|e| {
                    crate::api_server::unified_catalog::canonical_id(&e.name)
                        == crate::api_server::unified_catalog::canonical_id(&m.name)
                }) {
                    let _ = custom_models::remove(data_dir, &hit.id);
                }
            }
            match custom_models::upsert(data_dir, entry) {
                Ok(_) => {
                    report.models_added += 1;
                    report
                        .details
                        .push(format!("新增模型 {} → {}", m.name, m.base_url));
                }
                Err(e) => {
                    report.models_skipped += 1;
                    report.details.push(format!("跳过 {}：{}", m.name, e));
                }
            }
        }
    }
    if opts.create_keys {
        for k in &plan.keys {
            let before = api_keys::load(data_dir).keys.len();
            match ensure_gateway_key(data_dir, &k.name) {
                Ok(_) => {
                    let after = api_keys::load(data_dir).keys.len();
                    if after > before {
                        report.keys_added += 1;
                    } else {
                        report.keys_reused += 1;
                    }
                    report.details.push(format!("API Key「{}」就绪", k.name));
                }
                Err(e) => report.warnings.push(e),
            }
        }
    }
    if opts.register_mcp {
        let mut registry: Vec<AgentMcp> = crate::store::db(data_dir).kv_get(KV_MCP);
        let mut added = 0usize;
        for s in mcp {
            if !registry
                .iter()
                .any(|x| x.name == s.name && x.command == s.command && x.url == s.url)
            {
                registry.push(s.clone());
                added += 1;
            }
        }
        if added > 0 {
            registry.truncate(200);
            if let Err(e) = crate::store::db(data_dir).kv_set(KV_MCP, &registry) {
                report.warnings.push(format!("MCP 注册表写入失败：{e}"));
            }
        }
        report.mcp_added = added;
    }
    // 历史
    let mut history: Vec<AgentImportRecord> = crate::store::db(data_dir).kv_get(KV_HISTORY);
    history.push(AgentImportRecord {
        at: report.imported_at,
        provider_ids: plan.models.iter().map(|m| m.provider_id.clone()).collect(),
        models_added: report.models_added,
        models_skipped: report.models_skipped,
        keys_added: report.keys_added,
        mcp_added: report.mcp_added,
    });
    if history.len() > HISTORY_CAP {
        let drop = history.len() - HISTORY_CAP;
        history.drain(0..drop);
    }
    if let Err(e) = crate::store::db(data_dir).kv_set(KV_HISTORY, &history) {
        report.warnings.push(format!("导入历史写入失败：{e}"));
    }
    report
}

// ==================== 写回（一键接入本网关） ====================

fn gateway_base(data_dir: &Path, override_base: Option<&str>) -> String {
    if let Some(b) = override_base {
        let b = b.trim().trim_end_matches('/');
        if !b.is_empty() {
            return b.to_string();
        }
    }
    let gs = crate::api_server::gateway_settings::load(data_dir);
    let host = if gs.host.trim().is_empty() {
        "127.0.0.1"
    } else {
        gs.host.trim()
    };
    // 监听 0.0.0.0 时给客户端用环回地址
    let host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    format!("http://{host}:{}", gs.port)
}

fn backup_file(p: &Path) -> Option<PathBuf> {
    if !p.is_file() {
        return None;
    }
    let bak = PathBuf::from(format!("{}.bak_aiwork_{}", p.display(), now_ts()));
    std::fs::copy(p, &bak).ok().map(|_| bak)
}

fn write_if_changed(p: &Path, content: &str, report: &mut AgentConnectReport) -> Result<(), String> {
    if p.is_file() {
        if let Some(old) = read_text(p) {
            if old == content {
                report.notes.push(format!("{} 已是目标配置，无需改动", pshow(p)));
                return Ok(());
            }
        }
        if let Some(bak) = backup_file(p) {
            report.backups.push(pshow(&bak));
        }
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    crate::fs_utils::write_text_atomic(p, content)?;
    report.written.push(pshow(p));
    Ok(())
}

fn claude_payload(base: &str, model: &str, key: &str) -> Value {
    serde_json::json!({
        "env": {
            "ANTHROPIC_BASE_URL": base,
            "ANTHROPIC_AUTH_TOKEN": key,
            "ANTHROPIC_API_KEY": key,
            "ANTHROPIC_MODEL": model,
            "ANTHROPIC_DEFAULT_OPUS_MODEL": model,
            "ANTHROPIC_DEFAULT_SONNET_MODEL": model,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": model,
            "CLAUDE_CODE_SUBAGENT_MODEL": model,
        }
    })
}

/// Codex config.toml 最小行级编辑：只接管 `model` / `model_provider` 两个顶层键
/// 与 `[model_providers.aiwork]` 段，其余内容原样保留
fn merge_codex_toml(old: &str, base_v1: &str, model: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut skip_section = false;
    for line in old.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            skip_section = t.starts_with("[model_providers.aiwork]");
            if skip_section {
                continue;
            }
        }
        if skip_section {
            continue;
        }
        if t.starts_with("model =") || t.starts_with("model_provider =") {
            continue;
        }
        kept.push(line.to_string());
    }
    while kept.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        kept.pop();
    }
    let mut out = String::new();
    out.push_str(&format!("model = \"{}\"\n", toml_escape(model)));
    out.push_str(&format!(
        "model_provider = \"{}\"\n",
        GW_PROVIDER_ID
    ));
    for l in kept {
        out.push_str(&l);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&format!("[model_providers.{}]\n", GW_PROVIDER_ID));
    out.push_str(&format!("name = \"{}\"\n", toml_escape(GW_PROVIDER_NAME)));
    out.push_str(&format!("base_url = \"{}\"\n", toml_escape(base_v1)));
    out.push_str("wire_api = \"responses\"\n");
    out.push_str("requires_openai_auth = true\n");
    out
}

fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn snippet_for(target: &AgentSpec, base: &str, model: &str, key: &str) -> String {
    let base_v1 = format!("{base}/v1");
    match target.key {
        "claude-code" => format!(
            "# Claude Code · settings.json\n{}",
            serde_json::to_string_pretty(&claude_payload(base, model, key)).unwrap_or_default()
        ),
        "codex" => format!(
            "# Codex CLI · ~/.codex/config.toml\nmodel = \"{model}\"\nmodel_provider = \"{}\"\n\n[model_providers.{}]\nname = \"{}\"\nbase_url = \"{base_v1}\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n",
            GW_PROVIDER_ID, GW_PROVIDER_ID, GW_PROVIDER_NAME
        ),
        "opencode" => format!(
            "# OpenCode · ~/.config/opencode/opencode.json\n{{\n  \"provider\": {{\n    \"{}\": {{\n      \"npm\": \"@ai-sdk/openai-compatible\",\n      \"options\": {{ \"baseURL\": \"{base_v1}\", \"apiKey\": \"{key}\" }}\n    }}\n  }}\n}}",
            GW_PROVIDER_ID
        ),
        "continue" => format!(
            "# Continue · ~/.continue/config.json（models 数组追加一项）\n{{\n  \"title\": \"{}\",\n  \"provider\": \"openai\",\n  \"model\": \"{model}\",\n  \"apiBase\": \"{base_v1}\",\n  \"apiKey\": \"{key}\"\n}}",
            GW_PROVIDER_NAME
        ),
        "aider" => format!(
            "# Aider · ~/.aider.conf.yml\nopenai-api-base: {base_v1}\nopenai-api-key: {key}\nmodel: openai/{model}"
        ),
        _ => format!(
            "# {} · 通用 OpenAI 兼容配置\nBase URL: {base_v1}\nAPI Key : {key}\nModel   : {model}\n# 若客户端仅支持固定厂商端点，可把 Base URL 填到「自定义 / OpenAI Compatible」输入框",
            target.name
        ),
    }
}

fn connect_impl(
    data_dir: &Path,
    target_key: &str,
    opts: &AgentConnectOptions,
) -> Result<AgentConnectReport, String> {
    let spec = spec_of(target_key).ok_or_else(|| format!("未知智能体：{target_key}"))?;
    let gs = crate::api_server::gateway_settings::load(data_dir);
    let base = gateway_base(data_dir, opts.base_url.as_deref());
    let model = opts
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(gs.default_model);
    let key = match opts.api_key.clone() {
        Some(k) if !k.trim().is_empty() => k,
        _ => ensure_gateway_key(data_dir, GW_KEY_NAME)?,
    };
    let base_v1 = format!("{base}/v1");
    let mut report = AgentConnectReport {
        target: spec.key.to_string(),
        target_name: spec.name.to_string(),
        ..Default::default()
    };
    match spec.connect {
        ConnectKind::ClaudeSettings => {
            let path = resolve_path("~/.claude/settings.json")
                .ok_or("无法定位用户主目录")?;
            let mut root = if path.is_file() {
                read_json(&path).unwrap_or_else(|| serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            if !root.is_object() {
                root = serde_json::json!({});
            }
            let env = claude_payload(&base, &model, &key)["env"].clone();
            root["env"] = env;
            let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
            write_if_changed(&path, &format!("{text}\n"), &mut report)?;
            report
                .notes
                .push("Claude Code 读取 ANTHROPIC_BASE_URL（不带 /v1），已按官方格式写入".to_string());
        }
        ConnectKind::CodexToml => {
            let path = resolve_path("~/.codex/config.toml").ok_or("无法定位用户主目录")?;
            let old = read_text(&path).unwrap_or_default();
            let merged = merge_codex_toml(&old, &base_v1, &model);
            write_if_changed(&path, &merged, &mut report)?;
            // auth.json 补 Key（Codex 从 OPENAI_API_KEY 读鉴权）
            if let Some(auth_path) = resolve_path("~/.codex/auth.json") {
                let mut auth = read_json(&auth_path).unwrap_or_else(|| serde_json::json!({}));
                auth["OPENAI_API_KEY"] = Value::String(key.clone());
                let text = serde_json::to_string_pretty(&auth).map_err(|e| e.to_string())?;
                write_if_changed(&auth_path, &format!("{text}\n"), &mut report)?;
            }
            report
                .notes
                .push("Codex 使用 wire_api=responses 指向 /v1；网关已提供该协议端点".to_string());
        }
        ConnectKind::OpenCodeJson => {
            let path = resolve_path("~/.config/opencode/opencode.json")
                .or_else(|| resolve_path("~/.opencode.json"))
                .ok_or("无法定位用户主目录")?;
            let mut root = if path.is_file() {
                read_config_value(&path).unwrap_or_else(|| serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            if !root.is_object() {
                root = serde_json::json!({});
            }
            root["provider"][GW_PROVIDER_ID] = serde_json::json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": GW_PROVIDER_NAME,
                "options": { "baseURL": base_v1, "apiKey": key },
                "models": { model.clone(): { "name": model } }
            });
            let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
            write_if_changed(&path, &format!("{text}\n"), &mut report)?;
        }
        ConnectKind::ContinueJson => {
            let path = resolve_path("~/.continue/config.json").ok_or("无法定位用户主目录")?;
            let mut root = if path.is_file() {
                read_json(&path).unwrap_or_else(|| serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            if !root.is_object() {
                root = serde_json::json!({});
            }
            let mut models = root
                .get("models")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            models.retain(|m| m.get("title").and_then(Value::as_str) != Some(GW_PROVIDER_NAME));
            models.push(serde_json::json!({
                "title": GW_PROVIDER_NAME,
                "provider": "openai",
                "model": model,
                "apiBase": base_v1,
                "apiKey": key,
            }));
            root["models"] = Value::Array(models);
            let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
            write_if_changed(&path, &format!("{text}\n"), &mut report)?;
        }
        ConnectKind::AiderYaml => {
            let path = resolve_path("~/.aider.conf.yml").ok_or("无法定位用户主目录")?;
            let old = read_text(&path).unwrap_or_default();
            let mut kept: Vec<String> = Vec::new();
            for line in old.lines() {
                let t = line.trim_start();
                if t.starts_with("openai-api-base:")
                    || t.starts_with("openai-api-key:")
                    || t.starts_with("model:")
                {
                    continue;
                }
                kept.push(line.to_string());
            }
            while kept.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                kept.pop();
            }
            let mut out = String::new();
            for l in kept {
                out.push_str(&l);
                out.push('\n');
            }
            out.push_str(&format!("openai-api-base: {base_v1}\n"));
            out.push_str(&format!("openai-api-key: {key}\n"));
            out.push_str(&format!("model: openai/{model}\n"));
            write_if_changed(&path, &out, &mut report)?;
        }
        ConnectKind::SnippetOnly => {
            report.notes.push(
                "该客户端把上游凭据存在系统密钥环/应用内部库，直接改文件会被覆盖：已生成可复制片段"
                    .to_string(),
            );
        }
    }
    if report.snippet.is_empty() {
        report.snippet = snippet_for(spec, &base, &model, &key);
    }
    report
        .notes
        .push(format!("网关地址 {base}，模型 {model}（可改端口/模型后重新接入）"));
    Ok(report)
}

// ==================== Tauri 命令 ====================

/// 全量扫描：内置注册表 + 额外目录
#[tauri::command]
pub fn agent_scan_all(extra_roots: Option<Vec<String>>) -> AgentScanResult {
    let mut result = AgentScanResult {
        scanned_at: now_ts(),
        ..Default::default()
    };
    for spec in AGENTS {
        result.probes.push(scan_one(spec));
    }
    for root in extra_roots.unwrap_or_default() {
        let p = PathBuf::from(root.trim());
        if p.as_os_str().is_empty() {
            continue;
        }
        match scan_extra_dir(&p) {
            Some(probe) => result.extras.push(probe),
            None => result
                .warnings
                .push(format!("目录不存在或不可读：{}", pshow(&p))),
        }
    }
    let installed = result.probes.iter().filter(|p| p.installed).count();
    if installed == 0 {
        result
            .warnings
            .push("未检测到任何已安装的智能体：可在下方「额外目录」添加兄弟项目路径".to_string());
    }
    result
}

/// 单个体扫描
#[tauri::command]
pub fn agent_scan_one(key: String) -> Result<AgentProbe, String> {
    spec_of(&key)
        .map(scan_one)
        .ok_or_else(|| format!("未知智能体：{key}"))
}

/// 导入预览（不落库）
#[tauri::command]
pub fn agent_import_preview(
    state: tauri::State<'_, AppState>,
    provider_ids: Vec<String>,
    opts: Option<AgentImportOptions>,
    extra_roots: Option<Vec<String>>,
) -> AgentImportPlan {
    let opts = opts.unwrap_or_default();
    let roots = extra_roots.unwrap_or_default();
    build_plan(&state.data_dir, &provider_ids, &roots, &opts)
}

/// 执行导入
#[tauri::command]
pub fn agent_import_apply(
    state: tauri::State<'_, AppState>,
    provider_ids: Vec<String>,
    opts: Option<AgentImportOptions>,
    extra_roots: Option<Vec<String>>,
) -> AgentImportReport {
    let opts = opts.unwrap_or_default();
    let roots = extra_roots.unwrap_or_default();
    let plan = build_plan(&state.data_dir, &provider_ids, &roots, &opts);
    let (providers, mcp, _warn) = collect_selected(&provider_ids, &roots);
    apply_plan(&state.data_dir, &plan, &providers, &mcp, &opts)
}

/// 一键接入本网关（写回目标智能体配置；不支持落盘的返回片段）
#[tauri::command]
pub fn agent_connect(
    state: tauri::State<'_, AppState>,
    target: String,
    opts: Option<AgentConnectOptions>,
) -> Result<AgentConnectReport, String> {
    connect_impl(&state.data_dir, &target, &opts.unwrap_or_default())
}

/// 导出汇总（可分享的脱敏 bundle：上游清单 + MCP 注册表）
///
/// 抽成普通函数：`commands::hub` 的加密导出（`hub_export_bundle_sealed`）直接复用，
/// 避免 Tauri `State` 在两处命令间传递。
pub fn export_bundle(data_dir: &Path) -> String {
    let mut providers: Vec<Value> = Vec::new();
    for spec in AGENTS {
        let probe = scan_one(spec);
        for p in probe.providers {
            providers.push(serde_json::json!({
                "agent": p.agent,
                "agent_name": p.agent_name,
                "label": p.label,
                "base_url": p.base_url,
                "model": p.model,
                "models": p.models,
                "protocol": p.protocol,
                "key_masked": p.key_masked,
                "importable": p.importable,
                "source": p.source,
            }));
        }
    }
    let mcp: Vec<AgentMcp> = crate::store::db(data_dir).kv_get(KV_MCP);
    serde_json::to_string_pretty(&serde_json::json!({
        "kind": "aiwork-agent-bundle",
        "version": 1,
        "exported_at": now_ts(),
        "providers": providers,
        "mcp_servers": mcp,
        "note": "仅含上游地址与 Key 掩码，完整 Key 不导出；账号池请用各应用页的加密导出",
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

/// 导出汇总（命令入口）
#[tauri::command]
pub fn agent_export_bundle(state: tauri::State<'_, AppState>) -> String {
    export_bundle(&state.data_dir)
}

/// 导入历史（最近 50 条）
#[tauri::command]
pub fn agent_import_history(state: tauri::State<'_, AppState>) -> Vec<AgentImportRecord> {
    crate::store::db(&state.data_dir).kv_get(KV_HISTORY)
}

/// 统一 MCP 注册表
#[tauri::command]
pub fn agent_mcp_registry(state: tauri::State<'_, AppState>) -> Vec<AgentMcp> {
    crate::store::db(&state.data_dir).kv_get(KV_MCP)
}

/// 清空统一 MCP 注册表
#[tauri::command]
pub fn agent_mcp_clear(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let old: Vec<AgentMcp> = crate::store::db(&state.data_dir).kv_get(KV_MCP);
    let n = old.len();
    crate::store::db(&state.data_dir)
        .kv_set(KV_MCP, &Vec::<AgentMcp>::new())
        .map(|_| n)
}

/// 网关接入信息（前端展示「把这段配置粘到任意客户端」）
#[tauri::command]
pub fn agent_gateway_info(state: tauri::State<'_, AppState>) -> Value {
    let gs = crate::api_server::gateway_settings::load(&state.data_dir);
    let base = gateway_base(&state.data_dir, None);
    serde_json::json!({
        "base_url": base,
        "base_url_v1": format!("{base}/v1"),
        "host": gs.host,
        "port": gs.port,
        "default_model": gs.default_model,
        "key_name": GW_KEY_NAME,
        "provider_id": GW_PROVIDER_ID,
    })
}

// ==================== 单测 ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aw_agent_import_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn mask_key_keeps_head_and_tail() {
        assert_eq!(mask_key("sk-1234567890abcd"), "sk-1****abcd");
        assert_eq!(mask_key("short"), "****");
        assert_eq!(mask_key(""), "");
    }

    #[test]
    fn yaml_lite_parses_continue_shape() {
        let text = "\
name: demo
models:
  - title: 中转站
    provider: openai
    model: gpt-4o-mini
    apiBase: https://relay.example.com/v1
    apiKey: sk-abcdefghijkl
    roles:
      - chat
  - title: 官方
    provider: openai
    model: gpt-4o
";
        let v = yaml_lite(text);
        assert_eq!(v["name"], serde_json::json!("demo"));
        let models = v["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["title"], serde_json::json!("中转站"));
        assert_eq!(
            models[0]["apiBase"],
            serde_json::json!("https://relay.example.com/v1")
        );
        assert_eq!(models[0]["roles"][0], serde_json::json!("chat"));
        assert_eq!(models[1]["model"], serde_json::json!("gpt-4o"));
    }

    #[test]
    fn yaml_lite_handles_nested_map() {
        let v = yaml_lite("a:\n  b: 1\n  c:\n    - x\n    - y\nd: true\n");
        assert_eq!(v["a"]["b"], serde_json::json!(1));
        assert_eq!(v["a"]["c"][1], serde_json::json!("y"));
        assert_eq!(v["d"], serde_json::json!(true));
    }

    #[test]
    fn dotenv_parsing_ignores_comments() {
        let env = parse_dotenv("# c\nOPENAI_BASE_URL=https://x/v1\nOPENAI_API_KEY=\"sk-1\"\n");
        assert_eq!(env_get(&env, &["OPENAI_BASE_URL"]).as_deref(), Some("https://x/v1"));
        assert_eq!(env_get(&env, &["OPENAI_API_KEY"]).as_deref(), Some("sk-1"));
    }

    #[test]
    fn claude_settings_env_becomes_provider() {
        let dir = fixture();
        let f = dir.join("settings.json");
        std::fs::write(
            &f,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://relay.example.com","ANTHROPIC_AUTH_TOKEN":"sk-1234567890abcd","ANTHROPIC_MODEL":"claude-x"}}"#,
        )
        .unwrap();
        let spec = spec_of("claude-code").unwrap();
        let mut parsed = Parsed::default();
        parse_claude(spec, &[f], &mut parsed);
        assert_eq!(parsed.providers.len(), 1);
        let p = &parsed.providers[0];
        assert_eq!(p.base_url, "https://relay.example.com");
        assert_eq!(p.model, "claude-x");
        assert_eq!(p.key_masked, "sk-1****abcd");
        assert!(p.importable);
        assert_eq!(p.id, "claude-code::settings.json#env");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_toml_providers_are_listed() {
        let dir = fixture();
        let f = dir.join("config.toml");
        std::fs::write(
            &f,
            "model = \"gpt-5\"\nmodel_provider = \"relay\"\n\n[model_providers.relay]\nname = \"Relay\"\nbase_url = \"https://relay.example.com/v1\"\nwire_api = \"responses\"\n",
        )
        .unwrap();
        let spec = spec_of("codex").unwrap();
        let mut parsed = Parsed::default();
        parse_codex(spec, &[f], &mut parsed);
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].base_url, "https://relay.example.com/v1");
        assert_eq!(parsed.providers[0].model, "gpt-5");
        assert!(parsed.providers[0].label.contains("当前"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn continue_models_parsed_from_yaml() {
        let dir = fixture();
        let f = dir.join("config.yaml");
        std::fs::write(
            &f,
            "models:\n  - title: Relay\n    provider: openai\n    model: gpt-4o-mini\n    apiBase: https://relay.example.com/v1\n    apiKey: sk-abcdefghijkl\n",
        )
        .unwrap();
        let spec = spec_of("continue").unwrap();
        let mut parsed = Parsed::default();
        parse_continue(spec, &[f], &mut parsed);
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].base_url, "https://relay.example.com/v1");
        assert_eq!(parsed.providers[0].model, "gpt-4o-mini");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extra_dir_detects_gateway_port_and_models() {
        let dir = fixture();
        let sub = dir.join("qoder2api-hub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join("settings.json"),
            r#"{"port":7890,"api_key":"sk-1234567890abcd"}"#,
        )
        .unwrap();
        std::fs::write(
            sub.join("qoder_catalog_cn.json"),
            r#"[{"id":"qwen3.8-flash"},{"id":"kimi-k3"}]"#,
        )
        .unwrap();
        let probe = scan_extra_dir(&sub).unwrap();
        assert_eq!(probe.providers.len(), 1);
        assert_eq!(probe.providers[0].base_url, "http://127.0.0.1:7890/v1");
        assert_eq!(
            probe.providers[0].models,
            vec!["qwen3.8-flash".to_string(), "kimi-k3".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_toml_merge_is_idempotent() {
        let old = "model = \"old\"\nmodel_provider = \"relay\"\n\n[model_providers.relay]\nname = \"Relay\"\nbase_url = \"https://relay/v1\"\n";
        let once = merge_codex_toml(old, "http://127.0.0.1:7864/v1", "glm-5.3");
        assert!(once.contains("model = \"glm-5.3\""));
        assert!(once.contains("model_provider = \"aiwork\""));
        assert!(once.contains("[model_providers.aiwork]"));
        assert!(once.contains("[model_providers.relay]"), "其他 provider 必须保留");
        let twice = merge_codex_toml(&once, "http://127.0.0.1:7864/v1", "glm-5.3");
        assert_eq!(once, twice, "二次写入必须幂等");
        assert_eq!(twice.matches("[model_providers.aiwork]").count(), 1);
    }

    #[test]
    fn apply_plan_adds_custom_model_and_is_idempotent() {
        let dir = fixture();
        let provider = AgentProvider {
            id: "claude-code::settings.json#env".into(),
            agent: "claude-code".into(),
            agent_name: "Claude Code".into(),
            label: "Claude Code env".into(),
            base_url: "https://relay.example.com".into(),
            api_key: "sk-1234567890abcd".into(),
            key_masked: "sk-1****abcd".into(),
            model: "claude-x".into(),
            models: vec!["claude-x".into()],
            protocol: "anthropic".into(),
            source: "settings.json".into(),
            importable: true,
            note: String::new(),
        };
        let plan = AgentImportPlan {
            models: vec![PlannedModel {
                provider_id: provider.id.clone(),
                name: "claude-x".into(),
                base_url: provider.base_url.clone(),
                vendor: "Claude Code".into(),
                key_masked: provider.key_masked.clone(),
                enabled: true,
                conflict: String::new(),
            }],
            ..Default::default()
        };
        let opts = AgentImportOptions::default();
        let r1 = apply_plan(&dir, &plan, std::slice::from_ref(&provider), &[], &opts);
        assert_eq!(r1.models_added, 1);
        let list = custom_models::load(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "claude-x");
        assert_eq!(list[0].api_key, "sk-1234567890abcd");
        // 二次导入同一 provider：同名冲突 → 跳过
        let mut plan2 = plan.clone();
        plan2.models[0].conflict = "已存在".into();
        let r2 = apply_plan(&dir, &plan2, std::slice::from_ref(&provider), &[], &opts);
        assert_eq!(r2.models_added, 0);
        assert_eq!(r2.models_skipped, 1);
        // 历史记录累积
        let hist: Vec<AgentImportRecord> = crate::store::db(&dir).kv_get(KV_HISTORY);
        assert_eq!(hist.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gateway_key_reused_not_duplicated() {
        let dir = fixture();
        let k1 = ensure_gateway_key(&dir, GW_KEY_NAME).unwrap();
        let k2 = ensure_gateway_key(&dir, GW_KEY_NAME).unwrap();
        assert_eq!(k1, k2);
        assert!(k1.starts_with("ck_"));
        assert_eq!(k1.len(), 35);
        assert_eq!(api_keys::load(&dir).keys.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snippet_contains_base_and_key() {
        let spec = spec_of("codex").unwrap();
        let s = snippet_for(spec, "http://127.0.0.1:7864", "glm-5.3", "ck_abc");
        assert!(s.contains("base_url = \"http://127.0.0.1:7864/v1\""));
        assert!(s.contains("wire_api = \"responses\""));
    }
}
