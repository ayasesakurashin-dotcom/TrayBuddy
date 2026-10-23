#[path = "screen_targets.rs"]
mod targets;
use super::settings::Settings;
use std::sync::Mutex;
pub use targets::{
    capture_target, consume_target, invalidate_targets, prepare_target, start_tracking,
    stop_tracking, validate_target,
};
static LAST_EXTERNAL: Mutex<Option<(isize, String, std::time::Instant)>> = Mutex::new(None);

#[cfg(windows)]
pub fn track_foreground() {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    };
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != std::process::id() && !hwnd.0.is_null() {
            let mut text = [0u16; 1024];
            let len = GetWindowTextW(hwnd, &mut text);
            *LAST_EXTERNAL.lock().unwrap() = Some((
                hwnd.0 as isize,
                String::from_utf16_lossy(&text[..len.max(0) as usize]),
                std::time::Instant::now(),
            ));
        }
    }
}
#[cfg(not(windows))]
pub fn track_foreground() {}

pub fn restore_focus() {
    #[cfg(windows)]
    if let Some((handle, _, _)) = LAST_EXTERNAL.lock().unwrap().as_ref() {
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow},
        };
        let hwnd = HWND(*handle as *mut _);
        unsafe {
            if IsWindow(hwnd).as_bool() {
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(windows)]
pub fn capture(settings: &Settings, app: &tauri::AppHandle) -> Result<(String, Vec<u8>), String> {
    use tauri::Manager;
    use windows::Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::WindowsAndMessaging::{
            GetCursorPos, GetForegroundWindow, GetWindowRect, GetWindowTextW,
            GetWindowThreadProcessId, IsIconic, IsWindow,
        },
    };
    let (title, rect) = unsafe {
        let foreground = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(foreground, Some(&mut pid));
        if pid != std::process::id() && !foreground.0.is_null() {
            let mut text = [0u16; 1024];
            let len = GetWindowTextW(foreground, &mut text);
            *LAST_EXTERNAL.lock().unwrap() = Some((
                foreground.0 as isize,
                String::from_utf16_lossy(&text[..len.max(0) as usize]),
                std::time::Instant::now(),
            ));
        }
        let (handle, title, seen) = LAST_EXTERNAL
            .lock()
            .unwrap()
            .clone()
            .ok_or("还没有可观察的外部前台窗口")?;
        if seen.elapsed().as_secs() > 30 {
            return Err("最近没有有效的外部前台窗口".into());
        }
        if settings.excluded_windows.iter().any(|value| {
            !value.trim().is_empty() && title.to_lowercase().contains(&value.to_lowercase())
        }) {
            return Err("当前窗口在排除列表中".into());
        }
        let hwnd = HWND(handle as *mut _);
        if !IsWindow(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return Err("前台窗口不可见".into());
        }
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).map_err(|_| "无法获取窗口范围")?;
        if settings.capture_scope == "screen" {
            let mut point = POINT::default();
            GetCursorPos(&mut point).map_err(|_| "无法获取显示器")?;
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return Err("显示器范围不可用".into());
            }
            rect = info.rcMonitor;
        } else if settings.capture_scope == "region" {
            let [x, y, w, h] = settings.capture_region;
            rect = RECT {
                left: x,
                top: y,
                right: x.saturating_add(w),
                bottom: y.saturating_add(h),
            };
        }
        (title, rect)
    };
    capture_rect(title, rect, app)
}

#[cfg(windows)]
fn capture_rect(
    title: String,
    rect: windows::Win32::Foundation::RECT,
    app: &tauri::AppHandle,
) -> Result<(String, Vec<u8>), String> {
    use tauri::Manager;
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
        return Err("窗口截图尺寸无效".into());
    }
    let raw = crate::modules::screenshot::capture_screen_region_as_png_bytes(
        rect.left, rect.top, w as u32, h as u32,
    )?;
    let mut image = image::load_from_memory(&raw).map_err(|_| "截图编码失败")?;
    // Mask this app's overlays before uploading; selecting an external foreground
    // window alone does not remove the always-on-top pet from the captured pixels.
    use image::GenericImage;
    for window in app.webview_windows().values() {
        if !window.is_visible().unwrap_or(false) {
            continue;
        }
        if let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) {
            let left = (pos.x - rect.left).max(0).min(w) as u32;
            let top = (pos.y - rect.top).max(0).min(h) as u32;
            let right = (pos.x.saturating_add(size.width as i32) - rect.left)
                .max(0)
                .min(w) as u32;
            let bottom = (pos.y.saturating_add(size.height as i32) - rect.top)
                .max(0)
                .min(h) as u32;
            for y in top..bottom {
                for x in left..right {
                    image.put_pixel(x, y, image::Rgba([32, 32, 32, 255]));
                }
            }
        }
    }
    if image.width() > 1600 || image.height() > 1200 {
        image = image.thumbnail(1600, 1200);
    }
    let rgb = image.to_rgb8();
    if rgb
        .pixels()
        .step_by(100)
        .all(|pixel| pixel.0.iter().all(|v| *v < 5))
    {
        return Err("截图为空或受保护".into());
    }
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| "截图编码失败")?;
    Ok((title, output.into_inner()))
}
#[cfg(not(windows))]
pub fn capture(_: &Settings, _: &tauri::AppHandle) -> Result<(String, Vec<u8>), String> {
    Err("屏幕观察暂仅支持 Windows".into())
}
