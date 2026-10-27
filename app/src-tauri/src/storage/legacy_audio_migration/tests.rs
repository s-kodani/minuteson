use super::*;
use crate::storage::{AnalysisInput, JobKind, JobSpec, UtteranceInput};

#[test]
fn migration_preserves_published_text_and_restarts_unfinished_analysis_once() {
    let (_temp, storage, meeting, _) = super::super::audio_publication::tests::fixture();
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE recording_sessions SET audio_policy='legacy' WHERE meeting_id=?1",
            [&meeting],
        )
        .unwrap();
    let (hash, track): (String, String) = {
        let db = storage.connection.lock().unwrap();
        db.query_row(
        "SELECT r.audio_manifest_hash,t.id FROM recording_sessions r JOIN audio_tracks t ON t.session_id=r.id WHERE r.meeting_id=?1 AND t.kind='mic'",[&meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
    };
    let revision = storage
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: hash.clone(),
                profile_snapshot: serde_json::json!({}),
                config: serde_json::json!({}),
                speakers: vec![],
                segments: vec![],
                utterances: vec![UtteranceInput {
                    track_id: track,
                    start_us: 0,
                    end_us: 200000,
                    text: "移行前に保存した本文".into(),
                    speaker_keys: vec![],
                    confidence: None,
                }],
            },
        )
        .unwrap();
    let lock = storage.get_meeting(&meeting).unwrap().unwrap().lock_version;
    let committed = storage
        .commit_analysis_candidate(&meeting, &revision, lock, false)
        .unwrap();
    storage.ensure_analysis_flow(&meeting).unwrap();
    let spec = |operation: &str| JobSpec {
        meeting_id: meeting.clone(),
        kind: JobKind::Transcription,
        operation_id: operation.into(),
        input_analysis_id: None,
        input_hash: hash.clone(),
        profile_snapshot: serde_json::json!({}),
        parameters: serde_json::json!({"schemaVersion":4}),
        total_units: Some(1),
    };
    let old = storage.enqueue_job(spec("legacy-pending")).unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    storage
        .save_job_checkpoint(&old.id, 0, 0, serde_json::json!({"outputs":[]}))
        .unwrap();
    storage.migrate_single_wav(&|_, _, _| {}).unwrap();
    assert_eq!(storage.get_job(&old.id).unwrap().state, "cancelled");
    assert!(storage.fixed_job_input(&old.id).unwrap().1.is_some());
    let flow = storage.analysis_flow(&meeting).unwrap().unwrap();
    assert_eq!(flow.stage, "pending");
    assert_eq!(
        storage
            .pending_analysis_target(&flow.id)
            .unwrap()
            .unwrap()
            .analysis_id,
        revision
    );
    let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
    assert_eq!(view.id, revision);
    assert_eq!(view.content_hash, committed.content_hash);
    assert_eq!(view.utterances[0].effective_text, "移行前に保存した本文");
    assert_eq!(
        view.utterances[0].track_id,
        storage.canonical_audio_plan(&meeting).unwrap().tracks[0].id
    );
    assert!(storage.verify_analysis_content_hash(&revision).unwrap());
    let new = storage.enqueue_job(spec("new-work-after-upgrade")).unwrap();
    storage.migrate_single_wav(&|_, _, _| {}).unwrap();
    assert_eq!(storage.get_job(&new.id).unwrap().state, "queued");
    assert_eq!(
        storage.analysis_flow(&meeting).unwrap().unwrap().id,
        flow.id
    );
}

#[test]
fn only_the_known_previous_schema_is_admitted_by_the_bridge() {
    assert!(admits_schema(34));
    assert!(!admits_schema(33));
    assert!(!admits_schema(36));
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL,applied_at INTEGER NOT NULL);").unwrap();
    for &(version, sql) in &super::super::MIGRATIONS[..34] {
        db.execute_batch(sql).unwrap();
        db.execute(
            "INSERT INTO schema_migrations VALUES(?1,?2,0)",
            params![version, format!("{:x}", Sha256::digest(sql.as_bytes()))],
        )
        .unwrap();
    }
    upgrade_schema(&mut db).unwrap();
    upgrade_schema(&mut db).unwrap();
    assert_eq!(super::super::verify_migration_history(&db).unwrap(), 35);
    assert!(!db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
}

#[test]
fn failed_active_audio_flow_restarts_with_new_input_without_resuming_old_plan() {
    let (_temp, storage, meeting, _) = super::super::audio_publication::tests::fixture();
    storage
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE recording_sessions SET audio_policy='legacy'", [])
        .unwrap();
    storage.ensure_analysis_flow(&meeting).unwrap();
    let original = storage.analysis_flow(&meeting).unwrap().unwrap();
    let hash: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT audio_manifest_hash FROM recording_sessions WHERE meeting_id=?1",
            [&meeting],
            |r| r.get(0),
        )
        .unwrap();
    let job = storage
        .enqueue_job(JobSpec {
            meeting_id: meeting.clone(),
            kind: JobKind::Transcription,
            operation_id: "failed-legacy-operation".into(),
            input_analysis_id: None,
            input_hash: hash,
            profile_snapshot: serde_json::json!({}),
            parameters: serde_json::json!({"schemaVersion":4}),
            total_units: Some(1),
        })
        .unwrap();
    storage.acquire_next_job().unwrap().unwrap();
    storage
        .finish_job(&job.id, 0, "failed", Some("synthetic_failure"))
        .unwrap();
    storage.connection.lock().unwrap().execute("UPDATE analysis_flows SET stage='attention',transcription_job_id=?2 WHERE meeting_id=?1",params![meeting,job.id]).unwrap();
    storage.migrate_single_wav(&|_, _, _| {}).unwrap();
    let next = storage.analysis_flow(&meeting).unwrap().unwrap();
    assert_eq!(next.stage, "pending");
    assert_ne!(next.id, original.id);
    assert_eq!(storage.get_job(&job.id).unwrap().state, "failed");
}

#[test]
#[ignore = "requires an explicitly supplied isolated single-wav-migration-copy; never source app data"]
fn supplied_recording_copy_migrates_to_one_audio_without_changing_documents() {
    let root = PathBuf::from(
        std::env::var_os("MON_SINGLE_WAV_COPY_ROOT").expect("isolated copy required"),
    )
    .canonicalize()
    .unwrap();
    assert!(root.ends_with("single-wav-migration-copy"));
    assert!(root.join("single-wav-copy.json").is_file());
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("clock-repair-copy.json")).unwrap())
            .unwrap();
    let meeting = marker["meetingId"].as_str().unwrap();
    let storage = Storage::open(root.clone()).unwrap();
    let documents = || {
        let db = storage.connection.lock().unwrap();
        let mut hash = Sha256::new();
        for table in [
            "analysis_revisions",
            "utterances",
            "minutes_revisions",
            "minutes_markdown",
            "working_drafts",
            "edit_operations",
            "voice_samples",
        ] {
            let mut q = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let count = q.column_count();
            let mut rows = q.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for i in 0..count {
                    hash.update(format!("{:?}", row.get_ref(i).unwrap()).as_bytes());
                }
            }
        }
        format!("{:x}", hash.finalize())
    };
    let before = documents();
    let started = std::time::Instant::now();
    storage.recover_completed_recordings().unwrap();
    storage
        .migrate_single_wav(&|done, total, _| eprintln!("migration {done}/{total}"))
        .unwrap();
    let plan = storage.verify_canonical_audio(meeting, &|| false).unwrap();
    let count:i64=storage.connection.lock().unwrap().query_row("SELECT count(*) FROM file_assets WHERE meeting_id=?1 AND media_type LIKE 'audio/%' AND state='ready'",[meeting],|r|r.get(0)).unwrap();
    assert_eq!(count, 1);
    assert_eq!(documents(), before);
    storage.migrate_single_wav(&|_, _, _| {}).unwrap();
    assert_eq!(storage.canonical_audio_plan(meeting).unwrap(), plan);
    let result = serde_json::json!({"meetingId":meeting,"frames":plan.tracks[0].chunks[0].frames,"durationUs":plan.tracks[0].end_us,"audioBytes":plan.tracks[0].chunks[0].size,"sha256":plan.tracks[0].chunks[0].hash,"documentsHash":before,"elapsedSeconds":started.elapsed().as_secs_f64()});
    std::fs::write(
        root.join("single-wav-result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}
