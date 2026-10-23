use super::super::environment::WeatherInfo;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct City {
    pub id: String,
    pub name: String,
    pub region: String,
    pub country: String,
    pub timezone: String,
    pub latitude: f64,
    pub longitude: f64,
}
impl City {
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && !self.name.is_empty()
            && coordinates_valid(self.latitude, self.longitude)
    }
}
pub fn coordinates_valid(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub network_weather_enabled: bool,
    pub location_mode: String,
    pub fixed_city: Option<City>,
    pub fallback_city: Option<City>,
    pub device_location_enabled: bool,
    pub ip_fallback_enabled: bool,
    pub ip_confirmed_city_key: Option<String>,
    pub calendar_region: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            network_weather_enabled: true,
            location_mode: "fixed_city".into(),
            fixed_city: None,
            fallback_city: None,
            device_location_enabled: false,
            ip_fallback_enabled: false,
            ip_confirmed_city_key: None,
            calendar_region: "CN".into(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || !matches!(self.location_mode.as_str(), "fixed_city" | "automatic")
            || self.calendar_region != "CN"
            || self
                .fixed_city
                .iter()
                .chain(self.fallback_city.iter())
                .any(|c| !c.valid())
        {
            return Err("环境设置格式无效".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct LocationView {
    pub mode: String,
    pub key: Option<String>,
    pub city: Option<String>,
    pub source: Option<String>,
    pub confirmed: bool,
    pub age_seconds: Option<i64>,
    pub accuracy_m: Option<f64>,
    pub expires_at: Option<i64>,
    pub revision: u64,
    pub record_revision: u64,
    pub device_access: String,
    pub state: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct WeatherView {
    pub data: Option<WeatherInfo>,
    pub validity: String,
    pub reason: Option<String>,
    pub observed_at: Option<i64>,
    pub valid_at: Option<i64>,
    pub data_kind: String,
    pub received_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub provider: String,
    pub region: Option<String>,
    pub revision: u64,
    pub refresh_state: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct CalendarView {
    pub local_date: String,
    pub region: String,
    pub dataset_version: String,
    pub coverage: String,
    pub holidays: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub now: i64,
    pub local_time: String,
    pub timezone: String,
    pub clock_revision: u64,
    pub settings: SettingsView,
    pub location: LocationView,
    pub weather: WeatherView,
    pub calendar: CalendarView,
}
/// Public DTOs never serialize device/IP coordinates. Manual city coordinates
/// belong to the directory and are not evidence of the user's whereabouts.
#[derive(Clone, Debug, Serialize)]
pub struct SettingsView {
    pub network_weather_enabled: bool,
    pub location_mode: String,
    pub fixed_city: Option<City>,
    pub fallback_city: Option<City>,
    pub device_location_enabled: bool,
    pub ip_fallback_enabled: bool,
}
impl From<&Settings> for SettingsView {
    fn from(s: &Settings) -> Self {
        Self {
            network_weather_enabled: s.network_weather_enabled,
            location_mode: s.location_mode.clone(),
            fixed_city: s.fixed_city.clone(),
            fallback_city: s.fallback_city.clone(),
            device_location_enabled: s.device_location_enabled,
            ip_fallback_enabled: s.ip_fallback_enabled,
        }
    }
}
