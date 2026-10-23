use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[path = "store_v2.rs"]
mod v2;
pub use v2::{Checkpoint, ForgetObservationPreview, HistoryTurn, ReferenceState};
#[cfg(test)]
#[path = "store_v2_tests.rs"]
mod v2_tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub source: String,
    pub user_text: String,
    pub reply: String,
    pub japanese: String,
    pub created_at: String,
}
pub struct Pending {
    pub turn: Turn,
    pub operation_id: String,
    pub submitted: bool,
    pub serialized_content: String,
    pub history_epoch: u64,
}
pub struct Store {
    pub db: Connection,
}
type Result<T> = std::result::Result<T, String>;
fn err(e: rusqlite::Error) -> String {
    e.to_string()
}
fn turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<Turn> {
    Ok(Turn {
        id: row.get(0)?,
        source: row.get(1)?,
        user_text: row.get(2)?,
        reply: row.get(3)?,
        japanese: row.get(4)?,
        created_at: row.get(5)?,
    })
}
impl Store {
    pub fn open(path: &Path, bank_id: &str) -> Result<Self> {
        v2::open(path, bank_id)
    }
    #[allow(dead_code)]
    fn open_legacy(path: &Path, bank_id: &str) -> Result<Self> {
        let db = Connection::open(path).map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(err)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS turns(id TEXT PRIMARY KEY,source TEXT NOT NULL,user_text TEXT NOT NULL,
                reply TEXT NOT NULL,japanese TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL,
                forgotten INTEGER NOT NULL DEFAULT 0,sync_state TEXT NOT NULL DEFAULT 'pending',operation_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS summaries(id INTEGER PRIMARY KEY CHECK(id=1),text TEXT NOT NULL,through_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS inputs(id TEXT PRIMARY KEY,text TEXT NOT NULL,status TEXT NOT NULL,created_at TEXT NOT NULL);").map_err(err)?;
        db.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('bank_id',?1)",
            [bank_id],
        )
        .map_err(err)?;
        let stored: String = db
            .query_row("SELECT value FROM meta WHERE key='bank_id'", [], |r| {
                r.get(0)
            })
            .map_err(err)?;
        if stored != bank_id {
            return Err("聊天数据库与记忆身份不匹配，禁止连接或修改 bank".into());
        }
        Ok(Self { db })
    }
    pub fn insert(&self, value: &Turn) -> Result<()> {
        let metadata = super::observations::TurnMetadata::default();
        self.commit_turn(value, &metadata, None, &[], self.history_epoch()?, None)?;
        Ok(())
    }
    pub fn input(&self, id: &str, text: &str, status: &str) -> Result<()> {
        self.db.execute("INSERT INTO inputs(id,text,status,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET status=excluded.status",params![id,text,status,chrono::Utc::now().to_rfc3339()]).map_err(err)?;
        Ok(())
    }
    pub fn retry_operation(&self, id: &str) -> Result<()> {
        self.db
            .execute(
                "UPDATE turns SET operation_id=?2,sync_state='pending' WHERE id=?1 AND forgotten=0",
                params![id, super::settings::random_id()?],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn recent(&self, limit: usize) -> Result<Vec<Turn>> {
        let mut stmt = self.db.prepare("SELECT t.id,t.source,t.user_text,t.reply,t.japanese,t.created_at FROM turns t JOIN dialogue_order d ON d.turn_id=t.id WHERE t.forgotten=0 ORDER BY d.seq DESC LIMIT ?1").map_err(err)?;
        let mut rows = stmt
            .query_map([limit.min(200) as i64], turn)
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        rows.reverse();
        Ok(rows)
    }
    pub fn pending(&self) -> Result<Option<Pending>> {
        let mut stmt = self.db.prepare("SELECT id,source,user_text,reply,japanese,created_at,operation_id,sync_state FROM turns WHERE forgotten=0 AND sync_state NOT IN ('done','local_only') ORDER BY rowid LIMIT 1").map_err(err)?;
        let mut rows = stmt.query([]).map_err(err)?;
        match rows.next().map_err(err)? {
            Some(row) => {
                let turn = turn(row).map_err(err)?;
                Ok(Some(Pending {
                    serialized_content: self.memory_content(&turn.id)?,
                    history_epoch: self.history_epoch()?,
                    turn,
                    operation_id: row.get(6).map_err(err)?,
                    submitted: row.get::<_, String>(7).map_err(err)? == "submitted",
                }))
            }
            None => Ok(None),
        }
    }
    pub fn sync_state(&self, id: &str, state: &str) -> Result<()> {
        self.db
            .execute(
                "UPDATE turns SET sync_state=?2 WHERE id=?1 AND forgotten=0",
                params![id, state],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn japanese(&self, id: &str, text: &str) -> Result<()> {
        self.db
            .execute(
                "UPDATE turns SET japanese=?2 WHERE id=?1 AND forgotten=0",
                params![id, text],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn pending_count(&self) -> Result<i64> {
        self.db
            .query_row(
                "SELECT COUNT(*) FROM turns WHERE forgotten=0 AND sync_state NOT IN ('done','local_only')",
                [],
                |r| r.get(0),
            )
            .map_err(err)
    }
    pub fn visible_count(&self) -> Result<i64> {
        self.db
            .query_row("SELECT COUNT(*) FROM turns WHERE forgotten=0", [], |r| {
                r.get(0)
            })
            .map_err(err)
    }
    pub fn all(&self) -> Result<Vec<Turn>> {
        let mut stmt=self.db.prepare("SELECT t.id,t.source,t.user_text,t.reply,t.japanese,t.created_at FROM turns t JOIN dialogue_order d ON d.turn_id=t.id WHERE t.forgotten=0 ORDER BY d.seq").map_err(err)?;
        let rows = stmt
            .query_map([], turn)
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        Ok(rows)
    }
    pub fn summary(&self) -> String {
        self.active_checkpoint()
            .ok()
            .flatten()
            .map(|c| c.body)
            .unwrap_or_default()
    }
    pub fn profile(&self) -> String {
        self.db
            .query_row("SELECT value FROM meta WHERE key='user_profile'", [], |r| {
                r.get(0)
            })
            .unwrap_or_default()
    }
    pub fn profile_due(&self) -> Result<bool> {
        let completed: i64 = self
            .db
            .query_row(
                "SELECT COUNT(*) FROM turns WHERE forgotten=0 AND sync_state='done'",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        let last: i64 = self
            .db
            .query_row(
                "SELECT value FROM meta WHERE key='profile_count'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok(completed - last >= 12)
    }
    pub fn set_profile(&self, text: &str) -> Result<()> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO meta(key,value) VALUES('user_profile',?1)",
                [text],
            )
            .map_err(err)?;
        self.db.execute("INSERT OR REPLACE INTO meta(key,value) SELECT 'profile_count',CAST(COUNT(*) AS TEXT) FROM turns WHERE forgotten=0 AND sync_state='done'",[]).map_err(err)?;
        Ok(())
    }
    pub fn set_summary(&self, value: &str, through: &str) -> Result<()> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO summaries(id,text,through_id) VALUES(1,?1,?2)",
                params![value, through],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn forget(&mut self, id: Option<&str>) -> Result<()> {
        self.forget_v2(id)
    }
    #[allow(dead_code)]
    fn forget_legacy(&mut self, id: Option<&str>) -> Result<()> {
        let tx = self.db.transaction().map_err(err)?;
        if let Some(id) = id {
            tx.execute("UPDATE turns SET forgotten=1,user_text='',reply='',japanese='',sync_state='done' WHERE id=?1", [id]).map_err(err)?;
            tx.execute("DELETE FROM inputs WHERE id=?1", [id])
                .map_err(err)?;
        } else {
            tx.execute("DELETE FROM turns", []).map_err(err)?;
            tx.execute("DELETE FROM inputs", []).map_err(err)?;
        }
        tx.execute("DELETE FROM summaries", []).map_err(err)?;
        tx.execute(
            "DELETE FROM meta WHERE key IN ('user_profile','profile_count')",
            [],
        )
        .map_err(err)?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES('rebuild_required','1')",
            [],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn needs_rebuild(&self) -> bool {
        self.db
            .query_row(
                "SELECT value FROM meta WHERE key='rebuild_required'",
                [],
                |r| r.get::<_, String>(0),
            )
            .map(|v| v == "1")
            .unwrap_or(false)
    }
    pub fn rebuilt(&mut self) -> Result<()> {
        let tx = self.db.transaction().map_err(err)?;
        // New operation IDs after deleting the bank, since old operation IDs may still be retained by the server.
        let ids = {
            let mut q = tx
                .prepare("SELECT id FROM turns WHERE forgotten=0 AND sync_state!='local_only'")
                .map_err(err)?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(err)?;
            rows
        };
        for id in ids {
            tx.execute(
                "UPDATE turns SET sync_state='pending',operation_id=?2 WHERE id=?1",
                params![id, super::settings::random_id()?],
            )
            .map_err(err)?;
        }
        tx.execute("DELETE FROM meta WHERE key='rebuild_required'", [])
            .map_err(err)?;
        tx.commit().map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bank_binding_and_forgetting_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut store = Store::open(&path, "ema-test-one").unwrap();
        store
            .insert(&Turn {
                id: "one".into(),
                source: "chat".into(),
                user_text: "喜欢茶".into(),
                reply: "记住了".into(),
                japanese: String::new(),
                created_at: "2026-10-15T10:00:00Z".into(),
            })
            .unwrap();
        store.sync_state("one", "submitted").unwrap();
        drop(store);
        assert!(Store::open(&path, "work-bank").is_err());
        store = Store::open(&path, "ema-test-one").unwrap();
        assert!(store.pending().unwrap().unwrap().submitted);
        store.forget(Some("one")).unwrap();
        assert!(store.recent(20).unwrap().is_empty());
        assert!(store.pending().unwrap().is_none());
        assert!(store.needs_rebuild());
    }
}
