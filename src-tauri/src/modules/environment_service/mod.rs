//! Application-owned environment facts. All consumers share these validity rules.
pub mod calendar;
pub mod clock;
pub mod commands;
pub mod providers;
pub mod settings;
pub mod types;
pub mod weather;
#[cfg(windows)]
pub mod windows_runtime;
use chrono::TimeZone;
use clock::{Clock, SystemClock};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tauri::{Emitter, Manager};
use types::*;

static GLOBAL: OnceLock<Arc<EnvironmentService>> = OnceLock::new();
static WEATHER_DEADLINE: AtomicU64 = AtomicU64::new(0);
static WEATHER_GENERATION: AtomicU64 = AtomicU64::new(0);
static WEATHER_SNAPSHOT_REV: AtomicU64 = AtomicU64::new(0);
static WEATHER_TEMP: AtomicU64 = AtomicU64::new(0);
static WEATHER_CODE: AtomicU64 = AtomicU64::new(0);
static WEATHER_TEXT: AtomicU64 = AtomicU64::new(0);
fn text_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.trim().to_lowercase().hash(&mut h);
    h.finish()
}
/// Lock-free final check while the legacy state manager owns resource locks.
pub fn legacy_state_weather_allowed(state: &super::resource::StateInfo) -> bool {
    let has_temp = state.trigger_temp_start != i32::MIN || state.trigger_temp_end != i32::MAX;
    let rules = state
        .trigger_weather
        .iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>();
    if !has_temp && rules.is_empty() {
        return true;
    }
    let revision = WEATHER_SNAPSHOT_REV.load(Ordering::SeqCst);
    if revision % 2 != 0 {
        return false;
    }
    let generation = legacy_generation();
    let temp = f64::from_bits(WEATHER_TEMP.load(Ordering::SeqCst));
    let code = WEATHER_CODE.load(Ordering::SeqCst);
    let text = WEATHER_TEXT.load(Ordering::SeqCst);
    if !legacy_lease_valid(generation) || revision != WEATHER_SNAPSHOT_REV.load(Ordering::SeqCst) {
        return false;
    }
    (!has_temp
        || (temp >= state.trigger_temp_start as f64 && temp <= state.trigger_temp_end as f64))
        && (rules.is_empty()
            || rules.iter().any(|r| {
                if r.trim().chars().all(|c| c.is_ascii_digit()) {
                    r.parse::<u64>().ok() == Some(code)
                } else {
                    text_hash(r) == text
                }
            }))
}
pub fn global() -> Option<&'static Arc<EnvironmentService>> {
    GLOBAL.get()
}
pub fn legacy_generation() -> u64 {
    WEATHER_GENERATION.load(Ordering::SeqCst)
}
pub fn legacy_lease_valid(generation: u64) -> bool {
    global().is_some_and(|s| s.clock.monotonic() < WEATHER_DEADLINE.load(Ordering::SeqCst))
        && generation == legacy_generation()
}
#[derive(Clone)]
struct Selection {
    key: String,
    name: String,
    source: String,
    latitude: f64,
    longitude: f64,
    city: Option<City>,
    measured_at: i64,
    received_mono: u64,
    accuracy: Option<f64>,
    confirmed: bool,
}
impl Selection {
    fn from_city(c: City, source: &str, now: i64, mono: u64) -> Self {
        Self {
            key: c.id.clone(),
            name: c.name.clone(),
            source: source.into(),
            latitude: c.latitude,
            longitude: c.longitude,
            city: Some(c),
            measured_at: now,
            received_mono: mono,
            accuracy: None,
            confirmed: true,
        }
    }
    fn expires(&self) -> Option<i64> {
        match self.source.as_str() {
            "device" => Some(self.measured_at + 1800),
            "ip_approximate" => Some(self.measured_at + 3600),
            _ => None,
        }
    }
    fn valid(&self, now: i64, mono: u64) -> bool {
        self.confirmed
            && self.measured_at <= now + 120
            && self.expires().is_none_or(|e| now < e)
            && match self.source.as_str() {
                "device" => mono.saturating_sub(self.received_mono) < 1800,
                "ip_approximate" => mono.saturating_sub(self.received_mono) < 3600,
                _ => true,
            }
    }
}
struct Inner {
    settings: Settings,
    blocked: Option<String>,
    revision: u64,
    config_revision: u64,
    location_revision: u64,
    record_revision: u64,
    weather_revision: u64,
    clock_revision: u64,
    location: Option<Selection>,
    weather: Option<weather::Record>,
    device_access: String,
    location_state: String,
    weather_state: String,
    location_error: Option<String>,
    weather_error: Option<String>,
    location_attempt: Option<u64>,
    weather_attempt: Option<u64>,
    location_retry: u64,
    weather_retry: u64,
    location_failures: usize,
    weather_failures: usize,
    city_choices: Vec<City>,
    needs_revalidation: bool,
    last_weather_validity: String,
}
pub struct EnvironmentService {
    inner: Mutex<Inner>,
    pub clock: Arc<dyn Clock>,
    dir: PathBuf,
    location_gate: tokio::sync::Mutex<()>,
    weather_gate: tokio::sync::Mutex<()>,
    started: AtomicBool,
    stopped: AtomicBool,
    #[cfg(windows)]
    access_watcher: Mutex<Option<providers::AccessWatcher>>,
}
impl EnvironmentService {
    pub fn new(dir: PathBuf) -> Self {
        Self::with_clock(dir, Arc::new(SystemClock::default()))
    }
    fn with_clock(dir: PathBuf, clock: Arc<dyn Clock>) -> Self {
        let loaded = settings::load::<Settings>(&dir.join("settings.json")).and_then(|s| {
            s.validate()?;
            Ok(s)
        });
        let blocked = loaded.as_ref().err().cloned();
        let settings = loaded.unwrap_or_else(|_| Settings {
            network_weather_enabled: false,
            ..Settings::default()
        });
        Self {
            inner: Mutex::new(Inner {
                settings: settings.clone(),
                blocked,
                revision: 1,
                config_revision: 1,
                location_revision: 1,
                record_revision: 1,
                weather_revision: 1,
                clock_revision: 1,
                location: None,
                weather: None,
                device_access: if settings.location_mode == "automatic"
                    && settings.device_location_enabled
                {
                    providers::device_access().unwrap_or("not_requested").into()
                } else {
                    "not_requested".into()
                },
                location_state: "idle".into(),
                weather_state: "idle".into(),
                location_error: None,
                weather_error: None,
                location_attempt: None,
                weather_attempt: None,
                location_retry: 0,
                weather_retry: 0,
                location_failures: 0,
                weather_failures: 0,
                city_choices: Vec::new(),
                needs_revalidation: false,
                last_weather_validity: "unavailable".into(),
            }),
            clock,
            dir,
            location_gate: tokio::sync::Mutex::new(()),
            weather_gate: tokio::sync::Mutex::new(()),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            #[cfg(windows)]
            access_watcher: Mutex::new(None),
        }
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        {
            let mut i = self.inner.lock().unwrap();
            Self::invalidate(&mut i);
        }
        #[cfg(windows)]
        {
            self.access_watcher.lock().unwrap().take();
        }
        WEATHER_DEADLINE.store(0, Ordering::SeqCst);
    }
    pub fn snapshot(&self) -> Snapshot {
        let now = self.clock.utc();
        let mono = self.clock.monotonic();
        let local = chrono::Local
            .timestamp_opt(now, 0)
            .single()
            .unwrap_or_else(chrono::Local::now);
        let i = self.inner.lock().unwrap();
        let loc = i.location.as_ref();
        let location_valid = loc.is_some_and(|l| l.valid(now, mono)) && !i.needs_revalidation;
        let (validity, reason) = if !i.settings.network_weather_enabled {
            ("unavailable", Some("weather_disabled"))
        } else if i.needs_revalidation {
            ("unverified", Some("resume_requires_refresh"))
        } else if let Some(w) = &i.weather {
            weather::validity(
                w,
                now,
                mono,
                loc.map_or("", |l| l.key.as_str()),
                i.location_revision,
                location_valid,
            )
        } else {
            ("unavailable", Some("weather_unavailable"))
        };
        let expiry = i
            .weather
            .as_ref()
            .and_then(|w| w.valid_at().map(|o| (o + 1800).min(w.received_at + 900)))
            .map(|e| e.min(loc.and_then(Selection::expires).unwrap_or(i64::MAX)));
        WEATHER_SNAPSHOT_REV.fetch_add(1, Ordering::SeqCst);
        if validity == "fresh" {
            let remaining = expiry.unwrap_or(now).saturating_sub(now).max(0) as u64;
            let data = &i.weather.as_ref().unwrap().data;
            WEATHER_TEMP.store(data.temperature.to_bits(), Ordering::SeqCst);
            WEATHER_CODE.store(
                data.condition_code.parse().unwrap_or(u64::MAX),
                Ordering::SeqCst,
            );
            WEATHER_TEXT.store(text_hash(&data.condition), Ordering::SeqCst);
            WEATHER_DEADLINE.store(mono + remaining, Ordering::SeqCst);
        } else {
            WEATHER_DEADLINE.store(0, Ordering::SeqCst);
        }
        WEATHER_SNAPSHOT_REV.fetch_add(1, Ordering::SeqCst);
        Snapshot {
            schema_version: 1,
            revision: i.revision,
            now,
            local_time: local.to_rfc3339(),
            timezone: local.format("%:z").to_string(),
            clock_revision: i.clock_revision,
            settings: (&i.settings).into(),
            location: LocationView {
                mode: i.settings.location_mode.clone(),
                key: loc.map(|l| l.key.clone()),
                city: loc.map(|l| l.name.clone()),
                source: loc.map(|l| l.source.clone()),
                confirmed: location_valid,
                age_seconds: loc.map(|l| now.saturating_sub(l.measured_at).max(0)),
                accuracy_m: loc.and_then(|l| l.accuracy),
                expires_at: loc.and_then(Selection::expires),
                revision: i.location_revision,
                record_revision: i.record_revision,
                device_access: i.device_access.clone(),
                state: i.location_state.clone(),
                reason: i.blocked.clone().or_else(|| i.location_error.clone()),
            },
            weather: WeatherView {
                data: i.weather.as_ref().map(|w| w.data.clone()),
                validity: validity.into(),
                reason: i
                    .blocked
                    .clone()
                    .or_else(|| reason.map(str::to_string))
                    .or_else(|| i.weather_error.clone()),
                observed_at: i.weather.as_ref().and_then(|w| w.observed_at),
                valid_at: i.weather.as_ref().and_then(|w| w.valid_at()),
                data_kind: i.weather.as_ref().map_or("unknown", |w| w.data_kind).into(),
                received_at: i.weather.as_ref().map(|w| w.received_at),
                expires_at: expiry,
                provider: i.weather.as_ref().map_or("wttr.in", |w| w.provider).into(),
                region: i.weather.as_ref().and_then(|w| w.region.clone()),
                revision: i.weather_revision,
                refresh_state: i.weather_state.clone(),
            },
            calendar: calendar::read(local.date_naive()),
        }
    }
    pub fn current_weather(&self) -> Option<super::environment::WeatherInfo> {
        let s = self.snapshot();
        (s.weather.validity == "fresh")
            .then_some(s.weather.data)
            .flatten()
    }
    pub fn legacy_location(&self) -> Option<super::environment::GeoLocation> {
        let i = self.inner.lock().unwrap();
        let s = i.location.as_ref()?;
        if !matches!(s.source.as_str(), "manual_fixed" | "manual_fallback") {
            return None;
        }
        let c = s.city.as_ref()?;
        Some(super::environment::GeoLocation {
            latitude: c.latitude,
            longitude: c.longitude,
            timezone: Some(c.timezone.clone().into()),
            is_northern_hemisphere: c.latitude >= 0.0,
            city: Some(c.name.clone().into()),
            region: Some(c.region.clone().into()),
            country: Some(c.country.clone().into()),
        })
    }
    pub fn notify(&self, app: &tauri::AppHandle) {
        let _ = app.emit("environment-updated", self.snapshot());
    }
    fn invalidate(i: &mut Inner) {
        i.config_revision += 1;
        i.location_revision += 1;
        i.revision += 1;
        i.location = None;
        i.weather = None;
        i.location_attempt = None;
        i.weather_attempt = None;
        i.location_retry = 0;
        i.weather_retry = 0;
        i.needs_revalidation = false;
        WEATHER_DEADLINE.store(0, Ordering::SeqCst);
        WEATHER_GENERATION.fetch_add(1, Ordering::SeqCst);
    }
    pub fn save(&self, mut next: Settings) -> Result<(), String> {
        next.validate()?;
        let mut i = self.inner.lock().unwrap();
        // UI selects an identifier from directory results, never supplies a fix.
        for city in [&next.fixed_city, &next.fallback_city]
            .into_iter()
            .flatten()
        {
            if !i
                .city_choices
                .iter()
                .chain(i.settings.fixed_city.iter())
                .chain(i.settings.fallback_city.iter())
                .any(|c| c == city)
            {
                return Err("请从查询结果中选择城市".into());
            }
        }
        next.ip_confirmed_city_key = i.settings.ip_confirmed_city_key.clone();
        settings::save(&self.dir.join("settings.json"), &next)?;
        i.settings = next;
        if i.settings.location_mode == "automatic" && i.settings.device_location_enabled {
            if let Some(access) = providers::device_access() {
                i.device_access = access.into();
            }
        }
        i.blocked = None;
        Self::invalidate(&mut i);
        Ok(())
    }
    pub fn confirm_ip(&self, key: &str, revision: u64) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        if revision != i.location_revision
            || !i
                .location
                .as_ref()
                .is_some_and(|l| l.source == "ip_approximate" && l.key == key)
        {
            return Err("地点已变化，请重新确认".into());
        }
        let mut next = i.settings.clone();
        next.ip_confirmed_city_key = Some(key.into());
        settings::save(&self.dir.join("settings.json"), &next)?;
        i.settings = next;
        i.location.as_mut().unwrap().confirmed = true;
        i.location_revision += 1;
        i.revision += 1;
        i.weather = None;
        i.weather_attempt = None;
        WEATHER_GENERATION.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    pub async fn find_cities(&self, query: &str) -> Result<Vec<City>, String> {
        let cities = providers::cities(query).await?;
        self.inner.lock().unwrap().city_choices = cities.clone();
        Ok(cities)
    }
    fn commit_location(i: &mut Inner, selection: Option<Selection>) {
        let changed = i
            .location
            .as_ref()
            .map(|s| (&s.key, &s.source, s.confirmed))
            != selection.as_ref().map(|s| (&s.key, &s.source, s.confirmed));
        if changed {
            i.location_revision += 1;
            i.weather = None;
            i.weather_attempt = None;
            WEATHER_DEADLINE.store(0, Ordering::SeqCst);
            WEATHER_GENERATION.fetch_add(1, Ordering::SeqCst);
        }
        i.location = selection;
        i.record_revision += 1;
        i.revision += 1;
    }
    pub async fn refresh_location(&self, app: &tauri::AppHandle, manual: bool) {
        let Ok(_guard) = self.location_gate.try_lock() else {
            return;
        };
        let now = self.clock.utc();
        let mono = self.clock.monotonic();
        let (config, generation, access) = {
            let mut i = self.inner.lock().unwrap();
            if i.blocked.is_some() || (!manual && !i.settings.network_weather_enabled) {
                return;
            }
            if i.location_attempt
                .is_some_and(|t| mono.saturating_sub(t) < 30)
                || mono < i.location_retry
            {
                return;
            }
            i.location_attempt = Some(mono);
            i.location_state = "locating".into();
            (
                i.settings.clone(),
                i.config_revision,
                i.device_access.clone(),
            )
        };
        let mut error = None;
        let mut selection = None;
        if config.location_mode == "fixed_city" {
            selection = config
                .fixed_city
                .clone()
                .map(|c| Selection::from_city(c, "manual_fixed", now, mono));
            if selection.is_none() {
                error = Some("city_not_set".into());
            }
        } else {
            if config.device_location_enabled && access == "allowed" {
                match providers::device_location().await {
                    Ok(f)
                        if f.measured_at <= self.clock.utc() + 120
                            && self.clock.utc() < f.measured_at + 1800 =>
                    {
                        // Stable 0.1 degree weather request cells, not raw floating-point fixes.
                        let key = format!("device-cell:{:.1}:{:.1}", f.latitude, f.longitude);
                        selection = Some(Selection {
                            key,
                            name: "设备所在地区".into(),
                            source: "device".into(),
                            latitude: f.latitude,
                            longitude: f.longitude,
                            city: None,
                            measured_at: f.measured_at,
                            received_mono: self.clock.monotonic(),
                            accuracy: f.accuracy,
                            confirmed: true,
                        });
                    }
                    Ok(_) => error = Some("device_location_expired".into()),
                    Err(e) => error = Some(e),
                }
            }
            if selection.is_none() {
                selection = config
                    .fallback_city
                    .clone()
                    .map(|c| Selection::from_city(c, "manual_fallback", now, mono));
            }
            if selection.is_none() && config.ip_fallback_enabled {
                match providers::ip_location(self.clock.utc()).await {
                    Ok(f) => {
                        let c = f.city.unwrap();
                        let confirmed =
                            config.ip_confirmed_city_key.as_deref() == Some(c.id.as_str());
                        let mut s = Selection::from_city(
                            c,
                            "ip_approximate",
                            self.clock.utc(),
                            self.clock.monotonic(),
                        );
                        s.confirmed = confirmed;
                        selection = Some(s);
                        if !confirmed {
                            error = Some("ip_city_confirmation_required".into());
                        }
                    }
                    Err(e) => error = Some(e),
                }
            }
            if selection.is_none() && error.is_none() {
                error = Some("location_sources_unavailable".into());
            }
        }
        {
            let mut i = self.inner.lock().unwrap();
            if generation != i.config_revision {
                return;
            }
            if error.as_deref() == Some("device_denied") {
                i.device_access = "denied".into();
            }
            if selection.is_some() {
                i.location_failures = 0;
                i.location_retry = 0;
                i.location_state = "idle".into();
            } else {
                i.location_failures += 1;
                i.location_retry = self.clock.monotonic() + backoff(i.location_failures);
                i.location_state = "backoff".into();
            }
            i.location_error = error;
            Self::commit_location(&mut i, selection);
        }
        self.notify(app);
    }
    pub async fn refresh_weather(&self, app: &tauri::AppHandle) {
        let Ok(_guard) = self.weather_gate.try_lock() else {
            return;
        };
        let (loc, generation, revision) = {
            let mut i = self.inner.lock().unwrap();
            let mono = self.clock.monotonic();
            if i.blocked.is_some()
                || !i.settings.network_weather_enabled
                || i.weather_attempt
                    .is_some_and(|t| mono.saturating_sub(t) < 30)
                || mono < i.weather_retry
            {
                return;
            }
            let Some(loc) = i.location.clone() else {
                return;
            };
            if loc.expires().is_some_and(|e| self.clock.utc() >= e) {
                return;
            }
            i.weather_attempt = Some(mono);
            i.weather_state = "refreshing".into();
            (loc, i.config_revision, i.location_revision)
        };
        self.notify(app);
        let result = providers::current_weather(
            loc.latitude,
            loc.longitude,
            loc.city.as_ref(),
            self.clock.as_ref(),
            loc.key.clone(),
            revision,
        )
        .await;
        {
            let mut i = self.inner.lock().unwrap();
            if generation != i.config_revision || revision != i.location_revision {
                return;
            }
            match result {
                Ok(mut record) => {
                    let keep_existing = i.weather.as_ref().is_some_and(|old| {
                        (old.provider == record.provider
                            && old
                                .valid_at()
                                .zip(record.valid_at())
                                .is_some_and(|(a, b)| b < a))
                            || (!i.needs_revalidation
                                && weather::validity(
                                    old,
                                    self.clock.utc(),
                                    self.clock.monotonic(),
                                    &loc.key,
                                    revision,
                                    true,
                                )
                                .0 == "fresh"
                                && weather::validity(
                                    &record,
                                    self.clock.utc(),
                                    self.clock.monotonic(),
                                    &loc.key,
                                    revision,
                                    true,
                                )
                                .0 != "fresh")
                    });
                    if !keep_existing {
                        i.weather_revision += 1;
                        record.revision = i.weather_revision;
                        i.weather = Some(record);
                        i.needs_revalidation = false;
                    }
                    i.weather_state = "idle".into();
                    i.weather_error = None;
                    i.weather_failures = 0;
                    i.weather_retry = 0;
                }
                Err(e) => {
                    i.weather_failures += 1;
                    i.weather_retry = self.clock.monotonic() + backoff(i.weather_failures);
                    i.weather_state = "backoff".into();
                    i.weather_error = Some(e);
                }
            }
            i.revision += 1;
            WEATHER_GENERATION.fetch_add(1, Ordering::SeqCst);
        }
        self.notify(app);
    }
    pub fn start(service: Arc<Self>, app: tauri::AppHandle) {
        if service.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = GLOBAL.set(service.clone());
        #[cfg(windows)]
        {
            let weak = Arc::downgrade(&service);
            let handle = app.clone();
            *service.access_watcher.lock().unwrap() =
                providers::AccessWatcher::new(move |access| {
                    let (Some(service), Some(access)) = (weak.upgrade(), access) else {
                        return;
                    };
                    {
                        let mut i = service.inner.lock().unwrap();
                        if !i.settings.device_location_enabled
                            || i.settings.location_mode != "automatic"
                            || i.device_access == access
                        {
                            return;
                        }
                        i.device_access = access.into();
                        Self::invalidate(&mut i);
                    }
                    super::care::user_activity(&handle, "location-access", false);
                    service.notify(&handle);
                });
        }

        tauri::async_runtime::spawn(async move {
            let mut previous = (
                service.clock.utc(),
                service.clock.monotonic(),
                chrono::Local::now().offset().local_minus_utc(),
            );
            let mut visible_before = false;
            while !service.stopped.load(Ordering::SeqCst) {
                let utc = service.clock.utc();
                let mono = service.clock.monotonic();
                let offset = chrono::Local::now().offset().local_minus_utc();
                let anomaly = mono.saturating_sub(previous.1) > 90
                    || utc
                        .saturating_sub(previous.0)
                        .saturating_sub(mono.saturating_sub(previous.1) as i64)
                        .unsigned_abs()
                        > 30
                    || offset != previous.2;
                previous = (utc, mono, offset);
                if anomaly {
                    let mut i = service.inner.lock().unwrap();
                    i.clock_revision += 1;
                    Self::invalidate(&mut i);
                    i.needs_revalidation = true;
                    drop(i);
                    super::care::user_activity(&app, "resume", false);
                }
                let state = app.state::<crate::AppState>();
                let visible = {
                    let s = state.storage.lock().unwrap();
                    s.data.settings.show_character
                } && !state.session_locked.load(Ordering::SeqCst);
                if visible && !visible_before {
                    super::care::user_activity(&app, "resume", false);
                }
                visible_before = visible;
                if visible {
                    let (location_due, weather_due) = {
                        let i = service.inner.lock().unwrap();
                        let loc_interval = if i
                            .location
                            .as_ref()
                            .is_some_and(|l| l.source == "ip_approximate")
                        {
                            3600
                        } else {
                            1200
                        };
                        (
                            i.location_attempt
                                .is_none_or(|t| mono.saturating_sub(t) >= loc_interval)
                                || (i.location_state == "backoff" && mono >= i.location_retry),
                            i.weather_attempt
                                .is_none_or(|t| mono.saturating_sub(t) >= 600)
                                || (i.weather_state == "backoff" && mono >= i.weather_retry),
                        )
                    };
                    if location_due {
                        let s = service.clone();
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            s.refresh_location(&a, false).await;
                            s.refresh_weather(&a).await;
                        });
                    } else if weather_due {
                        let s = service.clone();
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            s.refresh_weather(&a).await;
                        });
                    }
                }
                let snapshot = service.snapshot();
                let changed = {
                    let mut i = service.inner.lock().unwrap();
                    if i.last_weather_validity != snapshot.weather.validity {
                        i.last_weather_validity = snapshot.weather.validity.clone();
                        i.revision += 1;
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    service.notify(&app);
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    }
}
fn backoff(attempt: usize) -> u64 {
    match attempt {
        0 | 1 => 60,
        2 => 300,
        3 => 900,
        _ => 1800,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicU64};
    struct FakeClock {
        utc: AtomicI64,
        mono: AtomicU64,
    }
    impl Clock for FakeClock {
        fn utc(&self) -> i64 {
            self.utc.load(Ordering::SeqCst)
        }
        fn monotonic(&self) -> u64 {
            self.mono.load(Ordering::SeqCst)
        }
    }
    fn city() -> City {
        City {
            id: "geonames:1796236".into(),
            name: "上海".into(),
            region: "上海".into(),
            country: "CN".into(),
            timezone: "Asia/Shanghai".into(),
            latitude: 31.23,
            longitude: 121.47,
        }
    }
    fn service() -> (tempfile::TempDir, EnvironmentService, Arc<FakeClock>) {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(FakeClock {
            utc: AtomicI64::new(1000),
            mono: AtomicU64::new(100),
        });
        let service = EnvironmentService::with_clock(dir.path().into(), clock.clone());
        (dir, service, clock)
    }
    fn seed(service: &EnvironmentService, confirmed: bool) {
        let c = city();
        let mut i = service.inner.lock().unwrap();
        let mut location = Selection::from_city(
            c.clone(),
            if confirmed {
                "manual_fixed"
            } else {
                "ip_approximate"
            },
            1000,
            100,
        );
        location.confirmed = confirmed;
        i.location = Some(location);
        i.weather = Some(weather::Record {
            data: super::super::environment::WeatherInfo {
                condition: "雨".into(),
                condition_code: "296".into(),
                temperature: 20.0,
                feels_like: None,
                humidity: None,
                wind_speed: None,
            },
            observed_at: Some(980),
            model_valid_at: None,
            provider: "wttr.in",
            data_kind: "observation",
            received_at: 1000,
            received_mono: 100,
            location_key: c.id,
            location_revision: i.location_revision,
            region: Some("上海".into()),
            matched: true,
            revision: 1,
        });
    }
    #[test]
    fn snapshot_expiry_is_computed_at_read_and_old_data_is_diagnostic_only() {
        let (_d, s, c) = service();
        seed(&s, true);
        assert_eq!(s.snapshot().weather.validity, "fresh");
        assert!(s.current_weather().is_some());
        c.utc.store(1900, Ordering::SeqCst);
        c.mono.store(1000, Ordering::SeqCst);
        assert_eq!(s.snapshot().weather.validity, "stale");
        assert!(s.current_weather().is_none());
        assert!(s.snapshot().weather.data.is_some());
    }
    #[test]
    fn unconfirmed_ip_never_becomes_a_current_fact() {
        let (_d, s, _c) = service();
        seed(&s, false);
        let snapshot = s.snapshot();
        assert!(!snapshot.location.confirmed);
        assert_eq!(snapshot.weather.validity, "unverified");
        assert!(s.current_weather().is_none());
    }
    #[test]
    fn model_current_snapshot_keeps_basis_and_expiry_without_fabricating_observation() {
        let (_d, s, c) = service();
        seed(&s, true);
        {
            let mut i = s.inner.lock().unwrap();
            let w = i.weather.as_mut().unwrap();
            w.observed_at = None;
            w.model_valid_at = Some(980);
            w.provider = "open-meteo.com";
            w.data_kind = "model_current";
        }
        let snapshot = s.snapshot();
        assert_eq!(snapshot.weather.observed_at, None);
        assert_eq!(snapshot.weather.valid_at, Some(980));
        assert_eq!(snapshot.weather.provider, "open-meteo.com");
        assert_eq!(snapshot.weather.data_kind, "model_current");
        assert_eq!(snapshot.weather.validity, "fresh");
        assert!(s.current_weather().is_some());
        c.utc.store(1900, Ordering::SeqCst);
        c.mono.store(1000, Ordering::SeqCst);
        assert_eq!(s.snapshot().weather.validity, "stale");
        assert!(s.current_weather().is_none());
    }
    #[test]
    fn disabling_or_changing_sources_revokes_old_results_immediately() {
        let (_d, s, _c) = service();
        seed(&s, true);
        let prior = s.inner.lock().unwrap().config_revision;
        let mut next = s.inner.lock().unwrap().settings.clone();
        next.network_weather_enabled = false;
        s.save(next).unwrap();
        let i = s.inner.lock().unwrap();
        assert!(i.config_revision > prior);
        assert!(i.location.is_none());
        assert!(i.weather.is_none());
        drop(i);
        assert_eq!(s.snapshot().weather.validity, "unavailable");
    }
    #[test]
    fn clock_and_calendar_use_the_same_injected_date() {
        let (_d, s, c) = service();
        let now = chrono::Local
            .with_ymd_and_hms(2027, 2, 6, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        c.utc.store(now, Ordering::SeqCst);
        let snapshot = s.snapshot();
        assert_eq!(snapshot.now, now);
        assert!(snapshot.local_time.starts_with("2027-02-06"));
        assert_eq!(snapshot.calendar.local_date,"2027-02-06");
        assert!(snapshot.calendar.holidays.is_empty());
        assert_eq!(snapshot.calendar.coverage,"not_imported");
    }
}
