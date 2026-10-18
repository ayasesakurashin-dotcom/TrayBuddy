use crate::modules::companion::{settings::Settings, store::Turn, Companion, View};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

fn local(window: &WebviewWindow) -> Result<(), String> {
    if ![
        "settings",
        "companion-input",
        "animation",
        "live2d",
        "pngremix",
        "threed",
    ]
    .contains(&window.label())
    {
        return Err("此窗口无权访问桌宠 AI".into());
    }
    Ok(())
}
fn settings_only(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("请在桌宠设置中管理 AI 配置与记忆".into());
    }
    Ok(())
}

#[tauri::command]
pub fn companion_get(window: WebviewWindow, state: State<'_, Companion>) -> Result<View, String> {
    local(&window)?;
    Ok(state.view())
}
#[tauri::command]
pub fn companion_set_input_collapsed(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Companion>,
    collapsed: bool,
) -> Result<bool, String> {
    if window.label() != "companion-input" {
        return Err("请从艾玛输入框展开或收起".into());
    }
    state.set_input_collapsed(collapsed)?;
    // The saved choice is authoritative; the tracking loop retries window sync.
    let _ = crate::modules::companion::sync_input_window(&app);
    Ok(collapsed)
}
#[tauri::command]
pub fn companion_save(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Companion>,
    settings: Settings,
) -> Result<View, String> {
    settings_only(&window)?;
    state.cancel(&app);
    state.save(settings)?;
    crate::modules::companion::refresh(&app);
    let _ = app.emit("companion-settings-changed", ());
    Ok(state.view())
}
#[tauri::command]
pub async fn companion_send(
    app: AppHandle,
    window: WebviewWindow,
    text: String,
) -> Result<(), String> {
    if window.label() != "companion-input" {
        return Err("请从艾玛输入框发送消息".into());
    }
    app.state::<Companion>().run(&app, text, false).await
}
#[tauri::command]
pub fn companion_stop(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Companion>,
) -> Result<(), String> {
    local(&window)?;
    state.cancel(&app);
    Ok(())
}
#[tauri::command]
pub fn companion_history(
    window: WebviewWindow,
    state: State<'_, Companion>,
) -> Result<Vec<Turn>, String> {
    local(&window)?;
    state.store.lock().unwrap().recent(200)
}
#[tauri::command]
pub async fn companion_forget(
    app: AppHandle,
    window: WebviewWindow,
    id: Option<String>,
) -> Result<(), String> {
    settings_only(&window)?;
    app.state::<Companion>().forget(&app, id).await
}
#[tauri::command]
pub async fn companion_observe(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    settings_only(&window)?;
    app.state::<Companion>()
        .run(&app, String::new(), true)
        .await
}
#[tauri::command]
pub async fn companion_test(
    app: AppHandle,
    window: WebviewWindow,
    kind: String,
) -> Result<String, String> {
    settings_only(&window)?;
    crate::modules::companion::test_connection(&app, &kind).await
}
#[tauri::command]
pub fn companion_playback_finished(
    window: WebviewWindow,
    state: State<'_, Companion>,
) -> Result<(), String> {
    local(&window)?;
    state
        .speaking_until
        .store(0, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}
#[tauri::command]
pub fn companion_blur(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "companion-input" {
        return Err("无效窗口".into());
    }
    crate::modules::companion::blur_input();
    Ok(())
}

#[tauri::command]
pub async fn companion_export(app: AppHandle, window: WebviewWindow) -> Result<String, String> {
    settings_only(&window)?;
    let rows = app.state::<Companion>().store.lock().unwrap().all()?;
    let text = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
    use tauri_plugin_dialog::DialogExt;
    tokio::task::spawn_blocking(move || {
        let file = app
            .dialog()
            .file()
            .add_filter("JSON", &["json"])
            .set_file_name("ema-companion-history.json")
            .blocking_save_file();
        if let Some(file) = file {
            let path = file.into_path().map_err(|_| "导出路径无效")?;
            std::fs::write(path, text).map_err(|e| e.to_string())?;
            Ok("已导出艾玛的全部记录".into())
        } else {
            Ok("已取消导出".into())
        }
    })
    .await
    .map_err(|_| "导出任务异常")?
}

#[tauri::command]
pub fn companion_voice_samples(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<crate::modules::companion::voice::Sample>, String> {
    settings_only(&window)?;
    crate::modules::companion::voice::samples(&app)
}
#[tauri::command]
pub fn companion_voice_sample_audio(
    app: AppHandle,
    window: WebviewWindow,
    name: String,
) -> Result<String, String> {
    settings_only(&window)?;
    use base64::Engine;
    let bytes = crate::modules::companion::voice::read(&app, &name)?;
    Ok(format!(
        "data:audio/wav;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
#[tauri::command]
pub async fn companion_voice_create(
    app: AppHandle,
    window: WebviewWindow,
    names: Vec<String>,
) -> Result<String, String> {
    settings_only(&window)?;
    crate::modules::companion::voice::create(&app, names).await
}
#[tauri::command]
pub async fn companion_speech_preview(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<String, String> {
    settings_only(&window)?;
    crate::modules::companion::speech_preview(&app).await
}
