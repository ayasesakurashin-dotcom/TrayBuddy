use super::Companion;
use tauri::{
    Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

pub const INPUT_LABEL: &str = "companion-input";

#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct InputAnchor {
    center_x: f64,
    bottom: f64,
    viewport_width: f64,
    viewport_height: f64,
}

impl InputAnchor {
    pub fn valid(self) -> bool {
        [self.center_x, self.bottom, self.viewport_width, self.viewport_height]
            .iter().all(|value| value.is_finite())
            && self.viewport_width > 0.0 && self.viewport_height > 0.0
            && self.center_x >= 0.0 && self.center_x <= self.viewport_width
            && self.bottom > 0.0 && self.bottom <= self.viewport_height
    }

    fn physical(self, size: PhysicalSize<u32>, scale: f64) -> PhysicalPosition<i32> {
        // DOM measurements are relative CSS pixels. Map the actual viewport to
        // the client area once; screen origins must never be scaled again.
        PhysicalPosition::new(
            (self.center_x * size.width as f64 / self.viewport_width).round() as i32,
            (self.bottom * size.height as f64 / self.viewport_height + 8.0 * scale).round() as i32,
        )
    }
}

// Serialize visibility and geometry on the UI thread. In particular, a queued
// tracking update must read the latest setting instead of showing an input
// window again after the user has disabled it.
pub fn sync(app: &tauri::AppHandle) -> Result<(), String> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(error) = sync_now(&handle) {
            eprintln!("companion input sync: {error}");
        }
    })
    .map_err(|e| e.to_string())
}

/// Movement events take the short path: move an existing input without
/// recreating, resizing, or showing it on every drag frame.
pub fn follow(app: &tauri::AppHandle) -> Result<(), String> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(INPUT_LABEL) else {
            return;
        };
        let service = handle.state::<Companion>();
        if !service.settings.lock().unwrap().input_enabled || !Companion::allowed(&handle, false) {
            let _ = set_visible(&window, false);
            return;
        }
        let Some(pet) = crate::get_render_window(&handle) else {
            let _ = set_visible(&window, false);
            return;
        };
        if !pet.is_visible().unwrap_or(false) {
            let _ = set_visible(&window, false);
            return;
        }
        if !window.is_visible().unwrap_or(false) {
            return;
        }
        if let Ok(size) = window.outer_size() {
            if let Ok(target) = input_position(&pet, size) {
                if window.outer_position().ok() != Some(target) {
                    let _ = window.set_position(target);
                }
            }
        }
    })
    .map_err(|e| e.to_string())
}

fn set_visible(window: &WebviewWindow, visible: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};
        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        unsafe {
            // Pair native show/hide calls so hiding affects the actual HWND,
            // including a window previously shown without activation.
            let _ = ShowWindow(
                windows::Win32::Foundation::HWND(hwnd.0 as *mut _),
                if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if visible {
            window.show()
        } else {
            window.hide()
        }
        .map_err(|e| e.to_string())
    }
}

fn input_position(
    pet: &WebviewWindow,
    size: PhysicalSize<u32>,
) -> Result<PhysicalPosition<i32>, String> {
    let pos = pet.inner_position().map_err(|e| e.to_string())?;
    let pet_size = pet.inner_size().map_err(|e| e.to_string())?;
    let scale = pet.scale_factor().map_err(|e| e.to_string())?;
    let service = pet.state::<Companion>();
    let anchor = service.input_anchors.lock().unwrap().get(pet.label())
        .map(|anchor| anchor.physical(pet_size, scale));
    let area = pet
        .current_monitor()
        .map_err(|e| e.to_string())?
        .map(|monitor| {
            let area = monitor.work_area();
            (area.position, area.size)
        });
    Ok(place_input(pos, pet_size, size, area, anchor))
}

fn place_input(
    pet: PhysicalPosition<i32>,
    pet_size: PhysicalSize<u32>,
    input: PhysicalSize<u32>,
    area: Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>,
    anchor: Option<PhysicalPosition<i32>>,
) -> PhysicalPosition<i32> {
    // All values here are physical pixels, including negative monitor origins.
    let mut x = anchor.map_or(
        pet.x + (pet_size.width as i32 - input.width as i32) / 2,
        |anchor| pet.x + anchor.x - input.width as i32 / 2,
    );
    let mut y = pet.y + anchor.map_or(pet_size.height as i32, |anchor| anchor.y);
    if let Some((origin, size)) = area {
        let right = origin.x + size.width as i32;
        let bottom = origin.y + size.height as i32;
        x = x.clamp(origin.x, (right - input.width as i32).max(origin.x));
        // Keep the input in the work area without ever moving the pet itself.
        y = y.clamp(origin.y, (bottom - input.height as i32).max(origin.y));
    }
    PhysicalPosition::new(x, y)
}

fn sync_now(app: &tauri::AppHandle) -> Result<(), String> {
    let service = app.state::<Companion>();
    let (enabled, collapsed) = {
        let settings = service.settings.lock().unwrap();
        (settings.input_enabled, settings.input_collapsed)
    };
    let (width, height) = if collapsed { (132., 32.) } else if service.input_expanded.load(std::sync::atomic::Ordering::SeqCst) { (330., 110.) } else { (330., 84.) };
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
            set_visible(&window, false)?;
        }
        return Ok(());
    }
    let pet = pet.ok_or("桌宠窗口不可用")?;
    let window = if let Some(window) = app.get_webview_window(INPUT_LABEL) {
        window
    } else {
        let window =
            WebviewWindowBuilder::new(app, INPUT_LABEL, WebviewUrl::App("companion_input".into()))
                .title("桌宠对话")
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
                let _ = handle
                    .get_webview_window(INPUT_LABEL)
                    .map(|w| set_visible(&w, false));
                refresh_menu(&handle);
                use tauri::Emitter;
                let _ = handle.emit("companion-settings-changed", ());
            } else if matches!(event, tauri::WindowEvent::ScaleFactorChanged { .. }) {
                let _ = sync(&handle);
            }
        });
        window
    };
    let scale = pet.scale_factor().map_err(|e| e.to_string())?;
    let input_w = (width * scale).round() as u32;
    let input_h = (height * scale).round() as u32;
    let target = input_position(&pet, PhysicalSize::new(input_w, input_h))?;
    if window.outer_size().ok() != Some(PhysicalSize::new(input_w, input_h)) {
        window
            .set_size(PhysicalSize::new(input_w, input_h))
            .map_err(|e| e.to_string())?;
    }
    if window.outer_position().ok() != Some(target) {
        window.set_position(target).map_err(|e| e.to_string())?;
    }
    if !window.is_visible().unwrap_or(false) {
        set_visible(&window, true)?;
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
        "桌宠 AI",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_follows_pet_below_with_physical_pixel_centering() {
        assert_eq!(
            place_input(
                PhysicalPosition::new(300, 180),
                PhysicalSize::new(500, 450),
                PhysicalSize::new(495, 126),
                Some((PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1040))),
                None,
            ),
            PhysicalPosition::new(302, 630),
        );
    }

    #[test]
    fn bottom_edge_clamps_input_without_moving_the_pet() {
        assert_eq!(
            place_input(
                PhysicalPosition::new(400, 700),
                PhysicalSize::new(330, 300),
                PhysicalSize::new(330, 84),
                Some((PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1040))),
                None,
            ),
            PhysicalPosition::new(400, 956),
        );
    }

    #[test]
    fn input_clamps_to_negative_monitor_origin_and_small_work_areas() {
        assert_eq!(
            place_input(
                PhysicalPosition::new(-1600, -200),
                PhysicalSize::new(300, 250),
                PhysicalSize::new(330, 84),
                Some((
                    PhysicalPosition::new(-1440, -100),
                    PhysicalSize::new(1440, 960)
                )),
                None,
            ),
            PhysicalPosition::new(-1440, 50),
        );
        assert_eq!(
            place_input(
                PhysicalPosition::new(0, 0),
                PhysicalSize::new(300, 250),
                PhysicalSize::new(330, 84),
                Some((PhysicalPosition::new(0, 0), PhysicalSize::new(200, 60))),
                None,
            ),
            PhysicalPosition::new(0, 0),
        );
    }

    #[test]
    fn measured_model_bottom_ignores_the_transparent_window_reserve() {
        let anchor = InputAnchor {
            center_x: 250.0, bottom: 540.0,
            viewport_width: 500.0, viewport_height: 800.0,
        };
        assert!(anchor.valid());
        for scale in [1.0, 1.5, 2.0] {
            let size = PhysicalSize::new((500.0 * scale) as u32, (800.0 * scale) as u32);
            for (width, height) in [(330.0, 84.0), (330.0, 110.0), (132.0, 32.0)] {
                assert_eq!(
                    place_input(
                        PhysicalPosition::new(-1600, -100), size,
                        PhysicalSize::new((width * scale) as u32, (height * scale) as u32),
                        None, Some(anchor.physical(size, scale)),
                    ),
                    PhysicalPosition::new(
                        -1600 + (250.0 * scale) as i32 - (width * scale) as i32 / 2,
                        -100 + (548.0 * scale) as i32,
                    ),
                );
            }
        }
    }

    #[test]
    fn measured_anchor_moves_with_pet_and_clamps_to_work_area() {
        let size = PhysicalSize::new(500, 800);
        let input = PhysicalSize::new(330, 84);
        let area = Some((PhysicalPosition::new(-1920, -100), PhysicalSize::new(1920, 1040)));
        for pet_y in [-100, 500] {
            assert_eq!(
                place_input(PhysicalPosition::new(-1900, pet_y), size, input, area,
                    Some(PhysicalPosition::new(250, 548))),
                PhysicalPosition::new(-1815, (pet_y + 548).min(856)),
            );
        }
    }

    #[test]
    fn invalid_or_off_viewport_measurements_are_rejected() {
        let anchor = InputAnchor {
            center_x: 250.0, bottom: 540.0,
            viewport_width: 500.0, viewport_height: 800.0,
        };
        for invalid in [
            InputAnchor { center_x: f64::NAN, ..anchor },
            InputAnchor { bottom: f64::INFINITY, ..anchor },
            InputAnchor { viewport_width: 0.0, ..anchor },
            InputAnchor { viewport_height: -1.0, ..anchor },
            InputAnchor { center_x: 501.0, ..anchor },
            InputAnchor { bottom: 801.0, ..anchor },
        ] { assert!(!invalid.valid()); }
    }
}
