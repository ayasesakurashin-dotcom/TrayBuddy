//! Future read-only capability boundary. No agent dispatch, shell, MCP, or tools loop.
pub trait CompanionContextSource: Send + Sync {
    fn context(&self) -> String;
}
pub struct LocalContext;
impl CompanionContextSource for LocalContext {
    fn context(&self) -> String {
        let weather = crate::modules::environment_service::global().map(|service| {
            let snapshot=service.snapshot();
            if snapshot.weather.validity=="fresh" {
                serde_json::json!({"city":snapshot.location.city,"location_source":snapshot.location.source,"provider":snapshot.weather.provider,"data_kind":snapshot.weather.data_kind,"valid_at":snapshot.weather.valid_at,"observed_at":snapshot.weather.observed_at,"received_at":snapshot.weather.received_at,"weather":snapshot.weather.data,"usage_note":"model_current 为当前天气模型估算，不是实况观测，回复时须说明估算来源"}).to_string()
            } else {format!("当前天气不可用（{}）；不能根据旧资料猜测当前天气",snapshot.weather.reason.unwrap_or(snapshot.weather.validity))}
        }).unwrap_or_else(||"当前天气不可用，不能据此猜测天气".into());
        format!(
            "应用只读数据：当前本机时间 {}。经校验的环境资料：{}。关注城市不证明用户身在当地。不支持执行设备任务。",
            chrono::Local::now().to_rfc3339(),
            weather
        )
    }
}
