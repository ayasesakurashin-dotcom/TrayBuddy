//! Weather evidence normalization; never infers hazards from unrelated conditions.
use crate::modules::{environment::WeatherInfo, environment_service};
pub fn phenomenon(w: &WeatherInfo) -> Option<&'static str> {
    let text = w.condition.to_ascii_lowercase();
    if ["沙尘", "沙暴", "dust", "sandstorm", "sand storm"]
        .iter()
        .any(|v| text.contains(v))
    {
        return Some("dust");
    }
    if matches!(
        w.condition_code.as_ref(),
        "200" | "386" | "389" | "392" | "395"
    ) || text.contains("thunder")
        || text.contains("雷")
    {
        return Some("thunder");
    }
    if matches!(
        w.condition_code.as_ref(),
        "185" | "281" | "284" | "311" | "314" | "350" | "374" | "377"
    ) {
        return Some("ice");
    }
    if matches!(w.condition_code.as_ref(), "143" | "248" | "260")
        || text.contains("fog")
        || text.contains("mist")
        || text.contains("雾")
    {
        return Some("fog");
    }
    if text.contains("haze") || text.contains("霾") {
        return Some("haze");
    }
    if let Some(kind) = environment_service::weather::kind(&w.condition_code) {
        return Some(kind);
    }
    if w.wind_speed.is_some_and(|v| v >= 50.) {
        return Some("wind");
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    fn w(text: &str, code: &str) -> WeatherInfo {
        WeatherInfo {
            condition: text.into(),
            condition_code: code.into(),
            temperature: 20.,
            feels_like: None,
            humidity: None,
            wind_speed: None,
        }
    }
    #[test]
    fn distinguish_dust_fog_haze_and_do_not_invent_missing_dust_evidence() {
        assert_eq!(phenomenon(&w("沙尘暴", "999")), Some("dust"));
        assert_eq!(phenomenon(&w("Fog", "248")), Some("fog"));
        assert_eq!(phenomenon(&w("Smoky haze", "149")), Some("haze"));
        assert_eq!(phenomenon(&w("晴", "113")), None);
    }
}
