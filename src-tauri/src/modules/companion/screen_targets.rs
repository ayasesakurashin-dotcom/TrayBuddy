//! Ephemeral, one-use foreground targets. No image, HWND or token is persisted.
use super::super::{
    observations::PreparedScreenTarget,
    providers,
    settings::{self, Settings},
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};
static TARGETS: OnceLock<Mutex<HashMap<String, ScreenTarget>>> = OnceLock::new();
static WATCHED: OnceLock<Mutex<HashMap<isize, Vec<Weak<AtomicBool>>>>> = OnceLock::new();
static HOOK_READY: AtomicBool = AtomicBool::new(false);
static HOOK_HANDLE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
fn targets() -> &'static Mutex<HashMap<String, ScreenTarget>> {
    TARGETS.get_or_init(Mutex::default)
}
fn watched() -> &'static Mutex<HashMap<isize, Vec<Weak<AtomicBool>>>> {
    WATCHED.get_or_init(Mutex::default)
}
#[derive(Clone)]
pub struct ScreenTarget {
    handle: isize,
    pid: u32,
    process_started: u64,
    class: String,
    label: String,
    epoch: u64,
    settings_revision: u64,
    prepared: Instant,
    alive: Arc<AtomicBool>,
}
pub struct CapturedScreen {
    pub label: String,
    pub image: Vec<u8>,
    pub observed_at: String,
    pub scope: String,
}
impl ScreenTarget {
    fn validate_lifetime(&self, epoch: u64, revision: u64) -> Result<(), String> {
        if self.epoch != epoch {
            return Err("history_changed".into());
        }
        if self.settings_revision != revision || !self.alive.load(Ordering::SeqCst) {
            return Err("screen_target_changed".into());
        }
        if self.prepared.elapsed() >= Duration::from_secs(600) {
            return Err("screen_target_expired".into());
        }
        Ok(())
    }
}
pub fn invalidate_targets() {
    for entries in watched().lock().unwrap().values() {
        for flag in entries.iter().filter_map(Weak::upgrade) {
            flag.store(false, Ordering::SeqCst);
        }
    }
    targets().lock().unwrap().clear();
}
#[cfg(windows)]
pub fn start_tracking(app: &tauri::AppHandle) {
    use windows::Win32::{
        Foundation::HWND,
        UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK},
    };
    unsafe extern "system" fn destroyed(
        _: HWINEVENTHOOK,
        _: u32,
        hwnd: HWND,
        object: i32,
        child: i32,
        _: u32,
        _: u32,
    ) {
        if object != 0 || child != 0 {
            return;
        }
        if let Some(entries) = watched().lock().unwrap().remove(&(hwnd.0 as isize)) {
            for flag in entries.iter().filter_map(Weak::upgrade) {
                flag.store(false, Ordering::SeqCst);
            }
        }
    }
    let _ = app.run_on_main_thread(|| unsafe {
        // The Tauri UI thread owns a message pump for out-of-context callbacks.
        let hook = SetWinEventHook(0x8001, 0x8001, None, Some(destroyed), 0, 0, 0);
        HOOK_HANDLE.store(hook.0 as isize, Ordering::SeqCst);
        HOOK_READY.store(!hook.0.is_null(), Ordering::SeqCst);
    });
}
#[cfg(not(windows))]
pub fn start_tracking(_: &tauri::AppHandle) {}
pub fn stop_tracking() {
    invalidate_targets();
    HOOK_READY.store(false, Ordering::SeqCst);
    #[cfg(windows)]
    unsafe {
        let handle = HOOK_HANDLE.swap(0, Ordering::SeqCst);
        if handle != 0 {
            let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(
                windows::Win32::UI::Accessibility::HWINEVENTHOOK(handle as *mut _),
            );
        }
    }
}

#[cfg(windows)]
fn identity(
    handle: isize,
) -> Result<(u32, u64, String, String, windows::Win32::Foundation::RECT), String> {
    use windows::Win32::{
        Foundation::{CloseHandle, FILETIME, HWND, RECT},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        UI::WindowsAndMessaging::{
            GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
            IsWindow, IsWindowVisible,
        },
    };
    unsafe {
        let hwnd = HWND(handle as *mut _);
        if !IsWindow(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool()
        {
            return Err("screen_target_unavailable".into());
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return Err("screen_target_unavailable".into());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|_| "screen_target_unavailable")?;
        let (mut creation, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(process);
        result.map_err(|_| "screen_target_unavailable")?;
        let started = (creation.dwHighDateTime as u64) << 32 | creation.dwLowDateTime as u64;
        let mut title = [0u16; 1024];
        let length = GetWindowTextW(hwnd, &mut title).max(0) as usize;
        let mut class = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class).max(0) as usize;
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).map_err(|_| "screen_target_unavailable")?;
        Ok((
            pid,
            started,
            String::from_utf16_lossy(&class[..class_len]),
            String::from_utf16_lossy(&title[..length]),
            rect,
        ))
    }
}
#[cfg(windows)]
fn check_window(
    target: &ScreenTarget,
) -> Result<(String, windows::Win32::Foundation::RECT), String> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    target.validate_lifetime(target.epoch, target.settings_revision)?;
    let (pid, started, class, title, rect) = identity(target.handle)?;
    if pid != target.pid || started != target.process_started || class != target.class {
        return Err("screen_target_changed".into());
    }
    unsafe {
        let foreground = GetForegroundWindow();
        let mut current_pid = 0;
        GetWindowThreadProcessId(foreground, Some(&mut current_pid));
        if !foreground.0.is_null()
            && current_pid != std::process::id()
            && foreground.0 as isize != target.handle
        {
            return Err("screen_target_changed".into());
        }
    }
    if super::LAST_EXTERNAL
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|(handle, _, _)| *handle != target.handle)
    {
        return Err("screen_target_changed".into());
    }
    Ok((title, rect))
}

pub fn prepare_target(epoch: u64, revision: u64) -> Result<PreparedScreenTarget, String> {
    #[cfg(windows)]
    {
        if !HOOK_READY.load(Ordering::SeqCst) {
            return Err("screen_target_unavailable".into());
        }
        super::track_foreground();
        let (handle, _, seen) = super::LAST_EXTERNAL
            .lock()
            .unwrap()
            .clone()
            .ok_or("screen_target_unavailable")?;
        if seen.elapsed() > Duration::from_secs(30) {
            return Err("screen_target_unavailable".into());
        }
        let (pid, process_started, class, title, _) = identity(handle)?;
        let label = providers::redact(&title)
            .chars()
            .take(160)
            .collect::<String>();
        let alive = Arc::new(AtomicBool::new(true));
        let target = ScreenTarget {
            handle,
            pid,
            process_started,
            class,
            label: label.clone(),
            epoch,
            settings_revision: revision,
            prepared: Instant::now(),
            alive: alive.clone(),
        };
        {
            let mut watched = watched().lock().unwrap();
            watched.retain(|_, entries| {
                entries.retain(|v| v.strong_count() > 0);
                !entries.is_empty()
            });
            watched
                .entry(handle)
                .or_default()
                .push(Arc::downgrade(&alive));
        }
        let token = settings::random_id()?;
        {
            let mut pending = targets().lock().unwrap();
            pending.retain(|_, v| {
                v.prepared.elapsed() < Duration::from_secs(600) && v.alive.load(Ordering::SeqCst)
            });
            if pending.len() >= 16 {
                pending.clear();
            }
            pending.insert(token.clone(), target);
        }
        return Ok(PreparedScreenTarget {
            target_token: token,
            label: if label.is_empty() {
                "应用窗口".into()
            } else {
                label
            },
            scope: "foreground".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (epoch, revision);
        Err("screen_target_unavailable".into())
    }
}
pub fn validate_target(token: &str, epoch: u64, revision: u64) -> Result<(), String> {
    let target = targets()
        .lock()
        .unwrap()
        .get(token)
        .cloned()
        .ok_or("screen_target_expired")?;
    target.validate_lifetime(epoch, revision)?;
    #[cfg(windows)]
    {
        check_window(&target)?;
    }
    Ok(())
}
pub fn consume_target(token: &str, epoch: u64, revision: u64) -> Result<ScreenTarget, String> {
    validate_target(token, epoch, revision)?;
    targets()
        .lock()
        .unwrap()
        .remove(token)
        .ok_or("screen_target_expired".into())
}
pub fn capture_target(
    target: &ScreenTarget,
    settings: &Settings,
    app: &tauri::AppHandle,
) -> Result<CapturedScreen, String> {
    #[cfg(windows)]
    {
        let (title, rect) = check_window(target)?;
        if settings
            .excluded_windows
            .iter()
            .any(|s| !s.trim().is_empty() && title.to_lowercase().contains(&s.to_lowercase()))
        {
            return Err("screen_excluded".into());
        }
        let (_, image) =
            super::capture_rect(title.clone(), rect, app).map_err(|_| "screen_capture_failed")?;
        let (after, after_rect) = check_window(target)?;
        if after != title
            || after_rect.left != rect.left
            || after_rect.top != rect.top
            || after_rect.right != rect.right
            || after_rect.bottom != rect.bottom
        {
            return Err("screen_target_changed".into());
        }
        return Ok(CapturedScreen {
            label: providers::redact(&title),
            image,
            observed_at: chrono::Utc::now().to_rfc3339(),
            scope: "foreground".into(),
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (target, settings, app);
        Err("screen_capture_failed".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_lifetime_binds_epoch_revision_and_destroyed_window() {
        let t = ScreenTarget {
            handle: 1,
            pid: 2,
            process_started: 3,
            class: "app".into(),
            label: "window".into(),
            epoch: 4,
            settings_revision: 5,
            prepared: Instant::now(),
            alive: Arc::new(AtomicBool::new(true)),
        };
        assert!(t.validate_lifetime(4, 5).is_ok());
        assert_eq!(t.validate_lifetime(3, 5).unwrap_err(), "history_changed");
        assert!(t.validate_lifetime(4, 6).is_err());
        t.alive.store(false, Ordering::SeqCst);
        assert_eq!(
            t.validate_lifetime(4, 5).unwrap_err(),
            "screen_target_changed"
        );
    }
}
