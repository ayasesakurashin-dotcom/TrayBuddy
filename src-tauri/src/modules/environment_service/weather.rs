use super::super::environment::WeatherInfo;
use super::{providers, types::City};
use chrono::{NaiveDateTime, TimeZone};
use serde_json::Value;

#[derive(Clone)]
pub struct Record {
    pub data: WeatherInfo,
    pub observed_at: Option<i64>,
    /// A model's valid time is not an observed measurement timestamp.
    pub model_valid_at: Option<i64>,
    pub provider: &'static str,
    pub data_kind: &'static str,
    pub received_at: i64,
    pub received_mono: u64,
    pub location_key: String,
    pub location_revision: u64,
    pub region: Option<String>,
    pub matched: bool,
    pub revision: u64,
}
impl Record {
    pub fn valid_at(&self) -> Option<i64> {
        self.observed_at.or(self.model_valid_at)
    }
}
pub fn distance_km(a: f64, b: f64, c: f64, d: f64) -> f64 {
    let p = (c - a).to_radians();
    let q = (d - b).to_radians();
    let h = (p / 2.0).sin().powi(2)
        + a.to_radians().cos() * c.to_radians().cos() * (q / 2.0).sin().powi(2);
    6371.0 * 2.0 * h.sqrt().min(1.0).asin()
}
fn number(v: &Value, key: &str) -> Option<f64> {
    v[key]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v[key].as_f64())
        .filter(|n| n.is_finite())
}
pub fn parse(
    v: &Value,
    lat: f64,
    lon: f64,
    city: Option<&City>,
    now: i64,
    mono: u64,
    key: String,
    location_revision: u64,
) -> Result<Record, String> {
    let c = &v["current_condition"][0];
    let temperature = number(c, "temp_C")
        .filter(|n| (-100.0..=70.0).contains(n))
        .ok_or("invalid_weather")?;
    let code = c["weatherCode"]
        .as_str()
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
        .ok_or("invalid_weather")?;
    let area = &v["nearest_area"][0];
    let matched = number(area, "latitude")
        .zip(number(area, "longitude"))
        .is_some_and(|(a, b)| {
            super::types::coordinates_valid(a, b) && distance_km(lat, lon, a, b) <= 50.0
        });
    let region = area["areaName"][0]["value"].as_str().map(str::to_string);
    let timezone = providers::observation_timezone(v, city);
    let observed_at = c["localObsDateTime"]
        .as_str()
        .and_then(|s| {
            ["%Y-%m-%d %I:%M %p", "%Y-%m-%d %H:%M"]
                .iter()
                .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        })
        .and_then(|d| timezone.and_then(|z| z.from_local_datetime(&d).single()))
        .map(|d| d.timestamp());
    // A clock-only observation_time never receives today's date.
    let data = WeatherInfo {
        condition: c["lang_zh"][0]["value"]
            .as_str()
            .or_else(|| c["weatherDesc"][0]["value"].as_str())
            .unwrap_or("天气资料")
            .into(),
        condition_code: code.into(),
        temperature,
        feels_like: number(c, "FeelsLikeC").filter(|n| (-100.0..=90.0).contains(n)),
        humidity: number(c, "humidity")
            .filter(|n| (0.0..=100.0).contains(n))
            .map(|n| n as u32),
        wind_speed: number(c, "windspeedKmph").filter(|n| (0.0..=500.0).contains(n)),
    };
    Ok(Record {
        data,
        observed_at,
        model_valid_at: None,
        provider: "wttr.in",
        data_kind: "observation",
        received_at: now,
        received_mono: mono,
        location_key: key,
        location_revision,
        region,
        matched,
        revision: 0,
    })
}
pub fn validity(
    record: &Record,
    now: i64,
    mono: u64,
    location_key: &str,
    location_revision: u64,
    location_valid: bool,
) -> (&'static str, Option<&'static str>) {
    if !location_valid
        || record.location_key != location_key
        || record.location_revision != location_revision
        || !record.matched
    {
        return ("unverified", Some("location_unverified"));
    }
    let Some(observed) = record.valid_at() else {
        return ("unverified", Some("observation_time_unknown"));
    };
    if observed > now + 120 {
        return ("unverified", Some("observation_in_future"));
    }
    if mono.saturating_sub(record.received_mono) >= 900 || now >= observed + 1800 {
        return ("stale", Some("weather_expired"));
    }
    ("fresh", None)
}

/// Open-Meteo current conditions have a complete UTC valid time, but are model
/// estimates. Do not copy their timestamp into observed_at or fabricate a date
/// for the independent wttr.in payload.
pub fn parse_model_current(
    v: &Value,
    lat: f64,
    lon: f64,
    now: i64,
    mono: u64,
    key: String,
    location_revision: u64,
    region: Option<String>,
) -> Result<Record, String> {
    let c = &v["current"];
    let units = &v["current_units"];
    if units["time"].as_str() != Some("unixtime")
        || units["temperature_2m"].as_str() != Some("°C")
        || units["wind_speed_10m"].as_str() != Some("km/h")
    {
        return Err("invalid_weather".into());
    }
    let valid_at = c["time"]
        .as_i64()
        .filter(|t| *t > 0)
        .ok_or("invalid_weather")?;
    let temperature = number(c, "temperature_2m")
        .filter(|n| (-100.0..=70.0).contains(n))
        .ok_or("invalid_weather")?;
    let (condition, code) = match c["weather_code"].as_u64().ok_or("invalid_weather")? {
        0 => ("晴", "113"),
        1 => ("大部晴朗", "116"),
        2 => ("多云", "119"),
        3 => ("阴", "122"),
        45 | 48 => ("雾", "248"),
        51 => ("小毛毛雨", "263"),
        53 | 55 => ("毛毛雨", "266"),
        56 | 57 => ("冻毛毛雨", "281"),
        61 => ("小雨", "296"),
        63 => ("中雨", "302"),
        65 => ("大雨", "308"),
        66 | 67 => ("冻雨", "311"),
        71 => ("小雪", "326"),
        73 => ("中雪", "329"),
        75 => ("大雪", "332"),
        77 => ("米雪", "185"),
        80 => ("小阵雨", "353"),
        81 => ("阵雨", "356"),
        82 => ("强阵雨", "359"),
        85 => ("阵雪", "368"),
        86 => ("强阵雪", "371"),
        95 | 96 | 99 => ("雷阵雨", "389"),
        _ => return Err("invalid_weather".into()),
    };
    let matched = number(v, "latitude")
        .zip(number(v, "longitude"))
        .is_some_and(|(a, b)| {
            super::types::coordinates_valid(a, b) && distance_km(lat, lon, a, b) <= 50.0
        });
    Ok(Record {
        data: WeatherInfo {
            condition: condition.into(),
            condition_code: code.into(),
            temperature,
            feels_like: number(c, "apparent_temperature").filter(|n| (-100.0..=90.0).contains(n)),
            humidity: number(c, "relative_humidity_2m")
                .filter(|n| (0.0..=100.0).contains(n))
                .map(|n| n as u32),
            wind_speed: number(c, "wind_speed_10m").filter(|n| (0.0..=500.0).contains(n)),
        },
        observed_at: None,
        model_valid_at: Some(valid_at),
        provider: "open-meteo.com",
        data_kind: "model_current",
        received_at: now,
        received_mono: mono,
        location_key: key,
        location_revision,
        region,
        matched,
        revision: 0,
    })
}
pub fn kind(code: &str) -> Option<&'static str> {
    match code {
        "176" | "263" | "266" | "281" | "284" | "293" | "296" | "299" | "302" | "305" | "308"
        | "311" | "314" | "353" | "356" | "359" | "386" | "389" => Some("rain"),
        "179" | "182" | "185" | "227" | "230" | "317" | "320" | "323" | "326" | "329" | "332"
        | "335" | "338" | "362" | "365" | "368" | "371" | "374" | "377" | "392" | "395" => {
            Some("snow")
        }
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Record {
        Record {
            data: WeatherInfo {
                condition: "雨".into(),
                condition_code: "296".into(),
                temperature: 20.0,
                feels_like: None,
                humidity: None,
                wind_speed: None,
            },
            observed_at: Some(1000),
            model_valid_at: None,
            provider: "wttr.in",
            data_kind: "observation",
            received_at: 1000,
            received_mono: 0,
            location_key: "a".into(),
            location_revision: 1,
            region: None,
            matched: true,
            revision: 1,
        }
    }
    #[test]
    fn late_old_observation_does_not_gain_life() {
        let mut r = fixture();
        r.received_at = 10000;
        assert_eq!(validity(&r, 10000, 1, "a", 1, true).0, "stale");
    }
    #[test]
    fn exact_deadlines_and_source_checks() {
        let r = fixture();
        assert_eq!(validity(&r, 1100, 899, "a", 1, true).0, "fresh");
        assert_eq!(validity(&r, 1100, 900, "a", 1, true).0, "stale");
        assert_eq!(validity(&r, 2800, 1, "a", 1, true).0, "stale");
        assert_eq!(validity(&r, 1100, 1, "b", 2, true).0, "unverified");
        assert_eq!(validity(&r, 800, 1, "a", 1, true).0, "unverified");
    }
    #[test]
    fn incomplete_timestamp_is_not_fabricated() {
        let v = serde_json::json!({"current_condition":[{"temp_C":"20","weatherCode":"296","observation_time":"09:00 AM"}],"nearest_area":[{"latitude":"31.2","longitude":"121.4"}]});
        let r = parse(&v, 31.2, 121.4, None, 1000, 0, "a".into(), 1).unwrap();
        assert!(r.observed_at.is_none());
    }

    fn model_fixture() -> Value {
        serde_json::json!({"latitude":39.89,"longitude":116.36,"current_units":{"time":"unixtime","temperature_2m":"°C","wind_speed_10m":"km/h"},"current":{"time":1792736100i64,"temperature_2m":15.4,"relative_humidity_2m":55,"apparent_temperature":13.3,"weather_code":61,"wind_speed_10m":7.6}})
    }
    #[test]
    fn model_valid_time_never_becomes_an_observation_or_a_receipt_timestamp() {
        let r = parse_model_current(
            &model_fixture(),
            39.90,
            116.41,
            1792736200,
            100,
            "a".into(),
            1,
            Some("北京".into()),
        )
        .unwrap();
        assert_eq!(r.observed_at, None);
        assert_eq!(r.valid_at(), Some(1792736100));
        assert_eq!(r.received_at, 1792736200);
        assert_eq!(r.data_kind, "model_current");
        assert_eq!(r.provider, "open-meteo.com");
        assert_eq!(kind(&r.data.condition_code), Some("rain"));
        assert_eq!(validity(&r, 1792736200, 101, "a", 1, true).0, "fresh");
        assert_eq!(validity(&r, 1792737900, 102, "a", 1, true).0, "stale");
        assert_eq!(validity(&r, 1792736200, 1000, "a", 1, true).0, "stale");
        assert_eq!(validity(&r, 1792735900, 101, "a", 1, true).0, "unverified");
        assert_eq!(validity(&r, 1792736200, 101, "a", 1, false).0, "unverified");
    }
    #[test]
    fn model_current_requires_full_utc_time_units_and_a_matching_grid() {
        let original = model_fixture();
        for (field, value) in [
            ("time", serde_json::json!("14:15")),
            ("time", Value::Null),
            ("temperature_2m", serde_json::json!(200)),
            ("weather_code", serde_json::json!(999)),
        ] {
            let mut v = original.clone();
            v["current"][field] = value;
            assert!(
                parse_model_current(&v, 39.9, 116.41, 1792736200, 100, "a".into(), 1, None)
                    .is_err()
            );
        }
        let mut v = original.clone();
        v["current_units"]["temperature_2m"] = serde_json::json!("°F");
        assert!(
            parse_model_current(&v, 39.9, 116.41, 1792736200, 100, "a".into(), 1, None).is_err()
        );
        let r = parse_model_current(&original, 31.2, 121.4, 1792736200, 100, "a".into(), 1, None)
            .unwrap();
        assert_eq!(validity(&r, 1792736200, 101, "a", 1, true).0, "unverified");
    }
}
