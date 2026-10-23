use super::*;
pub(super) enum Presentation {
    Ready(String),
    PresetBusy,
    Unavailable,
}
pub(super) fn presentation(app: &tauri::AppHandle) -> Presentation {
    let state = app.state::<crate::AppState>();
    let (id, visible) = {
        let s = state.storage.lock().unwrap();
        (
            s.data.info.current_mod.to_string(),
            s.data.settings.show_character && !s.data.settings.silence_mode,
        )
    };
    if !visible || state.session_locked.load(Ordering::SeqCst) {
        return Presentation::Unavailable;
    }
    if state.state_manager.lock().unwrap().is_locked() {
        return Presentation::PresetBusy;
    }
    if app
        .try_state::<crate::modules::companion::Companion>()
        .is_some_and(|c| c.interaction_active())
        || !crate::get_render_window(app).is_some_and(|w| w.is_visible().unwrap_or(false))
    {
        return Presentation::Unavailable;
    }
    Presentation::Ready(id)
}
pub(super) fn presentation_mod(app: &tauri::AppHandle) -> Option<String> {
    match presentation(app) {
        Presentation::Ready(id) => Some(id),
        _ => None,
    }
}
fn renderer(app: &tauri::AppHandle, window: &tauri::WebviewWindow) -> Result<(), String> {
    if crate::get_render_window(app).is_some_and(|w| w.label() == window.label()) {
        Ok(())
    } else {
        Err("当前渲染窗口不可用".into())
    }
}
#[tauri::command]
pub fn care_get_status(app: tauri::AppHandle) -> Status {
    app.state::<Arc<CareService>>().status()
}
#[tauri::command]
pub fn care_save_settings(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    settings: Settings,
) -> Result<Status, String> {
    environment_service::commands::settings_window(&window)?;
    let s = app.state::<Arc<CareService>>();
    s.save(settings)?;
    user_activity(&app, "settings", false);
    Ok(s.status())
}
#[tauri::command]
pub fn care_interaction_activity(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    active: bool,
) -> Result<(), String> {
    if window.label() != "companion-input" {
        renderer(&app, &window)?;
    }
    user_activity(
        &app,
        if window.label() == "companion-input" {
            "editing"
        } else {
            "character"
        },
        active,
    );
    Ok(())
}
#[tauri::command]
pub fn care_validate_offer(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    delivery_id: String,
    generation: u64,
) -> Result<bool, String> {
    renderer(&app, &window)?;
    let Some(mod_id) = presentation_mod(&app) else {
        return Ok(false);
    };
    let snapshot = app
        .state::<Arc<environment_service::EnvironmentService>>()
        .snapshot();
    let s = app.state::<Arc<CareService>>();
    let i = s.inner.lock().unwrap();
    Ok(i.mod_id == mod_id
        && i.settings.enabled
        && i.activities.is_empty()
        && mono() >= i.quiet_until
        && i.delivery.as_ref().is_some_and(|d| {
            d.offer.delivery_id == delivery_id
                && d.offer.generation == generation
                && d.offer.interaction_epoch == i.epoch
                && mono() < d.offered_mono + 2
                && candidate_valid(&d.candidate, &snapshot, &i.mod_id, now())
                && ai_current(&app, d)
        }))
}
#[tauri::command]
pub fn care_presented(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    delivery_id: String,
    generation: u64,
) -> Result<bool, String> {
    renderer(&app, &window)?;
    let Some(mod_id) = presentation_mod(&app) else {
        return Ok(false);
    };
    let snapshot = app
        .state::<Arc<environment_service::EnvironmentService>>()
        .snapshot();
    let s = app.state::<Arc<CareService>>();
    let mut i = s.inner.lock().unwrap();
    if !i.settings.enabled
        || !i.activities.is_empty()
        || mono() < i.quiet_until
        || i.mod_id != mod_id
        || !i.delivery.as_ref().is_some_and(|d| {
            d.offer.delivery_id == delivery_id
                && d.offer.generation == generation
                && d.offer.interaction_epoch == i.epoch
                && mono() < d.offered_mono + 2
                && candidate_valid(&d.candidate, &snapshot, &mod_id, now())
                && ai_current(&app, d)
        })
    {
        return Ok(false);
    }
    let result = i
        .ledger
        .as_ref()
        .ok_or("关怀记录库不可用")?
        .presented(&delivery_id, now());
    match result {
        Ok(shown) => {
            if shown {
                i.last_shown_mono = Some(mono());
                let d = i.delivery.as_ref().unwrap();
                let id = d.offer.delivery_id.clone();
                let text = d.offer.text.clone();
                let epoch = d.history_epoch;
                let _ = app
                    .state::<crate::modules::companion::Companion>()
                    .record_care(&id, &text, epoch);
                let _ = app.emit("companion-history-changed", ());
                i.delivery.as_mut().unwrap().presented = true;
            }
            Ok(shown)
        }
        Err(e) => {
            i.blocked = Some(e.clone());
            Err(e)
        }
    }
}
#[tauri::command]
pub fn care_rejected(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    delivery_id: String,
    generation: u64,
) -> Result<(), String> {
    renderer(&app, &window)?;
    let s = app.state::<Arc<CareService>>();
    let mut i = s.inner.lock().unwrap();
    if i.delivery.as_ref().is_some_and(|d| {
        d.offer.delivery_id == delivery_id && d.offer.generation == generation && !d.presented
    }) {
        i.ledger
            .as_ref()
            .ok_or("关怀记录库不可用")?
            .reject(&delivery_id)?;
        i.delivery = None;
    }
    Ok(())
}
#[tauri::command]
pub fn care_finished(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    delivery_id: String,
    generation: u64,
) -> Result<(), String> {
    renderer(&app, &window)?;
    let s = app.state::<Arc<CareService>>();
    let mut i = s.inner.lock().unwrap();
    if i.delivery
        .as_ref()
        .is_some_and(|d| d.offer.delivery_id == delivery_id && d.offer.generation == generation)
    {
        i.delivery = None;
    }
    Ok(())
}

fn ai_current(app: &tauri::AppHandle, d: &Delivery) -> bool {
    let c = app.state::<crate::modules::companion::Companion>();
    c.care_revision() == d.ai_revision
        && c.store.lock().unwrap().history_epoch().ok() == Some(d.history_epoch)
}
fn audio_current(app: &tauri::AppHandle, id: &str, generation: u64) -> bool {
    let Some(mod_id) = presentation_mod(app) else {
        return false;
    };
    let Some(s) = global() else { return false };
    let Some(e) = environment_service::global() else {
        return false;
    };
    let snapshot = e.snapshot();
    let i = s.inner.lock().unwrap();
    i.settings.enabled
        && i.activities.is_empty()
        && i.delivery.as_ref().is_some_and(|d| {
            d.presented
                && d.offer.delivery_id == id
                && d.offer.generation == generation
                && candidate_valid(&d.candidate, &snapshot, &mod_id, now())
                && ai_current(app, d)
        })
}
#[tauri::command]
pub fn care_calendar_get(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<calendar::Data, String> {
    environment_service::commands::settings_window(&window)?;
    let s = app.state::<Arc<CareService>>();
    let book = s.calendar.lock().unwrap();
    if let Some(e) = &book.error {
        return Err(e.clone());
    }
    Ok(book.data.clone())
}
fn calendar_changed(app: &tauri::AppHandle) {
    user_activity(app, "calendar", false);
    let _ = app.emit("care-calendar-changed", ());
}
#[tauri::command]
pub async fn care_calendar_import(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    country: String,
    first_year: i32,
    last_year: i32,
) -> Result<(), String> {
    environment_service::commands::settings_window(&window)?;
    let sets = import::download(&country, first_year, last_year).await?;
    app.state::<Arc<CareService>>()
        .calendar
        .lock()
        .unwrap()
        .edit(|d| {
            for set in sets {
                d.datasets.retain(|old| old.id != set.id);
                d.datasets.push(set);
            }
            Ok(())
        })?;
    calendar_changed(&app);
    Ok(())
}
#[tauri::command]
pub fn care_calendar_remove(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    environment_service::commands::settings_window(&window)?;
    app.state::<Arc<CareService>>()
        .calendar
        .lock()
        .unwrap()
        .edit(|d| {
            d.datasets.retain(|set| set.id != id);
            Ok(())
        })?;
    calendar_changed(&app);
    Ok(())
}
#[tauri::command]
pub fn care_special_save(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    date: calendar::SpecialDate,
) -> Result<calendar::SpecialDate, String> {
    environment_service::commands::settings_window(&window)?;
    let saved = app
        .state::<Arc<CareService>>()
        .calendar
        .lock()
        .unwrap()
        .save_date(date)?;
    calendar_changed(&app);
    Ok(saved)
}
#[tauri::command]
pub fn care_special_delete(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    environment_service::commands::settings_window(&window)?;
    app.state::<Arc<CareService>>()
        .calendar
        .lock()
        .unwrap()
        .edit(|d| {
            d.special_dates.retain(|s| s.id != id);
            Ok(())
        })?;
    calendar_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn care_audio_allowed(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    delivery_id: String,
    generation: u64,
) -> Result<bool, String> {
    renderer(&app, &window)?;
    let cfg = app
        .state::<crate::modules::companion::Companion>()
        .settings
        .lock()
        .unwrap()
        .clone();
    let audible = !app
        .state::<crate::AppState>()
        .storage
        .lock()
        .unwrap()
        .data
        .settings
        .no_audio_mode;
    Ok(cfg.speech_enabled && audible && audio_current(&app, &delivery_id, generation))
}
