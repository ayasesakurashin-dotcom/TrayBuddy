//! CTX-OBS-1 schema, transactions and derived state. All text is role independent.
use super::super::observations::*;
use super::*;
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryTurn {
    #[serde(flatten)]
    pub turn: Turn,
    pub seq: u64,
    #[serde(flatten)]
    pub metadata: TurnMetadata,
    pub observation_refs: Vec<ObservationRef>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReferenceState {
    pub strong_ids: Vec<String>,
    pub weak_ids: Vec<String>,
    pub anchor_seq: u64,
    pub anchored_at: String,
    pub history_epoch: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u64,
    pub history_epoch: u64,
    pub previous_version: Option<u64>,
    pub through_seq: u64,
    pub increment_start_seq: u64,
    pub increment_end_seq: u64,
    pub source_digest: String,
    pub body: String,
    pub source_ids: Vec<String>,
    pub token_count: usize,
    pub tokenizer_version: String,
    pub prompt_version: String,
    pub excluded_observations: usize,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ForgetObservationPreview {
    pub observation_id: String,
    pub dialogue_count: usize,
    pub observation_count: usize,
    pub history_epoch: u64,
    pub preview_token: String,
}

fn add_column(tx: &Transaction<'_>, table: &str, name: &str, definition: &str) -> Result<()> {
    let mut q = tx
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(err)?;
    let columns = q
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(err)?;
    drop(q);
    if !columns.iter().any(|v| v == name) {
        tx.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {name} {definition}"
        ))
        .map_err(err)?;
    }
    Ok(())
}
fn has_table(db: &Connection, name: &str) -> Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )
    .map_err(err)
}
fn require_epoch(db: &Connection, expected: u64) -> Result<()> {
    let epoch: u64 = db
        .query_row(
            "SELECT history_epoch FROM context_state WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if epoch != expected {
        Err("history_changed".into())
    } else {
        Ok(())
    }
}
pub(super) fn open(path: &Path, bank_id: &str) -> Result<Store> {
    let mut db = Connection::open(path).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(err)?;
    let version: u32 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(err)?;
    if version > 2 {
        return Err("聊天数据库版本较新，已停用 AI 写入并保留原数据".into());
    }
    let existing = has_table(&db, "turns")?;
    if has_table(&db, "meta")? {
        let identity: Option<String> = db
            .query_row("SELECT value FROM meta WHERE key='bank_id'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        if identity.as_deref().is_some_and(|s| s != bank_id) {
            return Err("聊天数据库与记忆身份不匹配，禁止连接或修改 bank".into());
        }
        let schema: Option<String> = db
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if schema
            .and_then(|s| s.parse::<u32>().ok())
            .is_some_and(|v| v > 2)
        {
            return Err("聊天数据库版本较新，已停用 AI 写入并保留原数据".into());
        }
    }
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(err)?;
    if version < 2 {
        if existing && path != Path::new(":memory:") {
            let backup = path.with_extension(format!(
                "schema1-backup-{}.sqlite",
                super::super::settings::random_id()?
            ));
            db.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])
                .map_err(|_| "聊天数据库迁移备份失败，原数据已保留".to_string())?;
        }
        let tx = db.transaction().map_err(err)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS turns(id TEXT PRIMARY KEY,source TEXT NOT NULL,user_text TEXT NOT NULL,reply TEXT NOT NULL,japanese TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL,forgotten INTEGER NOT NULL DEFAULT 0,sync_state TEXT NOT NULL DEFAULT 'pending',operation_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS summaries(id INTEGER PRIMARY KEY CHECK(id=1),text TEXT NOT NULL,through_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS inputs(id TEXT PRIMARY KEY,text TEXT NOT NULL,status TEXT NOT NULL,created_at TEXT NOT NULL);").map_err(err)?;
        add_column(
            &tx,
            "turns",
            "interaction_kind",
            "TEXT NOT NULL DEFAULT 'chat'",
        )?;
        add_column(
            &tx,
            "turns",
            "finish_kind",
            "TEXT NOT NULL DEFAULT 'complete'",
        )?;
        add_column(&tx, "turns", "user_initiated", "INTEGER DEFAULT 1")?;
        for (name, def) in [
            ("payload_fingerprint", "TEXT NOT NULL DEFAULT ''"),
            ("request_epoch", "INTEGER NOT NULL DEFAULT 0"),
            ("text_committed", "INTEGER NOT NULL DEFAULT 0"),
            ("outcome_json", "TEXT"),
            ("request", "INTEGER"),
            ("turn_id", "TEXT"),
        ] {
            add_column(&tx, "inputs", name, def)?;
        }
        tx.execute_batch("CREATE TABLE IF NOT EXISTS dialogue_order(seq INTEGER PRIMARY KEY AUTOINCREMENT,turn_id TEXT UNIQUE NOT NULL REFERENCES turns(id));
            CREATE TABLE IF NOT EXISTS observations(id TEXT PRIMARY KEY,created_by_turn_id TEXT NOT NULL REFERENCES turns(id),origin TEXT NOT NULL,observed_at TEXT NOT NULL,time_basis TEXT NOT NULL,scope TEXT NOT NULL,target_label TEXT NOT NULL,summary TEXT NOT NULL,evidence_json TEXT,confidence REAL,legacy_original_text TEXT,forgotten INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS turn_observations(turn_id TEXT NOT NULL REFERENCES turns(id),observation_id TEXT NOT NULL REFERENCES observations(id),relation_kind TEXT NOT NULL CHECK(relation_kind IN ('attached','explicit','continuation_candidate')),ordinal INTEGER NOT NULL,PRIMARY KEY(turn_id,observation_id));
            CREATE TABLE IF NOT EXISTS context_state(id INTEGER PRIMARY KEY CHECK(id=1),history_epoch INTEGER NOT NULL DEFAULT 0,strong_ids TEXT NOT NULL DEFAULT '[]',weak_ids TEXT NOT NULL DEFAULT '[]',anchor_seq INTEGER NOT NULL DEFAULT 0,anchored_at TEXT NOT NULL DEFAULT '');
            INSERT OR IGNORE INTO context_state(id) VALUES(1);
            CREATE TABLE IF NOT EXISTS context_checkpoints(version INTEGER PRIMARY KEY AUTOINCREMENT,history_epoch INTEGER NOT NULL,previous_version INTEGER,through_seq INTEGER NOT NULL,increment_start_seq INTEGER NOT NULL,increment_end_seq INTEGER NOT NULL,source_digest TEXT NOT NULL,body TEXT NOT NULL,source_ids TEXT NOT NULL,token_count INTEGER NOT NULL,tokenizer_version TEXT NOT NULL,prompt_version TEXT NOT NULL,excluded_observations INTEGER NOT NULL DEFAULT 0,created_at TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 0);
            CREATE UNIQUE INDEX IF NOT EXISTS checkpoint_one_active ON context_checkpoints(history_epoch) WHERE active=1;
            CREATE TABLE IF NOT EXISTS turn_searches(turn_id TEXT PRIMARY KEY REFERENCES turns(id),record_json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS deletion_previews(token TEXT PRIMARY KEY,observation_id TEXT NOT NULL,history_epoch INTEGER NOT NULL,scope_digest TEXT NOT NULL,expires_at TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS observation_valid_time ON observations(forgotten,observed_at);
            CREATE INDEX IF NOT EXISTS observation_creator ON observations(created_by_turn_id);
            CREATE INDEX IF NOT EXISTS observation_reverse_refs ON turn_observations(observation_id);
            CREATE INDEX IF NOT EXISTS turn_sync_queue ON turns(forgotten,sync_state);
            CREATE INDEX IF NOT EXISTS input_epoch_status ON inputs(request_epoch,status);
            INSERT OR IGNORE INTO dialogue_order(turn_id) SELECT id FROM turns ORDER BY rowid;").map_err(err)?;
        let legacy = {
            let mut q=tx.prepare("SELECT id,user_text,created_at FROM turns WHERE source='screen' AND forgotten=0 ORDER BY rowid").map_err(err)?;
            let rows = q
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(err)?;
            rows
        };
        for (id, text, time) in legacy {
            let observation_id = format!("legacy:{id}");
            tx.execute("INSERT OR IGNORE INTO observations(id,created_by_turn_id,origin,observed_at,time_basis,scope,target_label,summary,legacy_original_text) VALUES(?1,?2,'legacy',?3,'legacy_turn_time','legacy_unknown','旧版观察',?4,?4)",params![observation_id,id,time,text]).map_err(err)?;
            tx.execute("INSERT OR IGNORE INTO turn_observations(turn_id,observation_id,relation_kind,ordinal) VALUES(?1,?2,'continuation_candidate',0)",params![id,observation_id]).map_err(err)?;
        }
        tx.execute("UPDATE turns SET interaction_kind='observation_comment',user_text='',user_initiated=NULL WHERE source='screen'",[]).map_err(err)?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES('legacy_summary','1')",
            [],
        )
        .map_err(err)?;
        tx.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES('bank_id',?1)",
            [bank_id],
        )
        .map_err(err)?;
        tx.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES('schema_version','2')",
            [],
        )
        .map_err(err)?;
        tx.execute_batch("PRAGMA user_version=2;").map_err(err)?;
        let invalid: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        let integrity: String = tx
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(err)?;
        if invalid || integrity != "ok" {
            return Err("聊天数据库迁移完整性检查失败，已回滚".into());
        }
        tx.commit().map_err(err)?;
    }
    db.execute("UPDATE inputs SET status=CASE WHEN text_committed=1 THEN 'finished' ELSE 'interrupted' END,request=NULL WHERE status IN ('pending','text_committed')",[]).map_err(err)?;
    db.execute("DELETE FROM deletion_previews", [])
        .map_err(err)?;
    Ok(Store { db })
}

fn fingerprint(payload: &ChatSendPayload) -> Result<String> {
    let value = json!({"text":payload.text.trim(),"screen":payload.screen,"expected_history_epoch":payload.expected_history_epoch});
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).map_err(|_| "request_invalid")?)
    ))
}
impl Store {
    pub fn history_epoch(&self) -> Result<u64> {
        self.db
            .query_row(
                "SELECT history_epoch FROM context_state WHERE id=1",
                [],
                |r| r.get(0),
            )
            .map_err(err)
    }
    pub fn request_receipt(&self, id: &str) -> Result<Option<RequestReceipt>> {
        self.db.query_row("SELECT id,request_epoch,status,request,turn_id,text_committed,outcome_json FROM inputs WHERE id=?1",[id],|r|{
            let raw:Option<String>=r.get(6)?;
            Ok(RequestReceipt {client_message_id:r.get(0)?,history_epoch:r.get(1)?,state:r.get(2)?,request:r.get(3)?,turn_id:r.get(4)?,text_committed:r.get(5)?,outcome:raw.and_then(|v|serde_json::from_str(&v).ok())})
        }).optional().map_err(err)
    }
    pub fn check_input(&self, payload: &ChatSendPayload) -> Result<Option<SendOutcome>> {
        payload.validate()?;
        require_epoch(&self.db, payload.expected_history_epoch)?;
        if let Some(receipt) = self.request_receipt(&payload.client_message_id)? {
            let prior: String = self
                .db
                .query_row(
                    "SELECT payload_fingerprint FROM inputs WHERE id=?1",
                    [&payload.client_message_id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if prior != fingerprint(payload)? {
                return Err("request_id_conflict".into());
            }
            if receipt.state == "pending" {
                return Err("request_in_progress".into());
            }
            let mut result = receipt.outcome.unwrap_or(SendOutcome {
                request: receipt.request,
                turn_id: receipt.turn_id,
                result: if receipt.text_committed {
                    "completed"
                } else {
                    "failed"
                }
                .into(),
                text_committed: receipt.text_committed,
                observation_ids: vec![],
                history_epoch: receipt.history_epoch,
                replayed: true,
                error_code: if receipt.text_committed {
                    None
                } else {
                    Some("request_interrupted".into())
                },
                warning_code: None,
            });
            result.replayed = true;
            return Ok(Some(result));
        }
        Ok(None)
    }
    pub fn reserve_input(
        &self,
        payload: &ChatSendPayload,
        request: u64,
    ) -> Result<Option<SendOutcome>> {
        if let Some(outcome) = self.check_input(payload)? {
            return Ok(Some(outcome));
        }
        self.db.execute("INSERT INTO inputs(id,text,status,created_at,payload_fingerprint,request_epoch,request) VALUES(?1,?2,'pending',?3,?4,?5,?6)",params![payload.client_message_id,super::super::providers::redact(payload.text.trim()),chrono::Utc::now().to_rfc3339(),fingerprint(payload)?,payload.expected_history_epoch,request]).map_err(err)?;
        Ok(None)
    }
    pub fn finish_input(&self, id: &str, outcome: &SendOutcome) -> Result<()> {
        require_epoch(&self.db, outcome.history_epoch)?;
        let receipt = self.request_receipt(id)?.ok_or("request_receipt_missing")?;
        if receipt.text_committed && !outcome.text_committed {
            return Err("request_already_committed".into());
        }
        self.db.execute("UPDATE inputs SET status='finished',outcome_json=?2 WHERE id=?1 AND request_epoch=?3",params![id,serde_json::to_string(outcome).map_err(|_|"receipt_invalid")?,outcome.history_epoch]).map_err(err)?;
        Ok(())
    }
    pub fn commit_turn(
        &self,
        value: &Turn,
        metadata: &TurnMetadata,
        draft: Option<&ObservationDraft>,
        references: &[ObservationRef],
        expected_epoch: u64,
        request: Option<u64>,
    ) -> Result<SendOutcome> {
        metadata.validate()?;
        if references.len() > 3
            || references
                .iter()
                .map(|r| &r.observation_id)
                .collect::<HashSet<_>>()
                .len()
                != references.len()
            || references.iter().any(|r| {
                !matches!(
                    r.relation_kind.as_str(),
                    "attached" | "explicit" | "continuation_candidate"
                )
            })
            || value.reply.trim().is_empty()
            || (metadata.interaction_kind == "observation_comment"
                && (!value.user_text.is_empty() || value.source != "screen"))
            || (metadata.interaction_kind == "proactive_care"
                && (!value.user_text.is_empty() || value.source != "care" || metadata.user_initiated != Some(false) || draft.is_some() || !references.is_empty()))
            || (!matches!(metadata.interaction_kind.as_str(), "observation_comment" | "proactive_care")
                && (value.user_text.trim().is_empty() || value.source != "chat"))
        {
            return Err("turn_metadata_invalid".into());
        }
        if let Some(draft) = draft {
            draft.validate()?;
        }
        let tx = self.db.unchecked_transaction().map_err(err)?;
        require_epoch(&tx, expected_epoch)?;
        for r in references {
            if draft.is_some_and(|d| d.id == r.observation_id) {
                continue;
            }
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM observations WHERE id=?1 AND forgotten=0)",
                    [&r.observation_id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !exists {
                return Err("observation_unavailable".into());
            }
            if r.relation_kind == "attached" {
                return Err("observation_reference_invalid".into());
            }
        }
        if let Some(draft) = draft {
            if !references.iter().any(|r| r.observation_id == draft.id) {
                return Err("observation_reference_invalid".into());
            }
        }
        tx.execute("INSERT INTO turns(id,source,user_text,reply,japanese,created_at,operation_id,interaction_kind,finish_kind,user_initiated) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![value.id,value.source,value.user_text,value.reply,value.japanese,value.created_at,super::super::settings::random_id()?,metadata.interaction_kind,metadata.finish_kind,metadata.user_initiated]).map_err(err)?;
        tx.execute(
            "INSERT INTO dialogue_order(turn_id) VALUES(?1)",
            [&value.id],
        )
        .map_err(err)?;
        let seq = tx.last_insert_rowid();
        if let Some(draft) = draft {
            tx.execute("INSERT INTO observations(id,created_by_turn_id,origin,observed_at,time_basis,scope,target_label,summary,evidence_json,confidence) VALUES(?1,?2,?3,?4,'captured',?5,?6,?7,?8,?9)",params![draft.id,value.id,draft.origin,draft.observed_at,draft.scope,draft.target_label,draft.evidence.summary,serde_json::to_string(&draft.evidence).map_err(|_|"screen_vision_invalid")?,draft.evidence.confidence]).map_err(err)?;
        }
        for r in references {
            tx.execute("INSERT INTO turn_observations(turn_id,observation_id,relation_kind,ordinal) VALUES(?1,?2,?3,?4)",params![value.id,r.observation_id,r.relation_kind,r.ordinal]).map_err(err)?;
        }
        if let Some(web) = &metadata.web_context {
            tx.execute(
                "INSERT INTO turn_searches(turn_id,record_json) VALUES(?1,?2)",
                params![
                    value.id,
                    serde_json::to_string(web).map_err(|_| "search_record_invalid")?
                ],
            )
            .map_err(err)?;
        }
        let strong = references
            .iter()
            .filter(|r| matches!(r.relation_kind.as_str(), "attached" | "explicit"))
            .map(|r| &r.observation_id)
            .collect::<Vec<_>>();
        if !strong.is_empty() && metadata.interaction_kind != "observation_comment" {
            tx.execute("UPDATE context_state SET strong_ids=?1,weak_ids='[]',anchor_seq=?2,anchored_at=?3 WHERE id=1",params![serde_json::to_string(&strong).map_err(|_|"reference_invalid")?,seq,value.created_at]).map_err(err)?;
        } else if metadata.interaction_kind != "observation_comment" && !references.is_empty() {
            let weak: String = tx
                .query_row("SELECT weak_ids FROM context_state WHERE id=1", [], |r| {
                    r.get(0)
                })
                .map_err(err)?;
            if weak == "[]" {
                let ids = references
                    .iter()
                    .map(|r| &r.observation_id)
                    .collect::<Vec<_>>();
                tx.execute(
                    "UPDATE context_state SET weak_ids=?1,anchor_seq=?2,anchored_at=?3 WHERE id=1",
                    params![
                        serde_json::to_string(&ids).map_err(|_| "reference_invalid")?,
                        seq,
                        value.created_at
                    ],
                )
                .map_err(err)?;
            }
        }
        let outcome = SendOutcome {
            request,
            turn_id: Some(value.id.clone()),
            result: "completed".into(),
            text_committed: true,
            observation_ids: references
                .iter()
                .map(|r| r.observation_id.clone())
                .collect(),
            history_epoch: expected_epoch,
            replayed: false,
            error_code: None,
            warning_code: None,
        };
        tx.execute("UPDATE inputs SET text_committed=1,status='text_committed',turn_id=?1,outcome_json=?2 WHERE id=?1 AND request_epoch=?3",params![value.id,serde_json::to_string(&outcome).map_err(|_|"receipt_invalid")?,expected_epoch]).map_err(err)?;
        if metadata.interaction_kind == "proactive_care" {
            tx.execute("UPDATE turns SET sync_state='local_only' WHERE id=?1",[&value.id]).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(outcome)
    }
    pub fn references(&self, id: &str) -> Result<Vec<ObservationRef>> {
        let mut q=self.db.prepare("SELECT r.observation_id,r.relation_kind,r.ordinal FROM turn_observations r JOIN observations o ON o.id=r.observation_id WHERE r.turn_id=?1 AND o.forgotten=0 ORDER BY r.ordinal").map_err(err)?;
        let values = q
            .query_map([id], |r| {
                Ok(ObservationRef {
                    observation_id: r.get(0)?,
                    relation_kind: r.get(1)?,
                    ordinal: r.get(2)?,
                })
            })
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        Ok(values)
    }
    pub fn web_context(&self, id: &str) -> Result<Option<Value>> {
        let raw:Option<String>=self.db.query_row("SELECT s.record_json FROM turn_searches s JOIN turns t ON t.id=s.turn_id WHERE t.id=?1 AND t.forgotten=0",[id],|r|r.get(0)).optional().map_err(err)?;
        raw.map(|s| serde_json::from_str(&s).map_err(|_| "search_record_invalid".into()))
            .transpose()
    }
    pub fn metadata(&self, id: &str) -> Result<TurnMetadata> {
        let mut value=self.db.query_row("SELECT interaction_kind,finish_kind,user_initiated FROM turns WHERE id=?1 AND forgotten=0",[id],|r|Ok(TurnMetadata {interaction_kind:r.get(0)?,finish_kind:r.get(1)?,user_initiated:r.get(2)?,web_context:None})).map_err(err)?;
        value.web_context = self.web_context(id)?;
        Ok(value)
    }
    fn enrich(&self, rows: Vec<(Turn, u64)>) -> Result<Vec<HistoryTurn>> {
        rows.into_iter()
            .map(|(turn, seq)| {
                let metadata = self.metadata(&turn.id)?;
                let observation_refs = self.references(&turn.id)?;
                Ok(HistoryTurn {
                    turn,
                    seq,
                    metadata,
                    observation_refs,
                })
            })
            .collect()
    }
    pub fn history(&self, limit: usize) -> Result<Vec<HistoryTurn>> {
        let mut q=self.db.prepare("SELECT t.id,t.source,t.user_text,t.reply,t.japanese,t.created_at,d.seq FROM turns t JOIN dialogue_order d ON d.turn_id=t.id WHERE t.forgotten=0 ORDER BY d.seq DESC LIMIT ?1").map_err(err)?;
        let mut rows = q
            .query_map([limit.min(200) as u64], |r| {
                Ok((turn(r)?, r.get::<_, u64>(6)?))
            })
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        rows.reverse();
        self.enrich(rows)
    }
    pub fn history_range(&self, after: u64, through: u64) -> Result<Vec<HistoryTurn>> {
        let mut q=self.db.prepare("SELECT t.id,t.source,t.user_text,t.reply,t.japanese,t.created_at,d.seq FROM turns t JOIN dialogue_order d ON d.turn_id=t.id WHERE t.forgotten=0 AND d.seq>?1 AND d.seq<=?2 ORDER BY d.seq").map_err(err)?;
        let rows = q
            .query_map(params![after, through.min(i64::MAX as u64)], |r| {
                Ok((turn(r)?, r.get::<_, u64>(6)?))
            })
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        self.enrich(rows)
    }
    pub fn latest_seq(&self) -> Result<u64> {
        self.db
            .query_row("SELECT COALESCE(MAX(seq),0) FROM dialogue_order", [], |r| {
                r.get(0)
            })
            .map_err(err)
    }
    pub fn dialogues_named_in(&self, input: &str) -> Result<Vec<HistoryTurn>> {
        static IDS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let pattern = IDS.get_or_init(|| {
            regex::Regex::new(
                r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            )
            .unwrap()
        });
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for id in pattern.find_iter(input).take(3) {
            if !seen.insert(id.as_str()) {
                continue;
            }
            let row=self.db.query_row("SELECT t.id,t.source,t.user_text,t.reply,t.japanese,t.created_at,d.seq FROM turns t JOIN dialogue_order d ON d.turn_id=t.id WHERE t.id=?1 AND t.forgotten=0",[id.as_str()],|r|Ok((turn(r)?,r.get::<_,u64>(6)?))).optional().map_err(err)?;
            if let Some(row) = row {
                result.extend(self.enrich(vec![row])?);
            }
        }
        Ok(result)
    }
    pub fn get_observations(&self, ids: &[String]) -> Result<Vec<Observation>> {
        validate_ids(ids)?;
        let mut result = Vec::new();
        for id in ids {
            result.push(self.db.query_row("SELECT id,created_by_turn_id,origin,observed_at,time_basis,scope,target_label,summary,evidence_json,confidence,legacy_original_text FROM observations WHERE id=?1 AND forgotten=0",[id],|r|{
            let raw:Option<String>=r.get(8)?;Ok(Observation {id:r.get(0)?,created_by_turn_id:r.get(1)?,origin:r.get(2)?,observed_at:r.get(3)?,time_basis:r.get(4)?,scope:r.get(5)?,target_label:r.get(6)?,summary:r.get(7)?,evidence:raw.and_then(|v|serde_json::from_str(&v).ok()),confidence:r.get(9)?,legacy_original_text:r.get(10)?})
        }).optional().map_err(err)?.ok_or("observation_unavailable")?);
        }
        Ok(result)
    }
    pub fn latest_observation(&self) -> Result<Option<Observation>> {
        let id:Option<String>=self.db.query_row("SELECT o.id FROM observations o JOIN dialogue_order d ON d.turn_id=o.created_by_turn_id WHERE o.forgotten=0 ORDER BY d.seq DESC LIMIT 1",[],|r|r.get(0)).optional().map_err(err)?;
        id.map(|id| self.get_observations(&[id]).map(|mut v| v.remove(0)))
            .transpose()
    }
    pub fn reference_state(&self) -> Result<ReferenceState> {
        self.db.query_row("SELECT strong_ids,weak_ids,anchor_seq,anchored_at,history_epoch FROM context_state WHERE id=1",[],|r|{
            let strong:String=r.get(0)?;let weak:String=r.get(1)?;
            Ok(ReferenceState {strong_ids:serde_json::from_str(&strong).unwrap_or_default(),weak_ids:serde_json::from_str(&weak).unwrap_or_default(),anchor_seq:r.get(2)?,anchored_at:r.get(3)?,history_epoch:r.get(4)?})
        }).map_err(err)
    }
    pub fn observation_candidates(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<ObservationRef>> {
        let state = self.reference_state()?;
        let mut ids = state.strong_ids;
        let weak_valid = chrono::DateTime::parse_from_rfc3339(&state.anchored_at)
            .ok()
            .is_some_and(|t| now.signed_duration_since(t).num_minutes() < 30);
        if weak_valid {
            for id in state.weak_ids {
                if ids.len() < 3 && !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        let since = (now - chrono::Duration::minutes(10)).to_rfc3339();
        let mut q=self.db.prepare("SELECT id FROM observations WHERE forgotten=0 AND origin IN ('automatic','manual_once') AND observed_at>=?1 ORDER BY observed_at DESC LIMIT 2").map_err(err)?;
        let recent = q
            .query_map([since], |r| r.get::<_, String>(0))
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        for id in recent {
            if ids.len() < 3 && !ids.contains(&id) {
                ids.push(id);
            }
        }
        let mut result = Vec::new();
        for id in ids.into_iter().take(3) {
            if self.get_observations(&[id.clone()]).is_ok() {
                result.push(ObservationRef {
                    observation_id: id,
                    relation_kind: "continuation_candidate".into(),
                    ordinal: result.len() as u32,
                });
            }
        }
        Ok(result)
    }
    pub fn export(&self) -> Result<Value> {
        let dialogues = self.history_range(0, i64::MAX as u64)?;
        let mut observations = Vec::new();
        let mut references = Vec::new();
        let mut seen = HashSet::new();
        for row in &dialogues {
            for r in &row.observation_refs {
                references.push(json!({"turn_id":row.turn.id,"observation_id":r.observation_id,"relation_kind":r.relation_kind,"ordinal":r.ordinal}));
                if seen.insert(r.observation_id.clone()) {
                    observations.extend(self.get_observations(&[r.observation_id.clone()])?);
                }
            }
        }
        Ok(
            json!({"schema_version":2,"dialogues":dialogues,"observations":observations,"references":references}),
        )
    }
    pub fn memory_content(&self, id: &str) -> Result<String> {
        let value=self.db.query_row("SELECT id,source,user_text,reply,japanese,created_at FROM turns WHERE id=?1 AND forgotten=0",[id],turn).map_err(err)?;
        let metadata = self.metadata(id)?;
        let mut parts = vec![format!(
            "记录 ID：{}；来源：{}；结束状态：{}；时间：{}",
            value.id, metadata.interaction_kind, metadata.finish_kind, value.created_at
        )];
        if !value.user_text.is_empty() {
            parts.push(format!(
                "用户原话（问题和假设不自动等于事实声明）：{}",
                value.user_text
            ));
        } else {
            parts.push("此轮没有用户自述；下面是应用观察与主动发言。".into());
        }
        for reference in self.references(id)? {
            let o = self
                .get_observations(&[reference.observation_id])?
                .remove(0);
            parts.push(format!("某时屏幕资料 ID={}；时间={}（{}）；来源={}；概括={}；不确定性={}。这不是用户声明。",o.id,o.observed_at,o.time_basis,o.origin,o.summary,o.evidence.as_ref().map(|e|e.uncertainties.join("；")).unwrap_or_else(||"旧版未提供置信度".into())));
        }
        let searched = metadata
            .web_context
            .as_ref()
            .and_then(|v| v.get("attempted"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !searched && metadata.interaction_kind == "chat" {
            parts.push(format!(
                "AI 实际回复（不是用户声明；未完成回答不代表完整承诺）：{}",
                value.reply
            ));
        } else if !searched {
            parts.push("AI 对观察的回答属于交谈关系，不应抽取成独立用户事实。".into());
        }
        Ok(parts.join("\n"))
    }
    pub fn sync_state_at_epoch(&self, id: &str, state: &str, expected: u64) -> Result<()> {
        require_epoch(&self.db, expected)?;
        self.sync_state(id, state)
    }

    pub fn active_checkpoint(&self) -> Result<Option<Checkpoint>> {
        self.db.query_row("SELECT version,history_epoch,previous_version,through_seq,increment_start_seq,increment_end_seq,source_digest,body,source_ids,token_count,tokenizer_version,prompt_version,excluded_observations,created_at FROM context_checkpoints WHERE active=1 AND history_epoch=(SELECT history_epoch FROM context_state WHERE id=1)",[],|r|{
            let ids:String=r.get(8)?;Ok(Checkpoint {version:r.get(0)?,history_epoch:r.get(1)?,previous_version:r.get(2)?,through_seq:r.get(3)?,increment_start_seq:r.get(4)?,increment_end_seq:r.get(5)?,source_digest:r.get(6)?,body:r.get(7)?,source_ids:serde_json::from_str(&ids).unwrap_or_default(),token_count:r.get(9)?,tokenizer_version:r.get(10)?,prompt_version:r.get(11)?,excluded_observations:r.get(12)?,created_at:r.get(13)?})
        }).optional().map_err(err)
    }
    pub fn source_digest(&self, after: u64, through: u64) -> Result<String> {
        let rows = self.history_range(after, through)?;
        // Translations and sync receipts may change after text commit. Neither affects the summarized prefix.
        let stable=rows.iter().map(|r|json!({"id":r.turn.id,"seq":r.seq,"user":r.turn.user_text,"reply":r.turn.reply,"metadata":r.metadata,"refs":r.observation_refs})).collect::<Vec<_>>();
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&stable).map_err(|_| "checkpoint_invalid")?)
        ))
    }
    pub fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<bool> {
        let tx = self.db.unchecked_transaction().map_err(err)?;
        require_epoch(&tx, checkpoint.history_epoch)?;
        let active = self.active_checkpoint()?;
        if active.as_ref().map(|c| c.version) != checkpoint.previous_version {
            return Ok(false);
        }
        let previous = active.as_ref().map(|c| c.through_seq).unwrap_or(0);
        if checkpoint.through_seq <= previous
            || checkpoint.increment_start_seq <= previous
            || checkpoint.increment_end_seq != checkpoint.through_seq
            || checkpoint.body.trim().is_empty()
            || checkpoint.token_count > 8_192
            || checkpoint.source_digest != self.source_digest(previous, checkpoint.through_seq)?
        {
            return Err("checkpoint_invalid".into());
        }
        let rows = self.history_range(previous, checkpoint.through_seq)?;
        let ids: HashSet<&str> = rows
            .iter()
            .map(|r| r.turn.id.as_str())
            .chain(
                active
                    .as_ref()
                    .into_iter()
                    .flat_map(|c| c.source_ids.iter().map(String::as_str)),
            )
            .collect();
        if checkpoint
            .source_ids
            .iter()
            .any(|id| !ids.contains(id.as_str()))
        {
            return Err("checkpoint_source_invalid".into());
        }
        tx.execute("UPDATE context_checkpoints SET active=0 WHERE active=1", [])
            .map_err(err)?;
        tx.execute("INSERT INTO context_checkpoints(history_epoch,previous_version,through_seq,increment_start_seq,increment_end_seq,source_digest,body,source_ids,token_count,tokenizer_version,prompt_version,excluded_observations,created_at,active) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,1)",params![checkpoint.history_epoch,checkpoint.previous_version,checkpoint.through_seq,checkpoint.increment_start_seq,checkpoint.increment_end_seq,checkpoint.source_digest,checkpoint.body,serde_json::to_string(&checkpoint.source_ids).map_err(|_|"checkpoint_invalid")?,checkpoint.token_count,checkpoint.tokenizer_version,checkpoint.prompt_version,checkpoint.excluded_observations,checkpoint.created_at]).map_err(err)?;
        tx.execute("DELETE FROM context_checkpoints WHERE active=0 AND version NOT IN (SELECT version FROM context_checkpoints ORDER BY version DESC LIMIT 3)",[]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(true)
    }
    fn deletion_scope(&self, id: &str) -> Result<(Vec<String>, Vec<String>, String)> {
        self.get_observations(&[id.into()])?;
        let mut q=self.db.prepare("SELECT DISTINCT t.id FROM turns t JOIN turn_observations r ON r.turn_id=t.id WHERE r.observation_id=?1 AND t.forgotten=0 ORDER BY t.id").map_err(err)?;
        let turns = q
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        let mut observations = vec![id.to_string()];
        for id in &turns {
            let mut q = self
                .db
                .prepare("SELECT id FROM observations WHERE created_by_turn_id=?1 AND forgotten=0")
                .map_err(err)?;
            observations.extend(
                q.query_map([id], |r| r.get::<_, String>(0))
                    .map_err(err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(err)?,
            );
        }
        observations.sort();
        observations.dedup();
        let digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&turns, &observations))
                    .map_err(|_| "deletion_scope_invalid")?
            )
        );
        Ok((turns, observations, digest))
    }
    pub fn preview_forget_observation(&self, id: &str) -> Result<ForgetObservationPreview> {
        let (turns, observations, digest) = self.deletion_scope(id)?;
        let token = super::super::settings::random_id()?;
        let history_epoch = self.history_epoch()?;
        self.db.execute("INSERT INTO deletion_previews(token,observation_id,history_epoch,scope_digest,expires_at) VALUES(?1,?2,?3,?4,?5)",params![token,id,history_epoch,digest,(chrono::Utc::now()+chrono::Duration::minutes(5)).to_rfc3339()]).map_err(err)?;
        Ok(ForgetObservationPreview {
            observation_id: id.into(),
            dialogue_count: turns.len(),
            observation_count: observations.len(),
            history_epoch,
            preview_token: token,
        })
    }
    pub fn forget_observation(&mut self, id: &str, token: &str, expected: u64) -> Result<()> {
        // The transaction is acquired before recalculating scope, so another connection cannot expand it after validation.
        let tx = self.db.unchecked_transaction().map_err(err)?;
        require_epoch(&tx, expected)?;
        let preview:Option<(String,u64,String,String)>=tx.query_row("SELECT observation_id,history_epoch,scope_digest,expires_at FROM deletion_previews WHERE token=?1",[token],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(err)?;
        let (preview_id, preview_epoch, digest, expires) =
            preview.ok_or("deletion_scope_changed")?;
        if preview_id != id
            || preview_epoch != expected
            || expires < chrono::Utc::now().to_rfc3339()
        {
            return Err("deletion_scope_changed".into());
        }
        let (turns, observations, current) = self.deletion_scope(id)?;
        if current != digest {
            return Err("deletion_scope_changed".into());
        }
        forget_rows(&tx, &turns, &observations)?;
        invalidate_derived(&tx)?;
        tx.commit().map_err(err)
    }
    pub(super) fn forget_v2(&mut self, id: Option<&str>) -> Result<()> {
        let turns = if let Some(id) = id {
            vec![id.to_string()]
        } else {
            self.all()?.into_iter().map(|t| t.id).collect()
        };
        let mut observations = Vec::new();
        for id in &turns {
            let mut q = self
                .db
                .prepare("SELECT id FROM observations WHERE created_by_turn_id=?1 AND forgotten=0")
                .map_err(err)?;
            observations.extend(
                q.query_map([id], |r| r.get::<_, String>(0))
                    .map_err(err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(err)?,
            );
        }
        let tx = self.db.transaction().map_err(err)?;
        forget_rows(&tx, &turns, &observations)?;
        invalidate_derived(&tx)?;
        tx.commit().map_err(err)
    }
}
fn forget_rows(tx: &Transaction<'_>, turns: &[String], observations: &[String]) -> Result<()> {
    for id in observations {
        tx.execute("UPDATE observations SET forgotten=1,summary='',evidence_json=NULL,legacy_original_text=NULL,target_label='' WHERE id=?1",[id]).map_err(err)?;
        tx.execute(
            "DELETE FROM turn_observations WHERE observation_id=?1",
            [id],
        )
        .map_err(err)?;
    }
    for id in turns {
        tx.execute("UPDATE turns SET forgotten=1,user_text='',reply='',japanese='',sync_state='done' WHERE id=?1",[id]).map_err(err)?;
        tx.execute("DELETE FROM turn_observations WHERE turn_id=?1", [id])
            .map_err(err)?;
        tx.execute("DELETE FROM turn_searches WHERE turn_id=?1", [id])
            .map_err(err)?;
    }
    Ok(())
}
fn invalidate_derived(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("UPDATE context_state SET history_epoch=history_epoch+1,strong_ids='[]',weak_ids='[]',anchor_seq=0,anchored_at='' WHERE id=1;DELETE FROM inputs;DELETE FROM summaries;DELETE FROM context_checkpoints;DELETE FROM deletion_previews;DELETE FROM meta WHERE key IN ('user_profile','profile_count');INSERT OR REPLACE INTO meta(key,value) VALUES('rebuild_required','1');").map_err(err)
}
