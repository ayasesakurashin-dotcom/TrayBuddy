use super::Companion;
use crate::app_state::AppState;
use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

pub const INPUT_LABEL: &str = "companion-input";

pub fn sync(app: &tauri::AppHandle) -> Result<(), String> {
    let service = app.state::<Companion>();
    let (enabled, collapsed) = {
        let settings = service.settings.lock().unwrap();
        (settings.input_enabled, settings.input_collapsed)
    };
    let (width, height) = if collapsed { (132., 32.) } else { (330., 84.) };
    let allowed = Companion::allowed(app, false);
    let pet = crate::get_render_window(app);
    if !enabled
        || !allowed
        || pet
            .as_ref()
            .map(|p| !p.is_visible().unwrap_or(false))
            .unwrap_or(true)
    {
        if let Some(window) = app.get_webview_window(INPUT_LABEL) {
            let _ = window.hide();
        }
        return Ok(());
    }
    let pet = pet.ok_or("桌宠窗口不可用")?;
    let window = if let Some(window) = app.get_webview_window(INPUT_LABEL) {
        window
    } else {
        let window =
            WebviewWindowBuilder::new(app, INPUT_LABEL, WebviewUrl::App("companion_input".into()))
                .title("和艾玛聊聊")
                .inner_size(width, height)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .shadow(false)
                .visible(false)
                .focused(false)
                .build()
                .map_err(|e| e.to_string())?;
        let handle = app.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let service = handle.state::<Companion>();
                let mut cfg = service.settings.lock().unwrap().clone();
                cfg.input_enabled = false;
                let _ = service.save(cfg);
                let _ = handle.get_webview_window(INPUT_LABEL).map(|w| w.hide());
            }
        });
        window
    };
    let scale = pet.scale_factor().map_err(|e| e.to_string())?;
    let mut pos = pet.outer_position().map_err(|e| e.to_string())?;
    let size = pet.outer_size().map_err(|e| e.to_string())?;
    let input_w = (width * scale) as u32;
    let input_h = (height * scale) as u32;
    let mut x = pos.x + (size.width as i32 - input_w as i32) / 2;
    let mut y = pos.y + size.height as i32;
    if let Some(monitor) = pet.current_monitor().map_err(|e| e.to_string())? {
        let area = monitor.work_area();
        x = x
            .max(area.position.x)
            .min(area.position.x + area.size.width as i32 - input_w as i32);
        let bottom = area.position.y + area.size.height as i32;
        if y + input_h as i32 > bottom {
            let shift = y + input_h as i32 - bottom;
            pos.y = (pos.y - shift).max(area.position.y);
            pet.set_position(pos).map_err(|e| e.to_string())?;
            y = pos.y + size.height as i32;
        }
        y = y.max(area.position.y).min(bottom - input_h as i32);
    }
    let target = PhysicalPosition::new(x, y);
    if window.outer_size().ok() != Some(PhysicalSize::new(input_w, input_h)) {
        let _ = window.set_size(PhysicalSize::new(input_w, input_h));
    }
    if window.outer_position().ok() != Some(target) {
        let _ = window.set_position(target);
    }
    if !window.is_visible().unwrap_or(false) {
        #[cfg(windows)]
        {
            let hwnd = window.hwnd().map_err(|e| e.to_string())?;
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                    windows::Win32::Foundation::HWND(hwnd.0 as *mut _),
                    windows::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
                );
            }
        }
        #[cfg(not(windows))]
        window.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn refresh_menu(app: &tauri::AppHandle) {
    if let Some(tray) = app.tray_by_id(crate::modules::constants::TRAY_ID_MAIN) {
        if let Ok(menu) = crate::inner_build_tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

pub fn menu_items(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Submenu<tauri::Wry>> {
    use tauri::menu::{CheckMenuItem, MenuItem, Submenu};
    let cfg = app.state::<Companion>().settings.lock().unwrap().clone();
    let input = CheckMenuItem::with_id(
        app,
        "companion_input",
        "对话输入框",
        true,
        cfg.input_enabled,
        None::<&str>,
    )?;
    let speech = CheckMenuItem::with_id(
        app,
        "companion_speech",
        "AI 回复语音",
        true,
        cfg.speech_enabled,
        None::<&str>,
    )?;
    let screen = CheckMenuItem::with_id(
        app,
        "companion_observation",
        "屏幕观察",
        true,
        cfg.observation_enabled,
        None::<&str>,
    )?;
    let once = MenuItem::with_id(app, "companion_once", "立即观察一次", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "companion_stop", "停止回复和播放", true, None::<&str>)?;
    Submenu::with_items(
        app,
        "艾玛 AI",
        true,
        &[&input, &speech, &screen, &once, &stop],
    )
}
pub fn menu_event(app: &tauri::AppHandle, id: &str) {
    let service = app.state::<Companion>();
    if id == "companion_stop" {
        service.cancel(app);
        return;
    }
    if id == "companion_once" {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = handle
                .state::<Companion>()
                .run(&handle, String::new(), true)
                .await;
        });
        return;
    }
    let mut cfg = service.settings.lock().unwrap().clone();
    match id {
        "companion_input" => cfg.input_enabled = !cfg.input_enabled,
        "companion_speech" => cfg.speech_enabled = !cfg.speech_enabled,
        "companion_observation" => cfg.observation_enabled = !cfg.observation_enabled,
        _ => return,
    }
    service.cancel(app);
    if let Err(error) = service.save(cfg) {
        service.set_status(app, &error);
        return;
    }
    refresh_menu(app);
    let _ = sync(app);
    use tauri::Emitter;
    let _ = app.emit("companion-settings-changed", ());
}

pub fn refresh(app: &tauri::AppHandle) {
    refresh_menu(app);
    let _ = sync(app);
}
