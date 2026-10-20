//! UI 坐标点击（F-18 兜底，原 src-python/workbuddy_ui_click.py 的 Rust 移植）。
//! 无 API 可用时的最后手段：驱动鼠标对 WorkBuddy 客户端签到按钮做坐标点击。
//! 红线：仅手动触发、默认关闭（settings.ui_click_enabled）；坐标需用户预配置；
//! 全程零 token 输出；单次执行只点击一次，不做循环连点。
//! 注：windows-sys 的 mouse_event 已标注 deprecated（违反零 warning 红线），
//! 点击统一走等价的 SendInput（注入级 API，行为一致）。

use std::thread::sleep;
use std::time::Duration;

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_MOUSE, INPUT_0, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

const MOVE_DELAY_MS: u64 = 350; // 移动后停顿，给悬浮反馈留时间
const CAPTURE_DELAY_S: u64 = 3; // 取点倒计时
const CLICK_GAP_MS: u64 = 50; // 按下与抬起间隔（对齐 python 版 0.05s）

/// 3 秒倒计时后记录当前鼠标坐标（取点模式，对齐 python --capture：
/// 用户把鼠标移到签到按钮上，倒计时结束时读位置）。
pub fn capture_point() -> Result<(i32, i32), String> {
    sleep(Duration::from_secs(CAPTURE_DELAY_S));
    let mut pt = POINT { x: 0, y: 0 };
    let ok = unsafe { GetCursorPos(&mut pt) };
    if ok == 0 {
        return Err("取点失败：GetCursorPos 不可用".to_string());
    }
    Ok((pt.x, pt.y))
}

/// 移动到坐标并左键单击一次（对齐 python --click；坐标需正整数屏幕坐标，
/// 防负值/零值误触——项目输入坐标校验约定）。
pub fn click_at(x: i32, y: i32) -> Result<(), String> {
    if x <= 0 || y <= 0 {
        return Err("坐标无效（需正整数屏幕坐标）".to_string());
    }
    let moved = unsafe { SetCursorPos(x, y) };
    if moved == 0 {
        return Err("移动鼠标失败：SetCursorPos 返回 0".to_string());
    }
    sleep(Duration::from_millis(MOVE_DELAY_MS));
    send_mouse_flag(MOUSEEVENTF_LEFTDOWN);
    sleep(Duration::from_millis(CLICK_GAP_MS));
    send_mouse_flag(MOUSEEVENTF_LEFTUP);
    Ok(())
}

/// 注入单次鼠标事件（down/up 各调一次，等价 python mouse_event 两次调用）。
fn send_mouse_flag(dw_flags: u32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: dw_flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(test)]
mod ui_click_tests {
    use super::*;

    #[test]
    fn click_rejects_non_positive_coords() {
        // 不触达真实鼠标 API：坐标校验前置失败
        assert!(click_at(0, 100).is_err());
        assert!(click_at(100, 0).is_err());
        assert!(click_at(-1, -1).is_err());
    }
}
