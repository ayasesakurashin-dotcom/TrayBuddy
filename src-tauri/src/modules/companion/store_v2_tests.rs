use super::super::{
    context,
    observations::*,
    settings::{self, Endpoint},
};
use super::*;
use serde_json::json;

fn turn(user: &str, reply: &str) -> Turn {
    Turn {
        id: settings::random_id().unwrap(),
        source: "chat".into(),
        user_text: user.into(),
        reply: reply.into(),
        japanese: String::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}
fn draft() -> ObservationDraft {
    ObservationDraft {
        id: settings::random_id().unwrap(),
        origin: "attached_question".into(),
        observed_at: chrono::Utc::now().to_rfc3339(),
        scope: "foreground".into(),
        target_label: "编辑器".into(),
        evidence: VisualEvidence {
            usable: true,
            sensitive: false,
            confidence: 0.9,
            answerability: "partial".into(),
            summary: "画面显示错误代码 E42".into(),
            visible_text: "E42".into(),
            facts: vec![],
            uncertainties: vec!["看不清底部文字".into()],
        },
    }
}
fn reference(id: &str, relation: &str) -> ObservationRef {
    ObservationRef {
        observation_id: id.into(),
        relation_kind: relation.into(),
        ordinal: 0,
    }
}
fn commit(s: &Store, t: &Turn, d: Option<&ObservationDraft>, refs: &[ObservationRef]) {
    s.commit_turn(
        t,
        &TurnMetadata::default(),
        d,
        refs,
        s.history_epoch().unwrap(),
        None,
    )
    .unwrap();
}
fn store() -> Store {
    Store::open(Path::new(":memory:"), "test-bank").unwrap()
}

#[test]
fn legacy_screen_migration_keeps_actual_reply_but_separates_material() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.sqlite");
    {
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);INSERT INTO meta VALUES('bank_id','test-bank');CREATE TABLE turns(id TEXT PRIMARY KEY,source TEXT NOT NULL,user_text TEXT NOT NULL,reply TEXT NOT NULL,japanese TEXT NOT NULL DEFAULT '',created_at TEXT NOT NULL,forgotten INTEGER NOT NULL DEFAULT 0,sync_state TEXT NOT NULL DEFAULT 'pending',operation_id TEXT NOT NULL);INSERT INTO turns VALUES('old','screen','旧观察原文','实际说过的话','','2026-10-22T00:00:00Z',0,'done','old-operation');").unwrap();
    }
    let s = Store::open(&path, "test-bank").unwrap();
    let rows = s.history(20).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].turn.reply, "实际说过的话");
    assert_eq!(rows[0].turn.user_text, "");
    assert_eq!(rows[0].metadata.interaction_kind, "observation_comment");
    let observation = s.latest_observation().unwrap().unwrap();
    assert_eq!(observation.summary, "旧观察原文");
    assert_eq!(observation.time_basis, "legacy_turn_time");
    assert_eq!(s.export().unwrap()["schema_version"], 2);
    drop(s);
    let reopened = Store::open(&path, "test-bank").unwrap();
    assert_eq!(reopened.history(20).unwrap().len(), 1);
    assert_eq!(
        reopened.get_observations(&[observation.id]).unwrap().len(),
        1
    );
    assert!(std::fs::read_dir(dir.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("schema1-backup")));
}
#[test]
fn unknown_schema_and_identity_mismatch_do_not_initialize_empty_data() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("future.sqlite");
    let db = Connection::open(&p).unwrap();
    db.execute_batch("PRAGMA user_version=7;CREATE TABLE important(value TEXT);INSERT INTO important VALUES('preserve');").unwrap();
    assert!(Store::open(&p, "test-bank").is_err());
    assert_eq!(
        db.query_row("SELECT value FROM important", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "preserve"
    );
    let p2 = dir.path().join("owned.sqlite");
    drop(Store::open(&p2, "owner-a").unwrap());
    assert!(Store::open(&p2, "owner-b").is_err());
}
#[test]
fn receipt_and_text_commit_are_idempotent_and_payload_bound() {
    let s = store();
    let t = turn("我的问题", "正文");
    let p = ChatSendPayload {
        client_message_id: t.id.clone(),
        expected_history_epoch: 0,
        text: t.user_text.clone(),
        screen: ScreenSelection::None,
    };
    assert!(s.check_input(&p).unwrap().is_none());
    s.reserve_input(&p, 5).unwrap();
    assert_eq!(s.check_input(&p).unwrap_err(), "request_in_progress");
    let outcome = s
        .commit_turn(&t, &TurnMetadata::default(), None, &[], 0, Some(5))
        .unwrap();
    assert!(
        s.request_receipt(&p.client_message_id)
            .unwrap()
            .unwrap()
            .text_committed
    );
    s.finish_input(&p.client_message_id, &outcome).unwrap();
    assert!(s.check_input(&p).unwrap().unwrap().replayed);
    let mut changed = p.clone();
    changed.text = "其他问题".into();
    assert_eq!(s.check_input(&changed).unwrap_err(), "request_id_conflict");
    assert_eq!(s.history(20).unwrap().len(), 1);
}
#[test]
fn failed_reference_rolls_back_text_observation_and_sources() {
    let s = store();
    let t = turn("问题", "正文");
    let d = draft();
    let meta = TurnMetadata {
        web_context: Some(json!({"attempted":true,"sources":[{"url":"https://example.org"}]})),
        ..Default::default()
    };
    assert!(s
        .commit_turn(
            &t,
            &meta,
            Some(&d),
            &[
                reference(&d.id, "attached"),
                reference("missing", "explicit")
            ],
            0,
            None
        )
        .is_err());
    assert!(s.history(20).unwrap().is_empty());
    assert!(s.latest_observation().unwrap().is_none());
    assert!(s.web_context(&t.id).is_err() || s.web_context(&t.id).unwrap().is_none());
}
#[test]
fn background_observation_cannot_replace_user_strong_reference() {
    let s = store();
    let a = draft();
    commit(
        &s,
        &turn("讨论A", "答复A"),
        Some(&a),
        &[reference(&a.id, "attached")],
    );
    let mut b = draft();
    b.origin = "automatic".into();
    let mut tb = turn("", "短评论");
    tb.source = "screen".into();
    s.commit_turn(
        &tb,
        &TurnMetadata {
            interaction_kind: "observation_comment".into(),
            user_initiated: Some(false),
            ..Default::default()
        },
        Some(&b),
        &[reference(&b.id, "attached")],
        0,
        None,
    )
    .unwrap();
    assert_eq!(s.reference_state().unwrap().strong_ids, vec![a.id.clone()]);
    assert_eq!(
        s.observation_candidates(chrono::Utc::now()).unwrap()[0].observation_id,
        a.id
    );
}
#[test]
fn forgetting_one_turn_invalidates_its_picture_without_erasing_other_dialogue() {
    let mut s = store();
    let d = draft();
    let a = turn("看当前屏幕", "观察答复");
    commit(&s, &a, Some(&d), &[reference(&d.id, "attached")]);
    let b = turn("这是我补充的真实文字", "后续答复");
    commit(&s, &b, None, &[reference(&d.id, "explicit")]);
    s.forget(Some(&a.id)).unwrap();
    assert_eq!(s.history_epoch().unwrap(), 1);
    let history = s.history(20).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].turn.user_text, b.user_text);
    assert!(history[0].observation_refs.is_empty());
    assert!(s.get_observations(&[d.id]).is_err());
    assert!(s.needs_rebuild());
    assert!(!s.memory_content(&b.id).unwrap().contains("E42"));
    assert_eq!(s.export().unwrap()["observations"], json!([]));
}
#[test]
fn observation_preview_rejects_new_references_and_deletes_only_confirmed_direct_scope() {
    let mut s = store();
    let d = draft();
    let a = turn("问题A", "答复A");
    commit(&s, &a, Some(&d), &[reference(&d.id, "attached")]);
    let unrelated = turn("无关问题", "无关答复");
    commit(&s, &unrelated, None, &[]);
    let preview = s.preview_forget_observation(&d.id).unwrap();
    let b = turn("跟进A", "答复B");
    commit(&s, &b, None, &[reference(&d.id, "explicit")]);
    assert_eq!(
        s.forget_observation(&d.id, &preview.preview_token, 0)
            .unwrap_err(),
        "deletion_scope_changed"
    );
    let preview = s.preview_forget_observation(&d.id).unwrap();
    assert_eq!(preview.dialogue_count, 2);
    s.forget_observation(&d.id, &preview.preview_token, 0)
        .unwrap();
    assert_eq!(s.history(20).unwrap()[0].turn.id, unrelated.id);
    assert_eq!(s.history(20).unwrap().len(), 1);
}
#[test]
fn searched_answers_and_source_fragments_do_not_enter_memory_or_rebuild() {
    let mut s = store();
    let t = turn("这位角色的声优是谁？", "外部答案不应成为用户事实");
    let meta = TurnMetadata {
        web_context: Some(
            json!({"attempted":true,"sources":[{"excerpt":"网页原文","url":"https://example.org"}]}),
        ),
        ..Default::default()
    };
    s.commit_turn(&t, &meta, None, &[], 0, None).unwrap();
    let text = s.memory_content(&t.id).unwrap();
    assert!(text.contains("声优是谁"));
    assert!(!text.contains("外部答案不应"));
    assert!(!text.contains("网页原文"));
    let disposable = turn("临时文字", "临时答复");
    commit(&s, &disposable, None, &[]);
    s.forget(Some(&disposable.id)).unwrap();
    s.rebuilt().unwrap();
    let pending = s.pending().unwrap().unwrap();
    assert!(!pending.serialized_content.contains("网页原文"));
    assert!(!pending.serialized_content.contains("外部答案不应"));
}
#[test]
fn checkpoints_accept_appends_but_reject_stale_epoch_and_incomplete_output() {
    let mut s = store();
    let endpoint = Endpoint {
        base_url: "https://api.deepseek.com".into(),
        model: "deepseek-flash".into(),
        ..Default::default()
    };
    for _ in 0..7 {
        commit(&s, &turn("问题", &"资料".repeat(35000)), None, &[]);
    }
    let snapshot = context::summary_snapshot(&s, &endpoint).unwrap().unwrap();
    assert!(snapshot.chunks.len() > 1);
    let latest = turn("追加的新问题", "追加的新回复");
    commit(&s, &latest, None, &[]);
    let raw = json!({"summary":"保留用户问题与尚未完成事项","source_ids":snapshot.source_ids})
        .to_string();
    assert!(context::commit_checkpoint(&s, &snapshot, &raw, Some("length")).is_err());
    assert!(s.active_checkpoint().unwrap().is_none());
    assert!(context::commit_checkpoint(&s, &snapshot, &raw, Some("stop")).unwrap());
    assert!(s
        .history_range(snapshot.through_seq, s.latest_seq().unwrap())
        .unwrap()
        .iter()
        .any(|r| r.turn.id == latest.id));
    s.forget(Some(&latest.id)).unwrap();
    assert!(context::commit_checkpoint(&s, &snapshot, &raw, Some("stop")).is_err());
}
#[test]
fn named_original_can_be_recovered_after_checkpoint_without_rag_or_profile_leak() {
    let s = store();
    let a = turn("旧问题", "必须精确引用的旧原句");
    commit(&s, &a, None, &[]);
    let checkpoint = Checkpoint {
        version: 0,
        history_epoch: 0,
        previous_version: None,
        through_seq: 1,
        increment_start_seq: 1,
        increment_end_seq: 1,
        source_digest: s.source_digest(0, 1).unwrap(),
        body: "旧话题已摘要".into(),
        source_ids: vec![a.id.clone()],
        token_count: 20,
        tokenizer_version: "test".into(),
        prompt_version: "test".into(),
        excluded_observations: 0,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    s.save_checkpoint(&checkpoint).unwrap();
    commit(&s, &turn("新问题", "新答案"), None, &[]);
    s.set_profile("这是被关闭的Hindsight派生资料").unwrap();
    let endpoint = Endpoint {
        base_url: "https://api.deepseek.com".into(),
        model: "deepseek-flash".into(),
        ..Default::default()
    };
    let normal = context::build(
        &s,
        &endpoint,
        "稳定人格",
        "继续聊",
        &[],
        None,
        "",
        "召回秘密",
        false,
        &[],
    )
    .unwrap();
    let normal = serde_json::to_string(&normal.messages).unwrap();
    assert!(!normal.contains("必须精确引用的旧原句"));
    assert!(!normal.contains("被关闭的Hindsight"));
    assert!(!normal.contains("召回秘密"));
    let quoted = context::build(
        &s,
        &endpoint,
        "稳定人格",
        &format!("请引用记录 {} 的原句", a.id),
        &[],
        None,
        "",
        "",
        false,
        &[],
    )
    .unwrap();
    assert!(serde_json::to_string(&quoted.messages)
        .unwrap()
        .contains("必须精确引用的旧原句"));
}
#[test]
fn old_external_answers_are_time_marked_and_retry_keeps_the_same_screen_evidence() {
    let s = store();
    let t = turn("公开事实问题", "历史外部答案");
    s.commit_turn(
        &t,
        &TurnMetadata {
            web_context: Some(json!({"attempted":true,"completed_at":"2026-10-22T00:00:00Z"})),
            ..Default::default()
        },
        None,
        &[],
        0,
        None,
    )
    .unwrap();
    let endpoint = Endpoint {
        base_url: "https://api.deepseek.com".into(),
        model: "deepseek-flash".into(),
        ..Default::default()
    };
    let d = draft();
    let build = context::build(
        &s,
        &endpoint,
        "稳定人格",
        "屏幕问题",
        &[],
        Some(&d),
        "",
        "",
        false,
        &[],
    )
    .unwrap();
    assert!(serde_json::to_string(&build.messages)
        .unwrap()
        .contains("不是本轮实时核查"));
    let retry = context::retry_build(
        &s,
        &endpoint,
        "稳定人格",
        "屏幕问题",
        &[],
        Some(&d),
        "",
        "",
        false,
        &[],
    )
    .unwrap();
    assert_eq!(retry.observation_refs[0].observation_id, d.id);
    assert!(s.latest_observation().unwrap().is_none());
    assert!(retry.estimate_tokens <= 16 * 1024);
}
