use crate::modules::companion::{
    observations::{
        ChatSendPayload, Observation, PreparedScreenTarget, RequestReceipt, SendOutcome,
    },
    settings::Settings,
    store::HistoryTurn,
    Companion, View,
};
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
    let mut view=state.view();
    view.character_name=crate::modules::companion::character::resolve(window.app_handle(),&view.settings).display_name;
    Ok(view)
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
    crate::modules::care::user_activity(&app,"ai-settings",false);
    state.save(settings)?;
    crate::modules::companion::refresh(&app);
    let _ = app.emit("companion-settings-changed", ());
    Ok(state.view())
}
#[tauri::command]
pub async fn companion_send(
    app: AppHandle,
    window: WebviewWindow,
    payload: ChatSendPayload,
) -> Result<SendOutcome, String> {
    if window.label() != "companion-input" {
        return Err("请从艾玛输入框发送消息".into());
    }
    app.state::<Companion>().send(&app, payload).await
}
#[tauri::command]
pub fn companion_set_input_layout(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Companion>,
    expanded: bool,
) -> Result<(), String> {
    if window.label() != "companion-input" {
        return Err("无效窗口".into());
    }
    state
        .input_expanded
        .store(expanded, std::sync::atomic::Ordering::SeqCst);
    crate::modules::companion::sync_input_window(&app)
}
#[tauri::command]
pub fn companion_prepare_screen(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Companion>,
) -> Result<PreparedScreenTarget, String> {
    if window.label() != "companion-input" {
        return Err("无效窗口".into());
    }
    state.prepare_screen(&app)
}
#[tauri::command]
pub fn companion_request_receipt(
    window: WebviewWindow,
    state: State<'_, Companion>,
    client_message_id: String,
) -> Result<Option<RequestReceipt>, String> {
    if window.label() != "companion-input" {
        return Err("无效窗口".into());
    }
    state
        .store
        .lock()
        .unwrap()
        .request_receipt(&client_message_id)
}
#[tauri::command]
pub fn companion_get_observations(
    window: WebviewWindow,
    state: State<'_, Companion>,
    ids: Vec<String>,
) -> Result<Vec<Observation>, String> {
    local(&window)?;
    state.store.lock().unwrap().get_observations(&ids)
}
#[tauri::command]
pub fn companion_open_source(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Companion>,
    turn_id: String,
    source_id: String,
) -> Result<(), String> {
    local(&window)?;
    let record = state
        .store
        .lock()
        .unwrap()
        .web_context(&turn_id)?
        .ok_or("来源记录不可用")?;
    let source = record
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .and_then(|sources| {
            sources.iter().find(|s| {
                s.get("id").and_then(serde_json::Value::as_str) == Some(source_id.as_str())
            })
        })
        .ok_or("来源记录不可用")?;
    let url = source
        .get("url")
        .and_then(serde_json::Value::as_str)
        .and_then(crate::modules::companion::search::policy::public_url)
        .ok_or("来源链接无效")?;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "无法打开来源链接".into())
}
#[tauri::command]
pub fn companion_forget_observation_preview(
    window: WebviewWindow,
    state: State<'_, Companion>,
    id: String,
) -> Result<crate::modules::companion::store::ForgetObservationPreview, String> {
    settings_only(&window)?;
    state.store.lock().unwrap().preview_forget_observation(&id)
}
#[tauri::command]
pub fn companion_forget_observation(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Companion>,
    id: String,
    preview_token: String,
    expected_history_epoch: u64,
) -> Result<(), String> {
    settings_only(&window)?;
    state.cancel(&app);
    state
        .store
        .lock()
        .unwrap()
        .forget_observation(&id, &preview_token, expected_history_epoch)?;
    let _ = app.emit("companion-history-changed", ());
    Ok(())
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
) -> Result<Vec<HistoryTurn>, String> {
    local(&window)?;
    state.store.lock().unwrap().history(200)
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
pub fn companion_reading(
    window: WebviewWindow,
    state: State<'_, Companion>,
    request: u64,
    active: bool,
) -> Result<(), String> {
    if !["animation", "live2d", "pngremix", "threed"].contains(&window.label()) {
        return Err("此窗口无权更新桌宠阅读状态".into());
    }
    state.set_reading(request, active);
    crate::modules::care::user_activity(&window.app_handle().clone(), "reading", active);
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
    let rows = app.state::<Companion>().store.lock().unwrap().export()?;
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
