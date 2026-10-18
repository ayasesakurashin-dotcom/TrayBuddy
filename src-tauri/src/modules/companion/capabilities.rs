//! Future read-only capability boundary. No agent dispatch, shell, MCP, or tools loop.
pub trait CompanionContextSource: Send + Sync {
    fn context(&self) -> String;
}
pub struct LocalContext;
impl CompanionContextSource for LocalContext {
    fn context(&self) -> String {
        let weather = crate::modules::environment::get_cached_weather()
            .and_then(|value| serde_json::to_string(&value).ok())
            .unwrap_or_else(|| "天气缓存不可用，不能据此猜测天气".into());
        format!(
            "应用只读数据：当前本机时间 {}。天气仅为现有缓存：{}。不支持执行设备任务。",
            chrono::Local::now().to_rfc3339(),
            weather
        )
    }
}
