pub fn category(kind: &str) -> &'static str {
    match kind {
        "rain" | "snow" | "cold" | "hot" | "fog" | "dust" | "haze" | "thunder" | "ice" | "wind" => {
            "weather"
        }
        "night" => "night",
        "special" => "special",
        _ => "holiday",
    }
}
use rusqlite::{params, Connection};
pub struct Ledger(Connection);
impl Ledger {
    pub fn open(path: &std::path::Path, now: i64) -> Result<Self, String> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|_| "关怀记录目录不可用")?;
        }
        let c = Connection::open(path).map_err(|_| "关怀记录库不可用")?;
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS deliveries(id TEXT PRIMARY KEY,event_key TEXT NOT NULL,kind TEXT NOT NULL,status TEXT NOT NULL,at INTEGER NOT NULL,until_at INTEGER NOT NULL); CREATE UNIQUE INDEX IF NOT EXISTS shown_event ON deliveries(event_key) WHERE status='shown';").map_err(|_|"关怀记录库不可用")?;
        let has_category: bool = c
            .prepare("PRAGMA table_info(deliveries)")
            .map_err(|_| "关怀迁移失败")?
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|_| "关怀迁移失败")?
            .filter_map(Result::ok)
            .any(|v| v == "category");
        if !has_category {
            c.execute_batch("BEGIN IMMEDIATE; ALTER TABLE deliveries ADD COLUMN category TEXT NOT NULL DEFAULT 'holiday'; UPDATE deliveries SET category=CASE WHEN kind IN ('rain','snow','cold','hot','fog','dust','haze','thunder','ice','wind') THEN 'weather' WHEN kind='night' THEN 'night' WHEN kind='special' THEN 'special' ELSE 'holiday' END; COMMIT;").map_err(|_|"关怀记录迁移失败，原库已保留")?;
        }
        // A crash between display and receipt must not replay the offer.
        c.execute(
            "UPDATE deliveries SET status='uncertain' WHERE status='reserved'",
            [],
        )
        .map_err(|_| "关怀记录库不可用")?;
        c.execute(
            "DELETE FROM deliveries WHERE at < ?1 AND until_at < ?2",
            params![now - 172800, now],
        )
        .map_err(|_| "关怀记录清理失败")?;
        Ok(Self(c))
    }
    pub fn eligible(&self, event: &str, kind: &str, now: i64) -> Result<bool, String> {
        let n:i64=self.0.query_row("SELECT COUNT(*) FROM deliveries WHERE (status='shown' AND (event_key=?1 OR at>?2 OR at>?3)) OR (status IN ('reserved','uncertain') AND event_key=?1 AND until_at>?4)",params![event,now-2700,now+120,now],|r|r.get(0)).map_err(|_|"关怀记录读取失败")?;
        if n > 0 {
            return Ok(false);
        }
        let n: i64 = self
            .0
            .query_row(
                "SELECT COUNT(*) FROM deliveries WHERE status='shown' AND at>?1 AND category=?2",
                params![now - 86400, category(kind)],
                |r| r.get(0),
            )
            .map_err(|_| "关怀记录读取失败")?;
        if n >= 2 {
            return Ok(false);
        }
        if category(kind) == "weather" {
            let n: i64 = self
                .0
                .query_row(
                    "SELECT COUNT(*) FROM deliveries WHERE status='shown' AND kind=?1 AND at>?2",
                    params![kind, now - 28800],
                    |r| r.get(0),
                )
                .map_err(|_| "关怀记录读取失败")?;
            if n > 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn reserve(
        &self,
        id: &str,
        event: &str,
        kind: &str,
        now: i64,
        until: i64,
    ) -> Result<(), String> {
        self.0.execute("INSERT INTO deliveries(id,event_key,kind,status,at,until_at,category) VALUES(?1,?2,?3,'reserved',?4,?5,?6)",params![id,event,kind,now,until,category(kind)]).map(|_|()).map_err(|_|"关怀记录保存失败".into())
    }
    pub fn presented(&self, id: &str, now: i64) -> Result<bool, String> {
        self.0
            .execute(
                "UPDATE deliveries SET status='shown',at=?2 WHERE id=?1 AND status='reserved'",
                params![id, now],
            )
            .map(|n| n == 1)
            .map_err(|_| "关怀记录保存失败".into())
    }
    pub fn reject(&self, id: &str) -> Result<(), String> {
        self.0
            .execute(
                "UPDATE deliveries SET status='cancelled' WHERE id=?1 AND status='reserved'",
                [id],
            )
            .map(|_| ())
            .map_err(|_| "关怀记录保存失败".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_categories_have_independent_limits_but_share_the_45_minute_gap() {
        let dir = tempfile::tempdir().unwrap();
        let l = Ledger::open(&dir.path().join("care.sqlite"), 1000).unwrap();
        l.reserve("1", "a", "rain", 1000, 90000).unwrap();
        l.presented("1", 1000).unwrap();
        assert!(!l.eligible("b", "special", 3699).unwrap());
        assert!(l.eligible("b", "special", 3700).unwrap());
        l.reserve("2", "c", "cold", 4000, 90000).unwrap();
        l.presented("2", 4000).unwrap();
        assert!(!l.eligible("d", "snow", 10000).unwrap());
        for kind in ["night", "holiday", "special"] {
            assert!(l.eligible(kind, kind, 10000).unwrap());
        }
        assert!(l.eligible("d", "snow", 90401).unwrap());
    }
    #[test]
    fn legacy_rows_migrate_without_resetting_display_history() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v1.sqlite");
        let c = Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE deliveries(id TEXT PRIMARY KEY,event_key TEXT NOT NULL,kind TEXT NOT NULL,status TEXT NOT NULL,at INTEGER NOT NULL,until_at INTEGER NOT NULL); INSERT INTO deliveries VALUES('1','rain-today','rain','shown',1000,90000);").unwrap();
        drop(c);
        let l = Ledger::open(&p, 2000).unwrap();
        assert!(!l.eligible("rain-today", "rain", 5000).unwrap());
        assert!(!l.eligible("night", "night", 2000).unwrap());
    }
    #[test]
    fn receipt_idempotence_and_crash_suppression() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("care.sqlite");
        {
            let l = Ledger::open(&p, 1000).unwrap();
            assert!(l.eligible("a", "night", 1000).unwrap());
            l.reserve("1", "a", "night", 1000, 3000).unwrap();
        }
        let l = Ledger::open(&p, 1001).unwrap();
        assert!(!l.eligible("a", "night", 1001).unwrap());
        assert!(l.eligible("a", "night", 3000).unwrap());
        l.reserve("2", "b", "night", 4000, 5000).unwrap();
        assert!(l.presented("2", 4000).unwrap());
        assert!(!l.presented("2", 4001).unwrap());
        assert!(!l.eligible("b", "night", 10000).unwrap());
    }
}
