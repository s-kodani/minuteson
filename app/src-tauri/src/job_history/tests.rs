use super::*;

#[test]
fn model_check_outcome_uses_structured_state_before_message_wording() {
    let cancelled = Ok(serde_json::json!({"state":"cancelled","errorMessage":"operation stopped"}));
    assert_eq!(model_failure(&cancelled).unwrap().0, "cancelled");
    let failed = Ok(serde_json::json!({"state":"failed","errorMessage":"取消後の検証エラー"}));
    assert_eq!(model_failure(&failed).unwrap().0, "failed");
    assert!(model_failure(&Ok(serde_json::json!({"state":"passed"}))).is_none());
    assert_eq!(
        model_failure(&Err::<(), _>("処理を取り消しました".into()))
            .unwrap()
            .0,
        "cancelled"
    );
}

#[test]
fn worker_imports_more_than_queue_capacity_and_coalesces_progress_before_persisting() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::start(dir.path().into(), None);
    let time = now();
    let entries = (0..4200)
        .map(|id| entry(&format!("{id:05}"), "succeeded", time))
        .collect();
    assert!(service.tx.try_send(Message::Backfill(entries)).is_ok());
    let page = service
        .query(
            Query {
                limit: Some(100),
                ..Default::default()
            },
            BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(page.items.len(), 100);
    assert!(page.next_cursor.is_some());
    assert!(page.storage_error.is_none());
    let mut running = Entry::new("model", "one-operation", "モデル", None, "running");
    for _ in 0..5000 {
        running.observed_at += 1;
        service.record(running.clone());
    }
    let mut done = running;
    done.state = "succeeded".into();
    done.finished_at = Some(time + 1);
    service.record(done);
    let page = service
        .query(
            Query {
                kind: Some("model".into()),
                ..Default::default()
            },
            BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].state, "succeeded");
    assert!(page.storage_error.is_none());
}

fn entry(id: &str, state: &str, time: i64) -> Entry {
    let mut e = Entry::new("transcription", id, "会議", None, state);
    e.accepted_at = Some(time - 10);
    e.observed_at = time;
    e.finished_at = if state == "succeeded" {
        Some(time)
    } else {
        None
    };
    e
}
#[test]
fn paging_is_stable_for_identical_times_and_duplicates_and_filters_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path()).unwrap();
    for id in ["a", "b", "c"] {
        assert!(upsert(&db, entry(id, "succeeded", 1000), 1000).unwrap());
    }
    assert!(!upsert(&db, entry("a", "succeeded", 1000), 1000).unwrap());
    let page = query(
        &db,
        Query {
            limit: Some(2),
            ..Default::default()
        },
        &BTreeMap::new(),
        1000,
    )
    .unwrap();
    assert_eq!(
        page.items.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["transcription:c", "transcription:b"]
    );
    let next = query(
        &db,
        Query {
            cursor: page.next_cursor,
            limit: Some(2),
            ..Default::default()
        },
        &BTreeMap::new(),
        1000,
    )
    .unwrap();
    assert_eq!(next.items.len(), 1);
    assert_eq!(next.items[0].id, "transcription:a");
    assert!(next.next_cursor.is_none());
    assert!(query(
        &db,
        Query {
            kind: Some("unknown".into()),
            ..Default::default()
        },
        &BTreeMap::new(),
        1000
    )
    .is_err());
    assert!(query(
        &db,
        Query {
            limit: Some(101),
            ..Default::default()
        },
        &BTreeMap::new(),
        1000
    )
    .is_err());
}
#[test]
fn ninety_day_boundary_retains_active_work_and_never_prunes_source_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path()).unwrap();
    for (id, state, t) in [
        ("expired", "succeeded", 999),
        ("boundary", "succeeded", 1000),
        ("active", "running", 900),
    ] {
        upsert(&db, entry(id, state, t), t).unwrap();
    }
    let page = query(&db, Query::default(), &BTreeMap::new(), RETENTION_MS + 1000).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, "transcription:boundary");
    assert_eq!(
        db.query_row("SELECT count(*) FROM history WHERE active=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn deleted_meetings_erase_names_and_reject_late_records_but_keep_anonymous_deletion_summary() {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path()).unwrap();
    let mut e = entry("source", "succeeded", 1000);
    e.meeting_id = Some("meeting".into());
    e.target = "削除された会議名".into();
    upsert(&db, e.clone(), 1000).unwrap();
    let mut deletion = Entry::new("deletion", "operation", "会議 1件", None, "succeeded");
    deletion.observed_at = 1000;
    deletion.finished_at = Some(1000);
    upsert(&db, deletion, 1000).unwrap();
    let page = query(&db, Query::default(), &BTreeMap::new(), 1000).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].kind, "deletion");
    e.observed_at = 1001;
    assert!(!upsert(&db, e, 1001).unwrap());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM history WHERE target LIKE '%削除された%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn cold_start_marks_unconfirmed_work_interrupted_and_preserves_confirmed_results() {
    let dir = tempfile::tempdir().unwrap();
    {
        let db = open(dir.path()).unwrap();
        upsert(&db, entry("active", "running", 1000), 1000).unwrap();
        upsert(&db, entry("done", "succeeded", 1000), 1000).unwrap();
    }
    let db = open(dir.path()).unwrap();
    let page = query(&db, Query::default(), &BTreeMap::new(), 1001).unwrap();
    assert_eq!(page.items.len(), 2);
    let interrupted = page
        .items
        .iter()
        .find(|e| e.id.ends_with(":active"))
        .unwrap();
    assert_eq!(interrupted.state, "interrupted");
    assert!(interrupted.finished_at.is_none());
    let mut stale = entry("done", "running", 999);
    stale.finished_at = None;
    assert!(!upsert(&db, stale, 1001).unwrap());
}
#[test]
fn new_defaults_are_used_only_when_not_saved_and_threshold_boundaries_are_inclusive() {
    use crate::storage::{
        completion_preferences::CompletionPreferences, voice_preferences::VoiceMatchingPreferences,
    };
    let voice = VoiceMatchingPreferences::default();
    let completion = CompletionPreferences::default();
    assert_eq!(voice.similarity_threshold_milli, 700);
    assert_eq!(completion.similarity_threshold_milli, 700);
    assert!(voice.accepts(0.7, None, true));
    assert!(!voice.accepts(0.699, None, true));
    assert!(completion.accepts(0.7, Some(0.6), true));
    assert!(!completion.accepts(0.7, Some(0.601), true));
}
#[test]
fn stored_old_preferences_survive_reopen_instead_of_being_migrated_to_new_defaults() {
    use crate::storage::{
        completion_preferences::CompletionPreferences, voice_preferences::VoiceMatchingPreferences,
        Storage,
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::open(dir.path().into()).unwrap();
    let mut diarization = store.diarization_preferences().unwrap().payload;
    assert_eq!(diarization.standard.cluster_threshold_milli, 1100);
    assert_eq!(diarization.high_accuracy.cluster_threshold_milli, 1100);
    diarization.standard.cluster_threshold_milli = 500;
    diarization.high_accuracy.cluster_threshold_milli = 1300;
    store.save_diarization_preferences(0, diarization).unwrap();
    store
        .save_voice_matching_preferences(
            0,
            VoiceMatchingPreferences {
                similarity_threshold_milli: 800,
                margin_threshold_milli: 100,
            },
        )
        .unwrap();
    store
        .save_speaker_completion_preferences(
            0,
            CompletionPreferences {
                similarity_threshold_milli: 800,
                margin_threshold_milli: 50,
            },
        )
        .unwrap();
    drop(store);
    let store = Storage::open(dir.path().into()).unwrap();
    assert_eq!(
        store
            .diarization_preferences()
            .unwrap()
            .payload
            .standard
            .cluster_threshold_milli,
        500
    );
    assert_eq!(
        store
            .diarization_preferences()
            .unwrap()
            .payload
            .high_accuracy
            .cluster_threshold_milli,
        1300
    );
    assert_eq!(
        store
            .voice_matching_preferences()
            .unwrap()
            .payload
            .similarity_threshold_milli,
        800
    );
    assert_eq!(
        store
            .speaker_completion_preferences()
            .unwrap()
            .payload
            .similarity_threshold_milli,
        800
    );
}
