use super::{
    types::{City, Settings, Snapshot},
    EnvironmentService,
};
use std::sync::Arc;
use tauri::Manager;
pub fn settings_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == crate::modules::constants::WINDOW_LABEL_SETTINGS {
        Ok(())
    } else {
        Err("仅设置窗口可修改此项".into())
    }
}
fn diagnostic_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "settings" | "main") {
        Ok(())
    } else {
        Err("仅设置或调试窗口可刷新环境".into())
    }
}
#[tauri::command]
pub fn environment_get_snapshot(app: tauri::AppHandle) -> Snapshot {
    app.state::<Arc<EnvironmentService>>().snapshot()
}
#[tauri::command]
pub async fn environment_search_cities(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    query: String,
) -> Result<Vec<City>, String> {
    settings_window(&window)?;
    app.state::<Arc<EnvironmentService>>()
        .find_cities(&query)
        .await
}
#[tauri::command]
pub async fn environment_save_settings(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    settings: Settings,
) -> Result<Snapshot, String> {
    settings_window(&window)?;
    let service = app.state::<Arc<EnvironmentService>>().inner().clone();
    service.save(settings)?;
    super::super::care::user_activity(&app, "environment", false);
    service.notify(&app);
    let a = app.clone();
    let s = service.clone();
    tauri::async_runtime::spawn(async move {
        s.refresh_location(&a, false).await;
        s.refresh_weather(&a).await;
    });
    Ok(service.snapshot())
}
#[tauri::command]
pub async fn environment_refresh_weather(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Snapshot, String> {
    diagnostic_window(&window)?;
    let s = app.state::<Arc<EnvironmentService>>().inner().clone();
    s.refresh_location(&app, false).await;
    s.refresh_weather(&app).await;
    Ok(s.snapshot())
}
#[tauri::command]
pub async fn environment_refresh_location(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Snapshot, String> {
    diagnostic_window(&window)?;
    let s = app.state::<Arc<EnvironmentService>>().inner().clone();
    s.refresh_location(&app, true).await;
    s.refresh_weather(&app).await;
    Ok(s.snapshot())
}
#[tauri::command]
pub async fn environment_confirm_ip_city(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    key: String,
    revision: u64,
) -> Result<Snapshot, String> {
    settings_window(&window)?;
    let s = app.state::<Arc<EnvironmentService>>().inner().clone();
    s.confirm_ip(&key, revision)?;
    super::super::care::user_activity(&app, "environment", false);
    s.refresh_weather(&app).await;
    Ok(s.snapshot())
}
#[tauri::command]
pub async fn environment_request_location_access(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Snapshot, String> {
    settings_window(&window)?;
    if !window.is_focused().unwrap_or(false) {
        return Err("请在前台设置窗口申请定位权限".into());
    }
    let service = app.state::<Arc<EnvironmentService>>().inner().clone();
    let generation = {
        let i = service.inner.lock().unwrap();
        if !i.settings.device_location_enabled || i.settings.location_mode != "automatic" {
            return Err("请先开启自动模式的设备定位".into());
        }
        i.config_revision
    };
    #[cfg(windows)]
    let access = {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let result = if window.is_focused().unwrap_or(false) {
                super::windows_runtime::request_location_access()
                    .map_err(|_| "系统定位申请不可用".to_string())
            } else {
                Err("请保持设置窗口在前台".into())
            };
            let _ = tx.send(result);
        })
        .map_err(|_| "系统定位申请不可用")?;
        let operation = rx.await.map_err(|_| "系统定位申请已取消")??;
        match super::providers::wait_winrt(operation, 120).await {
            Ok(v) if v == windows::Devices::Geolocation::GeolocationAccessStatus::Allowed => {
                "allowed"
            }
            Ok(_) => "denied",
            Err(_) => "unavailable",
        }
    };
    #[cfg(not(windows))]
    let access = "unavailable";
    {
        let mut i = service.inner.lock().unwrap();
        if i.config_revision != generation || !i.settings.device_location_enabled {
            drop(i);
            return Ok(service.snapshot());
        }
        i.device_access = access.into();
        EnvironmentService::invalidate(&mut i);
    }
    service.refresh_location(&app, true).await;
    service.refresh_weather(&app).await;
    service.notify(&app);
    Ok(service.snapshot())
}
