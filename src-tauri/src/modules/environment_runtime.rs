// Compatibility methods delegate to the single application-owned service.
// No independent network client, IP fallback or cache remains.
impl EnvironmentManager {
    pub async fn get_location(&mut self) -> Option<GeoLocation> {
        get_cached_location()
    }
    pub async fn refresh_location(&mut self) -> Option<GeoLocation> {
        get_cached_location()
    }
    pub async fn get_weather(&mut self) -> Option<WeatherInfo> {
        get_cached_weather()
    }
    pub async fn get_environment_info(&mut self) -> EnvironmentInfo {
        EnvironmentInfo {
            location: get_cached_location(),
            datetime: self.get_datetime(),
            weather: get_cached_weather(),
        }
    }
}
#[deprecated(note = "EnvironmentService is registered and started once by the application")]
pub fn init_environment(_app_handle: Option<tauri::AppHandle>) {}
