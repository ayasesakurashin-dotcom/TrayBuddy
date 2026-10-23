//! Generic low-priority care: evidence -> candidate -> model -> validated display.
mod api;
pub mod calendar;
pub mod dates;
pub mod import;
pub mod ledger;
pub mod profile;
pub mod weather;
use crate::modules::environment_service::{self, settings as files, types::Snapshot};
pub use api::*;
use chrono::{TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tauri::{Emitter, Manager};
static GLOBAL: OnceLock<Arc<CareService>> = OnceLock::new();
pub fn global() -> Option<Arc<CareService>> {
    GLOBAL.get().cloned()
}
/// Native login/random activities preempt care without becoming user interaction,
/// changing care frequency, or entering the conversation/memory pipeline.
pub fn yield_to_preset(app: &tauri::AppHandle) {
    let Some(service) = global() else { return };
    let generation = {
        let mut i = service.inner.lock().unwrap();
        i.eligible_before = false;
        if !cancel_presentation(&mut i) {
            return;
        }
        i.generation
    };
    let _ = app.emit("care-cancel", serde_json::json!({"generation":generation}));
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub enabled: bool,
    pub rain: bool,
    pub legacy_weather: bool,
    pub night: bool,
    pub holiday: bool,
    pub special_dates: bool,
    pub capture_dates: bool,
    pub solar_terms: bool,
    pub cold_threshold: f64,
    pub hot_threshold: f64,
    pub night_start: u32,
    pub night_end: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: false,
            rain: true,
            legacy_weather: true,
            night: true,
            holiday: true,
            special_dates: true,
            capture_dates: true,
            solar_terms: false,
            cold_threshold: 5.,
            hot_threshold: 30.,
            night_start: 1380,
            night_end: 120,
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.night_start >= 1440
            || self.night_end >= 1440
            || self.night_start == self.night_end
            || !self.cold_threshold.is_finite()
            || !self.hot_threshold.is_finite()
            || self.cold_threshold >= self.hot_threshold
            || self.cold_threshold < -60.
            || self.hot_threshold > 60.
        {
            return Err("关怀时段或温度阈值无效".into());
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Candidate {
    event: String,
    kind: String,
    identity: String,
    mod_id: String,
    location_revision: u64,
    observation: Option<i64>,
    expires: i64,
    ready_mono: u64,
    event_until: i64,
    text: String,
    temp_range: Option<(f64, f64)>,
    calendar_version: Option<String>,
    clock_revision: u64,
}
#[derive(Clone, Serialize)]
pub struct Offer {
    pub delivery_id: String,
    pub generation: u64,
    pub interaction_epoch: u64,
    pub text: String,
    pub expires_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
}
struct Delivery {
    offer: Offer,
    candidate: Candidate,
    offered_mono: u64,
    presented: bool,
    history_epoch: u64,
    ai_revision: u64,
}
struct Generating {
    generation: u64,
    candidate: Candidate,
    cancel: Arc<AtomicBool>,
}
struct Inner {
    settings: Settings,
    blocked: Option<String>,
    last_error: Option<String>,
    ledger: Option<ledger::Ledger>,
    generation: u64,
    epoch: u64,
    quiet_until: u64,
    activities: HashMap<String, u64>,
    candidates: Vec<Candidate>,
    seen: HashSet<String>,
    delivery: Option<Delivery>,
    generating: Option<Generating>,
    mod_id: String,
    eligible_before: bool,
    eligibility_epoch: u64,
    last_shown_mono: Option<u64>,
}
fn cancel_presentation(i: &mut Inner) -> bool {
    if i.generating.is_none() && i.delivery.is_none() {
        return false;
    }
    if let Some(job) = i.generating.take() {
        job.cancel.store(true, Ordering::SeqCst);
    }
    i.delivery = None;
    i.generation += 1;
    true
}
#[derive(Serialize)]
pub struct Status {
    pub settings: Settings,
    pub reason: Option<String>,
    pub candidates: usize,
    pub presenting: bool,
    pub generating: bool,
}
pub struct CareService {
    inner: Mutex<Inner>,
    pub calendar: Mutex<calendar::Book>,
    dir: PathBuf,
    started: AtomicBool,
    stopped: AtomicBool,
}
fn mono() -> u64 {
    environment_service::global().map_or(0, |s| s.clock.monotonic())
}
fn now() -> i64 {
    environment_service::global().map_or_else(|| chrono::Utc::now().timestamp(), |s| s.clock.utc())
}
fn random_number() -> u64 {
    let mut bytes = [0u8; 8];
    let _ = getrandom::getrandom(&mut bytes);
    u64::from_le_bytes(bytes)
}
fn random_delay() -> u64 {
    20 + random_number() % 101
}
pub fn user_activity(app: &tauri::AppHandle, kind: &str, active: bool) {
    let Some(s) = GLOBAL.get() else { return };
    let mut i = s.inner.lock().unwrap();
    i.epoch += 1;
    i.generation += 1;
    i.quiet_until = mono() + 120;
    i.candidates.clear();
    i.eligible_before = false;
    if active {
        i.activities.insert(
            kind.into(),
            mono()
                + match kind {
                    "request" => 600,
                    "reading" => 35,
                    _ => 15,
                },
        );
    } else {
        i.activities.remove(kind);
    }
    if let Some(job) = i.generating.take() {
        job.cancel.store(true, Ordering::SeqCst);
    }
    let delivery = i.delivery.take();
    let generation = i.generation;
    drop(i);
    let _=app.emit("care-cancel",serde_json::json!({"generation":generation,"delivery_id":delivery.map(|d|d.offer.delivery_id)}));
}
impl CareService {
    pub fn new(dir: PathBuf) -> Self {
        let loaded = files::load::<Settings>(&dir.join("settings.json")).and_then(|s| {
            s.validate()?;
            Ok(s)
        });
        let mut blocked = loaded.as_ref().err().cloned();
        let settings = loaded.unwrap_or_default();
        let ledger = match ledger::Ledger::open(
            &dir.join("ledger.sqlite"),
            chrono::Utc::now().timestamp(),
        ) {
            Ok(l) => Some(l),
            Err(e) => {
                blocked = Some(e);
                None
            }
        };
        let calendar = Mutex::new(calendar::Book::new(dir.join("calendar.json")));
        Self {
            dir,
            calendar,
            inner: Mutex::new(Inner {
                settings,
                blocked,
                last_error: None,
                ledger,
                generation: 0,
                epoch: 0,
                quiet_until: 120,
                activities: HashMap::new(),
                candidates: vec![],
                seen: HashSet::new(),
                delivery: None,
                generating: None,
                mod_id: String::new(),
                eligible_before: false,
                eligibility_epoch: 0,
                last_shown_mono: None,
            }),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(j) = self.inner.lock().unwrap().generating.take() {
            j.cancel.store(true, Ordering::SeqCst);
        }
    }
    pub fn status(&self) -> Status {
        let i = self.inner.lock().unwrap();
        Status {
            settings: i.settings.clone(),
            reason: i.blocked.clone().or_else(|| {
                if !i.settings.enabled {
                    Some("自动关怀已关闭（原角色关怀也保持关闭）".into())
                } else if mono() < i.quiet_until || !i.activities.is_empty() {
                    Some("等待用户交互结束后的安静时段".into())
                } else {
                    i.last_error.clone()
                }
            }),
            candidates: i.candidates.len(),
            presenting: i.delivery.is_some(),
            generating: i.generating.is_some(),
        }
    }
    fn save(&self, s: Settings) -> Result<(), String> {
        s.validate()?;
        files::save(&self.dir.join("settings.json"), &s)?;
        self.inner.lock().unwrap().settings = s;
        Ok(())
    }
    fn tick(&self, app: &tauri::AppHandle) {
        let Some(env) = environment_service::global() else {
            return;
        };
        let snapshot = env.snapshot();
        let time = now();
        let monotonic = mono();
        let mod_id = match api::presentation(app) {
            api::Presentation::Ready(id) => id,
            api::Presentation::PresetBusy => {
                yield_to_preset(app);
                return;
            }
            api::Presentation::Unavailable => {
                let mut i = self.inner.lock().unwrap();
                i.eligible_before = false;
                if i.delivery.is_some() || i.generating.is_some() || !i.candidates.is_empty() {
                    drop(i);
                    user_activity(app, "unavailable", false);
                }
                return;
            }
        };
        let book = self.calendar.lock().unwrap().data.clone();
        let mut i = self.inner.lock().unwrap();
        if mod_id != i.mod_id {
            i.mod_id = mod_id.clone();
            drop(i);
            user_activity(app, "role", false);
            return;
        }
        i.activities.retain(|_, expires| *expires > monotonic);
        let ai_ready = app
            .try_state::<super::companion::Companion>()
            .is_some_and(|c| c.care_ready());
        if !i.settings.enabled
            || i.blocked.is_some()
            || !i.activities.is_empty()
            || monotonic < i.quiet_until
            || !ai_ready
        {
            i.eligible_before = false;
            if !ai_ready {
                i.last_error = Some("请先配置可用的对话模型；自动关怀需要模型生成内容".into());
            }
            return;
        }
        if !i.eligible_before {
            i.eligibility_epoch += 1;
            i.eligible_before = true;
        }
        if let Some(job) = &i.generating {
            if !candidate_valid(&job.candidate, &snapshot, &mod_id, time) {
                drop(i);
                user_activity(app, "facts-changed", false);
            }
            return;
        }
        if let Some(d) = &i.delivery {
            if !candidate_valid(&d.candidate, &snapshot, &mod_id, time)
                || (!d.presented && monotonic >= d.offered_mono + 2)
                || (d.presented && monotonic >= d.offered_mono + 75)
            {
                drop(i);
                user_activity(app, "care-expired", false);
            }
            return;
        }
        i.candidates
            .retain(|c| candidate_valid(c, &snapshot, &mod_id, time));
        let local = chrono::Local
            .timestamp_opt(time, 0)
            .single()
            .unwrap_or_else(chrono::Local::now);
        let minute = local.hour() * 60 + local.minute();
        let date = local.date_naive();
        let day_end = local_deadline(date.succ_opt().unwrap_or(date), 0).unwrap_or(time);
        let mut proposals = vec![];
        if (i.settings.rain || i.settings.legacy_weather) && snapshot.weather.validity == "fresh" {
            if let Some(w) = &snapshot.weather.data {
                let mut kinds = vec![];
                if i.settings.rain {
                    if let Some(k) = weather::phenomenon(w) {
                        kinds.push(k);
                    }
                }
                if i.settings.legacy_weather {
                    if w.temperature <= i.settings.cold_threshold {
                        kinds.push("cold");
                    }
                    if w.temperature >= i.settings.hot_threshold {
                        kinds.push("hot");
                    }
                }
                for kind in kinds {
                    let event = format!(
                        "{}:{}:{}:{}",
                        mod_id,
                        kind,
                        snapshot.location.key.as_deref().unwrap_or(""),
                        date
                    );
                    let facts = serde_json::json!({"category":"weather","phenomenon":kind,"region":snapshot.location.city,"condition":w.condition,"temperature_celsius":w.temperature,"valid_at":snapshot.weather.valid_at,"data_kind":snapshot.weather.data_kind,"usage":"这是用户关注地区，不能推断用户身在当地。model_current 必须说明基于天气估算，不能称实况观测。"});
                    proposals.push(Candidate {
                        identity: format!("{}:{}", event, snapshot.weather.valid_at.unwrap_or(0)),
                        event,
                        kind: kind.into(),
                        mod_id: mod_id.clone(),
                        location_revision: snapshot.location.revision,
                        observation: snapshot.weather.valid_at,
                        expires: (time + 300).min(snapshot.weather.expires_at.unwrap_or(time)),
                        ready_mono: 0,
                        event_until: day_end,
                        text: facts.to_string(),
                        temp_range: match kind {
                            "cold" => Some((-100., i.settings.cold_threshold)),
                            "hot" => Some((i.settings.hot_threshold, 100.)),
                            _ => None,
                        },
                        calendar_version: None,
                        clock_revision: snapshot.clock_revision,
                    });
                }
            }
        }
        if i.settings.night && in_period(minute, i.settings.night_start, i.settings.night_end) {
            let start_date =
                if i.settings.night_start > i.settings.night_end && minute < i.settings.night_end {
                    date.pred_opt().unwrap_or(date)
                } else {
                    date
                };
            let end_date = if i.settings.night_start > i.settings.night_end {
                start_date.succ_opt().unwrap_or(start_date)
            } else {
                start_date
            };
            let end = local_deadline(end_date, i.settings.night_end).unwrap_or(time);
            let event = format!("{}:night:{}", mod_id, start_date);
            proposals.push(Candidate{identity:format!("{}:idle:{}",event,i.eligibility_epoch),event,kind:"night".into(),mod_id:mod_id.clone(),location_revision:0,observation:None,expires:(time+600).min(end),ready_mono:0,event_until:end,text:serde_json::json!({"category":"night","local_time":snapshot.local_time,"fact":"已进入用户设定的晚间时段；不代表用户疲劳或持续工作"}).to_string(),temp_range:None,calendar_version:None,clock_revision:snapshot.clock_revision});
        }
        if (480..1380).contains(&minute) {
            let until = local_deadline(date, 1380).unwrap_or(time);
            if i.settings.holiday {
                for set in &book.datasets {
                    for event in set
                        .events
                        .iter()
                        .filter(|e| e.date == date && (e.kind != "term" || i.settings.solar_terms))
                    {
                        let key =
                            format!("{}:holiday:{}:{}:{}", mod_id, set.country, event.name, date);
                        proposals.push(Candidate{identity:format!("{}:idle:{}",key,i.eligibility_epoch),event:key,kind:"holiday".into(),mod_id:mod_id.clone(),location_revision:0,observation:None,expires:(time+600).min(until),ready_mono:0,event_until:day_end,text:serde_json::json!({"category":"holiday","name":event.name,"event_kind":event.kind,"country":set.country,"date":date,"source":set.source,"fact":"仅代表导入日历的节日；节气名称不代表实际天气，不代表用户当天休假或身在该国家"}).to_string(),temp_range:None,calendar_version:Some(calendar_key(&snapshot)),clock_revision:snapshot.clock_revision});
                    }
                }
            }
            if i.settings.special_dates {
                for event in book
                    .special_dates
                    .iter()
                    .filter(|s| s.occurs(date, &book.datasets))
                {
                    let key = format!("{}:special:{}:{}", mod_id, event.id, date);
                    proposals.push(Candidate{identity:format!("{}:idle:{}",key,i.eligibility_epoch),event:key,kind:"special".into(),mod_id:mod_id.clone(),location_revision:0,observation:None,expires:(time+600).min(until),ready_mono:0,event_until:day_end,text:serde_json::json!({"category":"special","name":event.name,"kind":event.kind,"date":date,"basis":event.description(),"source":event.source,"fact":"这是已保存的用户特殊日期；不要猜年龄或未提供的经历"}).to_string(),temp_range:None,calendar_version:Some(calendar_key(&snapshot)),clock_revision:snapshot.clock_revision});
                }
            }
        }
        // Random admission and selection among ordinary facts; personal dates may evict one.
        for n in (1..proposals.len()).rev() {
            let k = random_number() as usize % (n + 1);
            proposals.swap(n, k);
        }
        proposals.sort_by_key(|c| c.kind != "special");
        for mut c in proposals {
            if i.seen.contains(&c.identity) || i.candidates.iter().any(|old| old.event == c.event) {
                continue;
            }
            if !make_room(&mut i.candidates, &c) {
                continue;
            }
            match i
                .ledger
                .as_ref()
                .ok_or("关怀记录库不可用".to_string())
                .and_then(|l| l.eligible(&c.event, &c.kind, time))
            {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    i.blocked = Some(e);
                    return;
                }
            }
            let delay = random_delay();
            if time + delay as i64 >= c.expires {
                continue;
            }
            i.seen.insert(c.identity.clone());
            if i.seen.len() > 512 {
                i.seen.retain(|s| s.contains(&date.to_string()));
            }
            c.ready_mono = monotonic + delay;
            i.candidates.push(c);
        }
        if i.last_shown_mono
            .is_some_and(|t| monotonic.saturating_sub(t) < 2700)
        {
            return;
        }
        let Some(index) = select_ready(&i.candidates, monotonic, random_number()) else {
            return;
        };
        let candidate = i.candidates.remove(index);
        match i
            .ledger
            .as_ref()
            .unwrap()
            .eligible(&candidate.event, &candidate.kind, time)
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                i.blocked = Some(e);
                return;
            }
        }
        i.generation += 1;
        let generation = i.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        i.generating = Some(Generating {
            generation,
            candidate: candidate.clone(),
            cancel: cancel.clone(),
        });
        i.last_error = None;
        drop(i);
        let Some(service) = global() else { return };
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let companion = handle.state::<super::companion::Companion>();
            let result = tokio::select! {
                result=async {
                    let mut generated = companion.generate_care(&handle,&candidate.text).await?;
                    let estimated = candidate.observation.is_some()
                        && environment_service::global().is_some_and(|s| s.snapshot().weather.data_kind == "model_current");
                    if estimated && !generated.text.contains("估算") {
                        generated.text = format!("根据当前天气估算，{}", generated.text);
                    }
                    let audio = companion.care_audio(&handle, &generated.text).await.unwrap_or(None);
                    Ok((generated, audio))
                }=>result,
                _=async {while !cancel.load(Ordering::SeqCst){tokio::time::sleep(std::time::Duration::from_millis(100)).await;}}=>return,
            };
            service.complete(&handle, generation, result);
        });
    }
    fn complete(
        &self,
        app: &tauri::AppHandle,
        generation: u64,
        result: Result<(super::companion::proactive::Generated, Option<String>), String>,
    ) {
        let snapshot = environment_service::global().unwrap().snapshot();
        let mod_id = match api::presentation(app) {
            api::Presentation::Ready(id) => id,
            api::Presentation::PresetBusy => {
                yield_to_preset(app);
                return;
            }
            api::Presentation::Unavailable => {
                user_activity(app, "unavailable", false);
                return;
            }
        };
        let companion = app.state::<super::companion::Companion>();
        let book = self.calendar.lock().unwrap().data.clone();
        let mut i = self.inner.lock().unwrap();
        if !i
            .generating
            .as_ref()
            .is_some_and(|j| j.generation == generation)
        {
            return;
        }
        let job = i.generating.take().unwrap();
        let (generated, audio) = match result {
            Ok(v) => v,
            Err(_) => {
                i.last_error = Some("本次关怀生成未完成，已跳过".into());
                return;
            }
        };
        if !i.settings.enabled
            || !i.activities.is_empty()
            || mono() < i.quiet_until
            || !candidate_valid(&job.candidate, &snapshot, &mod_id, now())
            || companion.care_revision() != generated.settings_revision
            || companion.store.lock().unwrap().history_epoch().ok() != Some(generated.history_epoch)
        {
            return;
        }
        match i
            .ledger
            .as_ref()
            .unwrap()
            .eligible(&job.candidate.event, &job.candidate.kind, now())
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                i.blocked = Some(e);
                return;
            }
        }
        let Ok(id) = super::companion::settings::random_id() else {
            return;
        };
        if let Err(e) = i.ledger.as_ref().unwrap().reserve(
            &id,
            &job.candidate.event,
            &job.candidate.kind,
            now(),
            job.candidate.event_until,
        ) {
            i.blocked = Some(e);
            return;
        }
        if job.candidate.kind != "special" && special_due(&i, &book, &mod_id, now()) {
            return;
        }
        let mut candidate = job.candidate;
        candidate.text = generated.text;

        let offer = Offer {
            delivery_id: id,
            generation,
            interaction_epoch: i.epoch,
            text: candidate.text.clone(),
            expires_at: now() + 2,
            audio,
        };
        i.delivery = Some(Delivery {
            offer: offer.clone(),
            candidate,
            offered_mono: mono(),
            presented: false,
            history_epoch: generated.history_epoch,
            ai_revision: generated.settings_revision,
        });
        drop(i);
        if let Some(w) = crate::get_render_window(app) {
            let _ = w.emit("care-offer", offer);
        }
    }
    pub fn start(service: Arc<Self>, app: tauri::AppHandle) {
        if service.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = GLOBAL.set(service.clone());
        tauri::async_runtime::spawn(async move {
            while !service.stopped.load(Ordering::SeqCst) {
                service.tick(&app);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    }
}
fn local_deadline(date: chrono::NaiveDate, minute: u32) -> Option<i64> {
    chrono::Local
        .from_local_datetime(&date.and_hms_opt(minute / 60, minute % 60, 0)?)
        .earliest()
        .map(|d| d.timestamp())
}
pub fn in_period(minute: u32, start: u32, end: u32) -> bool {
    if start < end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }
}
fn calendar_key(s: &Snapshot) -> String {
    format!("{}:{}", s.calendar.dataset_version, s.calendar.local_date)
}
fn candidate_valid(c: &Candidate, s: &Snapshot, mod_id: &str, time: i64) -> bool {
    if c.mod_id != mod_id
        || c.clock_revision != s.clock_revision
        || time >= c.expires
        || c.calendar_version
            .as_ref()
            .is_some_and(|v| v != &calendar_key(s))
    {
        return false;
    }
    if c.observation.is_some() {
        s.weather.validity == "fresh"
            && s.location.revision == c.location_revision
            && s.weather.valid_at == c.observation
            && s.weather.data.as_ref().is_some_and(|w| {
                c.temp_range
                    .is_none_or(|(min, max)| w.temperature >= min && w.temperature <= max)
                    && (matches!(c.kind.as_str(), "cold" | "hot")
                        || weather::phenomenon(w) == Some(c.kind.as_str()))
            })
    } else {
        true
    }
}
fn presentation_mod(app: &tauri::AppHandle) -> Option<String> {
    api::presentation_mod(app)
}
fn make_room(candidates: &mut Vec<Candidate>, new: &Candidate) -> bool {
    if candidates.len() < 3 {
        return true;
    }
    if new.kind != "special" {
        return false;
    }
    if let Some(n) = candidates.iter().rposition(|c| c.kind != "special") {
        candidates.remove(n);
        true
    } else {
        false
    }
}
fn select_ready(candidates: &[Candidate], monotonic: u64, random: u64) -> Option<usize> {
    let special = candidates.iter().any(|c| c.kind == "special");
    let ready = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| monotonic >= c.ready_mono && (!special || c.kind == "special"))
        .map(|(n, _)| n)
        .collect::<Vec<_>>();
    if ready.is_empty() {
        None
    } else {
        Some(ready[random as usize % ready.len()])
    }
}
fn special_due(i: &Inner, book: &calendar::Data, mod_id: &str, time: i64) -> bool {
    if !i.settings.special_dates {
        return false;
    }
    let Some(local) = chrono::Local.timestamp_opt(time, 0).single() else {
        return false;
    };
    if !(480..1380).contains(&(local.hour() * 60 + local.minute())) {
        return false;
    }
    let date = local.date_naive();
    book.special_dates
        .iter()
        .filter(|s| s.occurs(date, &book.datasets))
        .any(|s| {
            i.ledger.as_ref().is_some_and(|l| {
                let event = format!("{}:special:{}:{}", mod_id, s.id, date);
                !i.seen
                    .contains(&format!("{}:idle:{}", event, i.eligibility_epoch))
                    && l.eligible(&event, "special", time).unwrap_or(false)
            })
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_preemption_cancels_presentation_without_user_quiet_time_or_frequency_changes() {
        let dir = tempfile::tempdir().unwrap();
        let service = CareService::new(dir.path().into());
        let mut i = service.inner.lock().unwrap();
        i.generation = 3;
        i.quiet_until = 50;
        i.last_shown_mono = Some(10);
        i.candidates.push(c("holiday"));
        let cancel = Arc::new(AtomicBool::new(false));
        i.generating = Some(Generating {
            generation: 3,
            candidate: c("night"),
            cancel: cancel.clone(),
        });
        assert!(cancel_presentation(&mut i));
        assert!(cancel.load(Ordering::SeqCst));
        assert_eq!(i.generation, 4);
        assert_eq!(i.quiet_until, 50);
        assert_eq!(i.last_shown_mono, Some(10));
        assert_eq!(i.candidates.len(), 1);
        assert!(i.activities.is_empty());
        assert!(i
            .ledger
            .as_ref()
            .unwrap()
            .eligible("ordinary-event", "night", 100)
            .unwrap());
        assert!(!cancel_presentation(&mut i));
        assert_eq!(i.generation, 4);
    }
    fn c(kind: &str) -> Candidate {
        Candidate {
            event: kind.into(),
            kind: kind.into(),
            identity: kind.into(),
            mod_id: "other".into(),
            location_revision: 2,
            observation: None,
            expires: 500,
            ready_mono: 20,
            event_until: 1000,
            text: String::new(),
            temp_range: None,
            calendar_version: None,
            clock_revision: 1,
        }
    }
    #[test]
    fn midnight_is_one_night() {
        assert!(in_period(1430, 1380, 120));
        assert!(in_period(10, 1380, 120));
        assert!(!in_period(120, 1380, 120));
        assert!(!in_period(1379, 1380, 120));
    }
    #[test]
    fn temperature_settings_are_generic_and_bounded() {
        assert!(Settings::default().validate().is_ok());
        let mut s = Settings::default();
        s.cold_threshold = 40.;
        assert!(s.validate().is_err());
    }
    #[test]
    fn personal_dates_evict_ordinary_candidates_and_take_priority_after_their_delay() {
        let mut q = vec![c("rain"), c("night"), c("holiday")];
        let mut special = c("special");
        special.ready_mono = 40;
        assert!(make_room(&mut q, &special));
        q.push(special);
        assert_eq!(q.len(), 3);
        assert_eq!(select_ready(&q, 30, 0), None);
        assert_eq!(q[select_ready(&q, 40, 0).unwrap()].kind, "special");
        let ordinary = vec![c("night"), c("holiday")];
        assert_eq!(select_ready(&ordinary, 20, 1), Some(1));
        assert_eq!(select_ready(&ordinary, 20, 0), Some(0));
    }
    #[test]
    fn late_facts_role_changes_and_calendar_edits_invalidate_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = environment_service::EnvironmentService::new(dir.path().into()).snapshot();
        let mut candidate = c("special");
        candidate.calendar_version = Some(calendar_key(&s));
        assert!(candidate_valid(&candidate, &s, "other", 100));
        assert!(!candidate_valid(&candidate, &s, "other", 500));
        assert!(!candidate_valid(&candidate, &s, "ema", 100));
        s.calendar.dataset_version = "changed".into();
        assert!(!candidate_valid(&candidate, &s, "other", 100));
        let mut rain = c("rain");
        rain.observation = Some(100);
        s.weather.validity = "fresh".into();
        s.weather.valid_at = Some(100);
        s.location.revision = 2;
        s.weather.data = Some(crate::modules::environment::WeatherInfo {
            condition: "雨".into(),
            condition_code: "296".into(),
            temperature: 10.,
            feels_like: None,
            humidity: None,
            wind_speed: None,
        });
        assert!(candidate_valid(&rain, &s, "other", 200));
        s.weather.data.as_mut().unwrap().condition_code = "113".into();
        assert!(!candidate_valid(&rain, &s, "other", 200));
    }
}
