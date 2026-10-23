use std::time::Instant;
pub trait Clock: Send + Sync {
    fn utc(&self) -> i64;
    fn monotonic(&self) -> u64;
}
pub struct SystemClock(Instant);
impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for SystemClock {
    fn utc(&self) -> i64 {
        chrono::Utc::now().timestamp()
    }
    fn monotonic(&self) -> u64 {
        self.0.elapsed().as_secs()
    }
}
