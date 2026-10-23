//! Network and platform adapters; no dialog, persona, memory or model access.
use super::types::{coordinates_valid, City};
use serde_json::Value;
use std::time::Duration;

pub async fn json(url: &str, timeout: u64, cap: usize) -> Result<Value, String> {
    if !url.starts_with("https://") {
        return Err("https_required".into());
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "network_unavailable")?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "network_unavailable")?
        .error_for_status()
        .map_err(|_| "provider_error")?;
    if response.content_length().is_some_and(|n| n > cap as u64) {
        return Err("response_too_large".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "network_unavailable")? {
        if bytes.len() + chunk.len() > cap {
            return Err("response_too_large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid_response".into())
}
pub async fn cities(query: &str) -> Result<Vec<City>, String> {
    if query.trim().chars().count() < 2 || query.chars().count() > 80 {
        return Err("请输入 2–80 个字符的城市名称".into());
    }
    let mut url = reqwest::Url::parse("https://geocoding-api.open-meteo.com/v1/search")
        .map_err(|_| "invalid_url")?;
    url.query_pairs_mut()
        .append_pair("name", query.trim())
        .append_pair("count", "10")
        .append_pair("language", "zh");
    let data = json(url.as_str(), 5, 256 * 1024).await?;
    Ok(data["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let c = City {
                id: format!("geonames:{}", v["id"].as_u64()?),
                name: v["name"].as_str()?.into(),
                region: v["admin1"].as_str().unwrap_or("").into(),
                country: v["country_code"].as_str()?.into(),
                timezone: v["timezone"].as_str()?.into(),
                latitude: v["latitude"].as_f64()?,
                longitude: v["longitude"].as_f64()?,
            };
            (c.valid() && c.timezone.parse::<chrono_tz::Tz>().is_ok()).then_some(c)
        })
        .collect())
}

pub async fn current_weather(
    lat: f64,
    lon: f64,
    city: Option<&City>,
    clock: &dyn super::clock::Clock,
    key: String,
    revision: u64,
) -> Result<super::weather::Record, String> {
    let url = format!("https://wttr.in/{lat:.4},{lon:.4}?format=j1&lang=zh");
    let primary = json(&url, 3, 1024 * 1024).await.and_then(|v| {
        super::weather::parse(
            &v,
            lat,
            lon,
            city,
            clock.utc(),
            clock.monotonic(),
            key.clone(),
            revision,
        )
    });
    if primary.as_ref().is_ok_and(|record| {
        super::weather::validity(record, clock.utc(), clock.monotonic(), &key, revision, true).0
            == "fresh"
    }) {
        return primary;
    }
    // The fallback is explicitly presented as model current conditions. Only
    // coarse weather-grid coordinates leave the service, never the device fix.
    let coarse_lat = (lat * 100.0).round() / 100.0;
    let coarse_lon = (lon * 100.0).round() / 100.0;
    let url=format!("https://api.open-meteo.com/v1/forecast?latitude={coarse_lat:.2}&longitude={coarse_lon:.2}&current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m&temperature_unit=celsius&wind_speed_unit=kmh&timezone=UTC&timeformat=unixtime&forecast_days=1");
    let region = primary
        .as_ref()
        .ok()
        .filter(|r| r.matched)
        .and_then(|r| r.region.clone())
        .or_else(|| city.map(|c| c.name.clone()));
    match json(&url, 5, 256 * 1024).await.and_then(|v| {
        super::weather::parse_model_current(
            &v,
            lat,
            lon,
            clock.utc(),
            clock.monotonic(),
            key,
            revision,
            region,
        )
    }) {
        Ok(record) => Ok(record),
        Err(error) => primary.or(Err(error)),
    }
}
#[derive(Clone)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    pub measured_at: i64,
    pub accuracy: Option<f64>,
    pub city: Option<City>,
}
pub async fn ip_location(now: i64) -> Result<Fix, String> {
    // ipapi.co supplies a canonical country/region/city tuple over HTTPS.
    let v = json("https://ipapi.co/json/", 5, 256 * 1024).await?;
    if v["error"].as_bool() == Some(true) {
        return Err("ip_unavailable".into());
    }
    let lat = v["latitude"].as_f64().ok_or("invalid_location")?;
    let lon = v["longitude"].as_f64().ok_or("invalid_location")?;
    let country = v["country_code"].as_str().ok_or("invalid_location")?;
    let region = v["region_code"].as_str().ok_or("invalid_location")?;
    let name = v["city"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("invalid_location")?;
    if !coordinates_valid(lat, lon) {
        return Err("invalid_location".into());
    }
    Ok(Fix {
        latitude: lat,
        longitude: lon,
        measured_at: now,
        accuracy: None,
        city: Some(City {
            id: format!("ip-city:{}:{}:{}", country, region, name.to_lowercase()),
            name: name.into(),
            region: v["region"].as_str().unwrap_or(region).into(),
            country: country.into(),
            timezone: v["timezone"].as_str().unwrap_or("").into(),
            latitude: lat,
            longitude: lon,
        }),
    })
}
#[cfg(windows)]
pub async fn device_location() -> Result<Fix, String> {
    use windows::{Devices::Geolocation::Geolocator, Foundation::TimeSpan};
    let locator = super::windows_runtime::geolocator()?;
    // RequestAccessAsync is deliberately absent here; only the foreground
    // settings command may create a permission prompt.
    let status = locator.LocationStatus().map_err(|_| "device_unavailable")?;
    if status == windows::Devices::Geolocation::PositionStatus::Disabled {
        return Err("device_denied".into());
    }
    let op = locator
        .GetGeopositionAsyncWithAgeAndTimeout(
            TimeSpan {
                Duration: 300 * 10_000_000,
            },
            TimeSpan {
                Duration: 10 * 10_000_000,
            },
        )
        .map_err(|_| "device_unavailable")?;
    let position = wait_winrt(op, 10).await?;
    let c = position.Coordinate().map_err(|_| "invalid_location")?;
    let point = c
        .Point()
        .and_then(|p| p.Position())
        .map_err(|_| "invalid_location")?;
    let latitude = point.Latitude;
    let longitude = point.Longitude;
    let accuracy = c.Accuracy().map_err(|_| "invalid_location")?;
    let measured_at =
        c.Timestamp().map_err(|_| "invalid_location")?.UniversalTime / 10_000_000 - 11_644_473_600;
    if !coordinates_valid(latitude, longitude)
        || !accuracy.is_finite()
        || !(0.0..=5000.0).contains(&accuracy)
    {
        return Err("device_accuracy_insufficient".into());
    }
    Ok(Fix {
        latitude,
        longitude,
        accuracy: Some(accuracy),
        measured_at,
        city: None,
    })
}
#[cfg(not(windows))]
pub async fn device_location() -> Result<Fix, String> {
    Err("device_unavailable".into())
}

/// Resolve timezone only, not an address. The timezone response contains model
/// weather fields but none enter the observation store. Device locations without
/// reliable timezone metadata cannot establish a wttr.in observation timestamp.
pub fn observation_timezone(value: &Value, city: Option<&City>) -> Option<chrono_tz::Tz> {
    let country_zone = match value["nearest_area"][0]["country"][0]["value"].as_str() {
        Some("China") => Some("Asia/Shanghai"),
        Some("Japan") => Some("Asia/Tokyo"),
        Some("Singapore") => Some("Asia/Singapore"),
        Some("United Kingdom") => Some("Europe/London"),
        _ => None,
    };
    value["time_zone"][0]["zoneName"]
        .as_str()
        .or_else(|| city.map(|c| c.timezone.as_str()))
        .or(country_zone)
        .and_then(|s| s.parse().ok())
}

/// Checks permission without asking for it or reading a coordinate.
#[cfg(windows)]
pub fn device_access() -> Option<&'static str> {
    use windows::Security::Authorization::AppCapabilityAccess::{
        AppCapability, AppCapabilityAccessStatus as S,
    };
    let status = super::windows_runtime::location_capability()
        .ok()?
        .CheckAccess()
        .ok()?;
    if status == S::Allowed {
        Some("allowed")
    } else if status == S::DeniedByUser || status == S::DeniedBySystem {
        Some("denied")
    } else if status == S::UserPromptRequired {
        Some("not_requested")
    } else {
        None
    }
}
#[cfg(not(windows))]
pub fn device_access() -> Option<&'static str> {
    Some("unavailable")
}
#[cfg(windows)]
pub struct AccessWatcher {
    capability: windows::Security::Authorization::AppCapabilityAccess::AppCapability,
    token: windows::Foundation::EventRegistrationToken,
}
#[cfg(windows)]
impl AccessWatcher {
    pub fn new(changed: impl Fn(Option<&'static str>) + Send + Sync + 'static) -> Option<Self> {
        use windows::{
            Foundation::TypedEventHandler,
            Security::Authorization::AppCapabilityAccess::{
                AppCapability, AppCapabilityAccessChangedEventArgs,
            },
        };
        let capability = super::windows_runtime::location_capability().ok()?;
        let token = capability
            .AccessChanged(&TypedEventHandler::<
                AppCapability,
                AppCapabilityAccessChangedEventArgs,
            >::new(move |_, _| {
                changed(device_access());
                Ok(())
            }))
            .ok()?;
        Some(Self { capability, token })
    }
}
#[cfg(windows)]
impl Drop for AccessWatcher {
    fn drop(&mut self) {
        let _ = self.capability.RemoveAccessChanged(self.token);
    }
}
/// windows 0.58 exposes completion handlers, not Future. Bridge completion to a
/// bounded async wait without blocking the UI thread or a runtime worker.
#[cfg(windows)]
pub async fn wait_winrt<T: windows::core::RuntimeType + Send + 'static>(
    operation: windows::Foundation::IAsyncOperation<T>,
    seconds: u64,
) -> Result<T, String> {
    use windows::Foundation::AsyncOperationCompletedHandler;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let sender = std::sync::Mutex::new(Some(tx));
    let copy = operation.clone();
    operation
        .SetCompleted(&AsyncOperationCompletedHandler::new(move |_, _| {
            if let Some(tx) = sender.lock().unwrap().take() {
                let _ = tx.send(
                    copy.GetResults()
                        .map_err(|_| "device_unavailable".to_string()),
                );
            }
            Ok(())
        }))
        .map_err(|_| "device_unavailable")?;
    match tokio::time::timeout(Duration::from_secs(seconds), rx).await {
        Ok(Ok(value)) => value,
        _ => {
            let _ = operation.Cancel();
            Err("device_timeout".into())
        }
    }
}
