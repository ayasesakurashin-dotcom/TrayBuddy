//! Independent companion AI; calls configured services directly, without other agents.
mod access;
mod capabilities;
pub mod character;
pub mod context;
mod diagnostics;
mod memory;
pub mod observations;
pub mod proactive;
pub(crate) mod providers;
mod reply_text;
mod request;
mod screen;
pub mod search;
pub mod settings;
pub mod store;
pub mod voice;
mod window;
pub use window::{
    follow as follow_input_window, menu_event, menu_items, refresh, sync as sync_input_window,
};

use crate::app_state::AppState;
use capabilities::CompanionContextSource;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use settings::{Identity, Secrets, Settings};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use store::{Store, Turn};
use tauri::{Emitter, Manager};

pub struct Companion {
    pub settings: Mutex<Settings>,
    pub store: Mutex<Store>,
    identity: Identity,
    pub dir: PathBuf,
    secrets: Secrets,
    busy: AtomicBool,
    generation: AtomicU64,
    memory_gate: tokio::sync::Mutex<()>,
    pub status: Mutex<String>,
    pub speaking_until: AtomicU64,
    reading_until: AtomicU64,
    blocked: Option<String>,
    save_gate: Mutex<()>,
    settings_revision: AtomicU64,
    pub input_expanded: AtomicBool,
    event_seq: AtomicU64,
    active_request: Mutex<Option<request::ActiveRequest>>,
    warning: Mutex<Option<String>>,
    notice: Mutex<Option<String>>,
    summary_busy: AtomicBool,
}
#[derive(Serialize)]
pub struct View {
    pub settings: Settings,
    pub bank_id: String,
    pub status: String,
    pub pending: i64,
    pub busy: bool,
    pub summary: String,
    pub history_epoch: u64,
    pub request: Option<u64>,
    pub last_observation: Option<observations::Observation>,
    pub character_name: String,
    pub warning: Option<String>,
    pub notice: Option<String>,
}
#[derive(Clone, Serialize)]
struct Reply {
    id: String,
    text: String,
    source: String,
    request: u64,
    done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    interaction_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<String>,
}

impl Companion {
    pub fn new(dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let identity = settings::load_identity(&dir)?;
        let settings = settings::load_settings(&dir)?;
        let store = Store::open(&dir.join("history.sqlite"), &identity.bank_id)?;
        Ok(Self {
            settings: Mutex::new(settings),
            store: Mutex::new(store),
            identity,
            secrets: Secrets(dir.clone()),
            dir,
            busy: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            memory_gate: tokio::sync::Mutex::new(()),
            status: Mutex::new("就绪，API 配置后可聊天".into()),
            speaking_until: AtomicU64::new(0),
            reading_until: AtomicU64::new(0),
            blocked: None,
            save_gate: Mutex::new(()),
            settings_revision: AtomicU64::new(0),
            input_expanded: AtomicBool::new(false),
            event_seq: AtomicU64::new(0),
            active_request: Mutex::new(None),
            warning: Mutex::new(None),
            notice: Mutex::new(None),
            summary_busy: AtomicBool::new(false),
        })
    }
    pub fn unavailable(error: String) -> Self {
        let mut value = Settings::default();
        value.input_enabled = false;
        value.observation_enabled = false;
        value.memory_enabled = false;
        Self {
            settings: Mutex::new(value),
            store: Mutex::new(
                Store::open(std::path::Path::new(":memory:"), "unavailable")
                    .expect("in-memory SQLite"),
            ),
            identity: Identity {
                bank_id: "未连接：请恢复桌宠身份或设置文件".into(),
                owner: String::new(),
            },
            dir: PathBuf::new(),
            secrets: Secrets(PathBuf::new()),
            busy: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            memory_gate: tokio::sync::Mutex::new(()),
            status: Mutex::new(format!("AI 已停用，原桌宠功能保留：{error}")),
            speaking_until: AtomicU64::new(0),
            reading_until: AtomicU64::new(0),
            blocked: Some(error),
            save_gate: Mutex::new(()),
            settings_revision: AtomicU64::new(0),
            input_expanded: AtomicBool::new(false),
            event_seq: AtomicU64::new(0),
            active_request: Mutex::new(None),
            warning: Mutex::new(None),
            notice: Mutex::new(None),
            summary_busy: AtomicBool::new(false),
        }
    }
    pub fn view(&self) -> View {
        let mut settings = self.settings.lock().unwrap().clone();
        settings.chat.has_key = self.secrets.has("chat");
        settings.vision.has_key = self.secrets.has("vision");
        settings.speech.has_key = self.secrets.has("speech");
        settings.memory.has_key = self.secrets.has("memory");
        let store = self.store.lock().unwrap();
        View {
            settings,
            bank_id: self.identity.bank_id.clone(),
            status: self.status.lock().unwrap().clone(),
            pending: store.pending_count().unwrap_or(0),
            busy: self.busy.load(Ordering::SeqCst),
            summary: format!("{}\n{}", store.profile(), store.summary())
                .trim()
                .into(),
            history_epoch: store.history_epoch().unwrap_or(0),
            request: self
                .active_request
                .lock()
                .unwrap()
                .as_ref()
                .map(|r| r.request),
            last_observation: store.latest_observation().ok().flatten(),
            character_name: "桌面伙伴".into(),
            warning: self.warning.lock().unwrap().clone(),
            notice: self.notice.lock().unwrap().clone(),
        }
    }
    pub fn ensure_ready(&self) -> Result<(), String> {
        if let Some(error) = &self.blocked {
            Err(format!("AI 已停用，原文件已保留：{error}"))
        } else {
            Ok(())
        }
    }
    pub fn save(&self, mut value: Settings) -> Result<(), String> {
        let _save = self.save_gate.lock().unwrap();
        if let Some(error) = &self.blocked {
            return Err(format!("AI 数据需先恢复，未覆盖原文件：{error}"));
        }
        // Only the dedicated input command changes this preference. A settings
        // page opened before a collapse must not overwrite the user's choice.
        value.input_collapsed = self.settings.lock().unwrap().input_collapsed;
        value.validate()?;
        for (kind, ep) in [
            ("chat", &mut value.chat),
            ("vision", &mut value.vision),
            ("speech", &mut value.speech),
            ("memory", &mut value.memory),
        ] {
            self.secrets.set(kind, &ep.api_key)?;
            ep.api_key.clear();
        }
        settings::atomic_write(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )?;
        *self.settings.lock().unwrap() = value;
        self.settings_revision.fetch_add(1, Ordering::SeqCst);
        screen::invalidate_targets();
        Ok(())
    }
    pub fn set_input_collapsed(&self, collapsed: bool) -> Result<(), String> {
        let _save = self.save_gate.lock().unwrap();
        self.ensure_ready()?;
        let mut value = self.settings.lock().unwrap().clone();
        if value.input_collapsed == collapsed {
            return Ok(());
        }
        value.input_collapsed = collapsed;
        settings::atomic_write(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )?;
        *self.settings.lock().unwrap() = value;
        Ok(())
    }
    pub fn cancel(&self, app: &tauri::AppHandle) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.speaking_until.store(0, Ordering::SeqCst);
        self.reading_until.store(0, Ordering::SeqCst);
        let _ = app.emit("companion-stop", ());
        screen::invalidate_targets();
        crate::modules::care::user_activity(app, "request", false);
    }
    pub fn set_reading(&self, request: u64, active: bool) {
        self.set_reading_at(request, active, chrono::Utc::now().timestamp() as u64);
    }
    fn set_reading_at(&self, request: u64, active: bool, now: u64) {
        if self.current(request) {
            // Each displayed page renews a short lease. A closed or crashed
            // renderer cannot leave automatic observation paused indefinitely.
            self.reading_until.store(
                if active { now.saturating_add(30) } else { 0 },
                Ordering::SeqCst,
            );
        }
    }
    fn reading(&self, now: u64) -> bool {
        self.reading_until.load(Ordering::SeqCst) > now
    }
    pub fn interaction_active(&self) -> bool {
        let now = chrono::Utc::now().timestamp().max(0) as u64;
        self.busy.load(Ordering::SeqCst)
            || self.reading(now)
            || self.speaking_until.load(Ordering::SeqCst) > now
    }
    fn current(&self, request: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == request
    }
    fn set_status(&self, app: &tauri::AppHandle, message: &str) {
        if self.busy.load(Ordering::SeqCst) {
            return;
        }
        *self.status.lock().unwrap() = message.into();
        let _ = app.emit(
            "companion-status",
            json!({"message":message,"busy":self.busy.load(Ordering::SeqCst)}),
        );
    }
    fn memory(&self, settings: &Settings) -> Result<memory::MemoryClient, String> {
        memory::MemoryClient::new(
            &settings.memory.base_url,
            &self.identity,
            self.secrets.get("memory")?,
        )
    }
    fn allowed(app: &tauri::AppHandle, automatic: bool) -> bool {
        if app.state::<Companion>().blocked.is_some() {
            return false;
        }
        let state = app.state::<AppState>();
        if state.session_locked.load(Ordering::SeqCst) {
            return false;
        }
        access::allowed(
            &state.storage,
            automatic,
            |storage| {
                !storage.data.info.current_mod.is_empty()
                    && !storage.data.settings.silence_mode
                    && storage.data.settings.show_character
            },
            || {
                crate::get_render_window(app)
                    .map(|window| window.is_visible().unwrap_or(false))
                    .unwrap_or(false)
            },
        )
    }
    fn audible(app: &tauri::AppHandle) -> bool {
        !app.state::<AppState>()
            .storage
            .lock()
            .unwrap()
            .data
            .settings
            .no_audio_mode
    }

    fn emit_reply(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        text: &str,
        observe: bool,
        request: u64,
        done: bool,
        error: Option<String>,
    ) {
        self.emit_presentation(app, id, text, observe, request, done, error, None);
    }
    fn emit_presentation(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        text: &str,
        observe: bool,
        request: u64,
        done: bool,
        error: Option<String>,
        audio: Option<String>,
    ) {
        let interaction_kind = self
            .active_request
            .lock()
            .unwrap()
            .as_ref()
            .filter(|r| r.request == request)
            .map(|r| match r.kind {
                "screen_question" => "screen_question",
                "observe_once" | "observe_auto" => "observation_comment",
                _ => "chat",
            })
            .unwrap_or("chat")
            .to_string();
        let finish_kind = if done {
            self.store
                .lock()
                .unwrap()
                .metadata(id)
                .ok()
                .map(|m| m.finish_kind)
        } else {
            None
        };
        if let Some(window) = crate::get_render_window(app) {
            let _ = app.emit_to(
                window.label(),
                "companion-reply",
                Reply {
                    id: id.into(),
                    text: text.into(),
                    source: if observe { "screen" } else { "chat" }.into(),
                    request,
                    done,
                    error,
                    interaction_kind,
                    finish_kind,
                    audio,
                },
            );
        }
    }
    fn speech_ready(&self, app: &tauri::AppHandle, settings: &Settings) -> bool {
        settings.speech_enabled
            && Self::audible(app)
            && !settings.speech.base_url.is_empty()
            && !character::resolve(app, settings).voice_id.is_empty()
            && self.secrets.has("speech")
    }
    async fn speech(
        &self,
        app: &tauri::AppHandle,
        settings: &Settings,
        id: &str,
        text: &str,
        request: u64,
    ) -> Result<Option<String>, String> {
        if !self.speech_ready(app, settings) {
            return Ok(None);
        }
        let prepare = self.prepare_speech(app, settings, id, text, request);
        tokio::pin!(prepare);
        // Turning audio off releases buffered text even if the provider stalls.
        let deadline = tokio::time::sleep(Duration::from_secs(45));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                result = &mut prepare => return result,
                _ = &mut deadline => return Err("语音请求超时".into()),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if !Self::audible(app) { return Ok(None); }
                }
            }
        }
    }
    async fn prepare_speech(
        &self,
        app: &tauri::AppHandle,
        settings: &Settings,
        id: &str,
        text: &str,
        request: u64,
    ) -> Result<Option<String>, String> {
        diagnostics::record(&self.dir, "translation_start", Some(request), None, None);
        let role = character::resolve(app, settings);
        let jp = if matches!(role.voice_locale.as_str(), "original" | "zh") {
            text.to_string()
        } else {
            providers::translate_for_role(settings, &self.secrets, text, &role).await?
        };
        let mut settings = settings.clone();
        settings.voice_id = role.voice_id;
        if !self.current(request) || !Self::audible(app) || !Self::allowed(app, false) {
            return Ok(None);
        }
        self.store.lock().unwrap().japanese(id, &jp)?;
        diagnostics::record(&self.dir, "speech_start", Some(request), None, None);
        let bytes = providers::synthesize(&settings, &self.secrets, &jp).await?;
        diagnostics::record(&self.dir, "speech_done", Some(request), Some(true), None);
        if self.current(request) && Self::audible(app) && Self::allowed(app, false) {
            use base64::Engine;
            self.speaking_until.store(
                (chrono::Utc::now().timestamp() + 60) as u64,
                Ordering::SeqCst,
            );
            return Ok(Some(
                base64::engine::general_purpose::STANDARD.encode(bytes),
            ));
        }
        Ok(None)
    }
    pub async fn sync_one(&self) -> Result<(), String> {
        if !self.settings.lock().unwrap().memory_enabled {
            return Ok(());
        }
        let _gate = self.memory_gate.lock().await;
        let settings = self.settings.lock().unwrap().clone();
        if !settings.memory_enabled {
            return Ok(());
        }
        let revision = self.settings_revision.load(Ordering::SeqCst);
        let future = self.sync_snapshot(&settings);
        tokio::pin!(future);
        loop {
            tokio::select! {result=&mut future=>return result,_=tokio::time::sleep(Duration::from_millis(100))=>{if self.settings_revision.load(Ordering::SeqCst)!=revision{return Ok(());}}}
        }
    }
    async fn sync_snapshot(&self, settings: &Settings) -> Result<(), String> {
        let client = self.memory(settings)?;
        if self.store.lock().unwrap().needs_rebuild() {
            let epoch = self.store.lock().unwrap().history_epoch()?;
            client.delete_owned_bank().await?;
            let mut store = self.store.lock().unwrap();
            if store.history_epoch()? != epoch {
                return Ok(());
            }
            store.rebuilt()?;
        }
        let pending = self.store.lock().unwrap().pending()?;
        if let Some(item) = pending {
            if item.submitted {
                let operation = client.operation(&item.operation_id).await?;
                let store = self.store.lock().unwrap();
                if store.history_epoch()? != item.history_epoch {
                    return Ok(());
                }
                match operation.as_str() {
                    "completed" => {
                        store.sync_state_at_epoch(&item.turn.id, "done", item.history_epoch)?
                    }
                    "failed" | "cancelled" | "missing" => store.retry_operation(&item.turn.id)?,
                    _ => {}
                }
            } else {
                client.submit(&item).await?;
                let store = self.store.lock().unwrap();
                if store.history_epoch()? != item.history_epoch {
                    return Ok(());
                }
                store.sync_state_at_epoch(&item.turn.id, "submitted", item.history_epoch)?;
            }
        }
        let profile_due = self.store.lock().unwrap().profile_due()?;
        if profile_due && !self.store.lock().unwrap().needs_rebuild() {
            let epoch = self.store.lock().unwrap().history_epoch()?;
            let profile = client.reflect_profile().await?;
            let store = self.store.lock().unwrap();
            if !store.needs_rebuild() && store.history_epoch()? == epoch {
                store.set_profile(&profile)?;
            }
        }
        Ok(())
    }
    pub async fn forget(&self, app: &tauri::AppHandle, id: Option<String>) -> Result<(), String> {
        self.cancel(app);
        // Apply the local tombstone immediately; remote cleanup is serialized by
        // the sync worker, without letting a slow request delay local forgetting.
        self.store.lock().unwrap().forget(id.as_deref())?;
        self.set_status(app, "已从本机遗忘；长期记忆正在重建，旧记忆检索已暂停");
        let _ = app.emit("companion-history-changed", ());
        Ok(())
    }
}

pub fn start(app: tauri::AppHandle) {
    screen::start_tracking(&app);
    diagnostics::record(
        &app.state::<Companion>().dir,
        "session_start",
        None,
        None,
        None,
    );
    let _ = window::sync(&app);
    // UI/window work stays on Tauri's main thread; network and storage work stay off it.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_observation = Instant::now();
        let mut last_window_sync = Instant::now();
        let mut access_signature = String::new();
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let service = handle.state::<Companion>();
            screen::track_foreground();
            let signature = {
                let app_state = handle.state::<AppState>();
                let storage = app_state.storage.lock().unwrap();
                format!(
                    "{}:{}:{}:{}",
                    storage.data.info.current_mod,
                    storage.data.settings.show_character,
                    storage.data.settings.silence_mode,
                    app_state.session_locked.load(Ordering::SeqCst)
                )
            };
            if access_signature != signature {
                if !access_signature.is_empty() {
                    service.cancel(&handle);
                    crate::modules::care::user_activity(&handle, "role-or-visibility", false);
                }
                screen::invalidate_targets();
                access_signature = signature;
            }
            let cfg = service.settings.lock().unwrap().clone();
            if last_window_sync.elapsed() >= Duration::from_secs(1) {
                last_window_sync = Instant::now();
                let _ = window::sync(&handle);
            }
            if !cfg.observation_enabled {
                last_observation = Instant::now();
                continue;
            }
            if last_observation.elapsed() >= Duration::from_secs(cfg.observation_interval_secs) {
                if service.busy.load(Ordering::SeqCst)
                    || service.reading(chrono::Utc::now().timestamp() as u64)
                    || service.speaking_until.load(Ordering::SeqCst)
                        > chrono::Utc::now().timestamp() as u64
                {
                    continue;
                }
                last_observation = Instant::now();
                diagnostics::record(&service.dir, "automatic_due", None, None, None);
                let eligible = Companion::allowed(&handle, true);
                diagnostics::record(
                    &service.dir,
                    "automatic_gate_done",
                    None,
                    Some(eligible),
                    None,
                );
                if eligible {
                    let request_handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = request_handle
                            .state::<Companion>()
                            .run_auto(&request_handle)
                            .await;
                    });
                }
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        let mut backoff = 2u64;
        loop {
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            let service = app.state::<Companion>();
            match service.sync_one().await {
                Ok(()) => backoff = 2,
                Err(error) => {
                    backoff = (backoff * 2).min(60);
                    service.set_status(&app, &error);
                }
            }
        }
    });
}

#[cfg(test)]
mod input_preference_tests {
    use super::*;

    #[test]
    fn reading_pages_renew_a_request_tagged_lease_and_finish_independently_of_audio() {
        let dir = tempfile::tempdir().unwrap();
        let service = Companion::new(dir.path().into()).unwrap();
        service.generation.store(7, Ordering::SeqCst);
        service.set_reading_at(7, true, 100);
        assert!(service.reading(129));
        assert!(!service.reading(130));
        service.set_reading_at(7, true, 120);
        assert!(service.reading(149));
        service.set_reading_at(6, false, 121);
        assert!(service.reading(149));
        service.speaking_until.store(500, Ordering::SeqCst);
        service.set_reading_at(7, false, 122);
        assert!(!service.reading(122));
        assert_eq!(service.speaking_until.load(Ordering::SeqCst), 500);
        service.generation.store(8, Ordering::SeqCst);
        service.set_reading_at(7, true, 123);
        assert!(!service.reading(123));
    }

    #[test]
    fn collapse_survives_restart_and_stale_settings_without_cancelling_a_reply() {
        let legacy: Settings = serde_json::from_str(r#"{"input_enabled":true}"#).unwrap();
        assert!(!legacy.input_collapsed);
        let dir = tempfile::tempdir().unwrap();
        let service = Companion::new(dir.path().into()).unwrap();
        let bank = service.identity.bank_id.clone();
        let mut stale_settings = service.settings.lock().unwrap().clone();
        service.busy.store(true, Ordering::SeqCst);
        service.generation.store(7, Ordering::SeqCst);
        service.set_input_collapsed(true).unwrap();
        assert!(service.busy.load(Ordering::SeqCst));
        assert_eq!(service.generation.load(Ordering::SeqCst), 7);
        stale_settings.nickname = "测试称呼".into();
        service.save(stale_settings).unwrap();
        drop(service);
        let reopened = Companion::new(dir.path().into()).unwrap();
        assert_eq!(reopened.identity.bank_id, bank);
        let settings = reopened.settings.lock().unwrap().clone();
        assert!(settings.input_enabled && settings.input_collapsed);
        assert_eq!(settings.nickname, "测试称呼");
        reopened.set_input_collapsed(false).unwrap();
        assert!(!settings::load_settings(dir.path()).unwrap().input_collapsed);
    }
}

pub fn blur_input() {
    screen::restore_focus();
}
pub(crate) fn shutdown(app: &tauri::AppHandle) {
    if let Some(service) = app.try_state::<Companion>() {
        service.cancel(app);
    }
    screen::stop_tracking();
}

pub async fn speech_preview(app: &tauri::AppHandle) -> Result<String, String> {
    let service = app.state::<Companion>();
    service.ensure_ready()?;
    if !Companion::audible(app) {
        return Err("请先关闭全局静音再试听".into());
    }
    let mut cfg = service.settings.lock().unwrap().clone();
    let role = character::resolve(app, &cfg);
    cfg.voice_id = role.voice_id.clone();
    let bytes = providers::synthesize(&cfg, &service.secrets, &role.preview_text()).await?;
    use base64::Engine;
    Ok(format!(
        "data:audio/mpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

pub async fn test_connection(app: &tauri::AppHandle, kind: &str) -> Result<String, String> {
    let service = app.state::<Companion>();
    let mut cfg = service.settings.lock().unwrap().clone();
    let role = character::resolve(app, &cfg);
    cfg.voice_id = role.voice_id.clone();
    service.ensure_ready()?;
    match kind {
        "memory" => {
            // Read-only test: no listing other banks, no creating a bank merely by testing.
            let exists = service.memory(&cfg)?.ensure(false).await?;
            Ok(if exists {
                "桌宠专用 bank 身份验证通过"
            } else {
                "记忆服务可连接，专用 bank 将在首次记忆写入时创建"
            }
            .into())
        }
        "chat" => providers::completion(
            &cfg.chat,
            &service.secrets.get("chat")?,
            vec![json!({"role":"user","content":"只回复：连接成功。"})],
            30,
            false,
        )
        .await
        .map(|_| "对话模型连接成功".into()),
        "vision" => {
            // Connection validation without taking a screenshot or uploading user content.
            providers::completion(
                &cfg.vision,
                &service.secrets.get("vision")?,
                vec![json!({"role":"user","content":"只回复：连接成功。"})],
                30,
                false,
            )
            .await
            .map(|_| "模型连接成功，图像能力需通过手动观察验证".into())
        }
        "speech" => {
            let _ = providers::synthesize(&cfg, &service.secrets, &role.preview_text()).await?;
            Ok("语音合成连接成功".into())
        }
        _ => Err("未知连接类型".into()),
    }
}
