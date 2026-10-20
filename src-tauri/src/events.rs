//! 事件发射统一出口（issue #44 遗留项审查）：emit 失败不再 `let _` 静默——
//! 落 stderr + 应用日志（可选），避免「前端收不到终态/结果事件却无任何线索」。
//!
//! 适用范围：终态/结果型事件（`*-done`、`account-captured`、`proxy-crashed` 等）。
//! 高频进度事件（`checkin-progress`、`update-download-progress`、`proxy-log` 等）
//! 可继续直接 emit：失败刷屏无意义，且不承载前端互斥解锁职责。
use std::path::Path;

use tauri::{AppHandle, Emitter};

/// 发射事件；失败时记 stderr + 可选应用日志。
pub fn emit_logged(app: &AppHandle, event: &str, payload: serde_json::Value, data_dir: Option<&Path>) {
    if let Err(e) = app.emit(event, payload) {
        let msg = format!("emit {event} 失败（前端收不到该事件，相关界面可能停留在进行中状态）: {e}");
        eprintln!("[event] {msg}");
        if let Some(dir) = data_dir {
            crate::fs_utils::app_log(dir, &msg);
        }
    }
}
