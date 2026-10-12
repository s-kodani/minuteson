use super::super::analysis::tests::{input, saved_meeting};
use super::super::drafts::{SpeakerNameEdit, UtteranceEdit};

#[test]
fn manual_splits_materialize_fresh_lineages_original_scope_and_fixed_minutes_without_rewriting_old_input(
) {
    use super::super::draft_splits::{SplitPart, UtteranceSplit};
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (mut request, original, draft) = fixture(&storage);
    let old_job = enqueue(&storage, request.clone());
    let old_input = storage.load_minutes_input(&old_job.id).unwrap();
    let mut p = draft.payload.clone();
    p.schema_version = 3;
    let u = &original.utterances[0];
    let text = p
        .utterances
        .iter()
        .find(|e| e.utterance_id == u.id)
        .unwrap()
        .effective_text
        .clone();
    let chars: Vec<_> = text.chars().collect();
    let middle = u.start_us + (u.end_us - u.start_us) / 2;
    let first = format!("split:{}", Uuid::new_v4());
    let second = format!("split:{}", Uuid::new_v4());
    p.utterance_splits.push(UtteranceSplit {
        source_utterance_id: u.id.clone(),
        source_effective_text: text.clone(),
        parts: vec![
            SplitPart {
                id: first.clone(),
                start_us: u.start_us,
                end_us: middle,
                effective_text: chars[..4].iter().collect(),
                speaker_ids: vec![original.speakers[0].id.clone()],
            },
            SplitPart {
                id: second.clone(),
                start_us: middle,
                end_us: u.end_us,
                effective_text: chars[4..].iter().collect(),
                speaker_ids: vec![],
            },
        ],
    });
    let saved = storage
        .save_transcript_draft(&draft.id, draft.lock_version, p)
        .unwrap();
    request.operation_id = "split-fixed".into();
    request.expected_draft = Some(DraftStamp {
        id: saved.id.clone(),
        lock_version: saved.lock_version,
    });
    let job = enqueue(&storage, request.clone());
    let fixed = storage.load_minutes_input(&job.id).unwrap();
    assert_eq!(
        fixed.analysis.utterances.len(),
        original.utterances.len() + 1
    );
    let parts: Vec<_> = fixed
        .analysis
        .utterances
        .iter()
        .filter(|u| u.recognition_is_source)
        .collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(
        parts
            .iter()
            .map(|u| u.effective_text.as_str())
            .collect::<String>(),
        text
    );
    assert!(parts
        .iter()
        .all(|part| part.recognized_text == u.recognized_text));
    assert_eq!(
        (
            parts[0].start_us,
            parts[0].end_us,
            parts[1].start_us,
            parts[1].end_us
        ),
        (u.start_us, middle, middle, u.end_us)
    );
    assert!(parts[1].speaker_ids.is_empty());
    assert_eq!(storage.load_minutes_input(&old_job.id).unwrap(), old_input);
    assert_eq!(
        storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap(),
        original
    );
    assert!(!storage.minutes_input_updated(&job.id).unwrap());
    assert!(storage.minutes_input_updated(&old_job.id).unwrap());
    {
        let db = storage.connection.lock().unwrap();
        let lineages = lineage_map(&db, "utterances", &fixed.analysis.id).unwrap();
        assert_eq!(lineages[&parts[0].id], first);
        assert_eq!(lineages[&parts[1].id], second);
        let old = lineage_map(&db, "utterances", &original.id).unwrap();
        assert!(parts.iter().all(|part| lineages[&part.id] != old[&u.id]));
        for part in &parts {
            assert_eq!(
                db.query_row(
                    "SELECT confidence FROM utterances WHERE id=?1",
                    params![part.id],
                    |r| r.get::<_, Option<f64>>(0)
                )
                .unwrap(),
                None
            );
        }
        assert!(db
            .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
            .optional()
            .unwrap()
            .is_none());
    }
    let mut edited = saved.payload.clone();
    edited.utterances.push(UtteranceEdit {
        utterance_id: first,
        effective_text: "分割後の修正".into(),
        speaker_ids: vec![],
    });
    let edited = storage
        .save_transcript_draft(&saved.id, saved.lock_version, edited)
        .unwrap();
    assert!(storage.minutes_input_updated(&job.id).unwrap());
    assert_eq!(storage.load_minutes_input(&job.id).unwrap(), fixed);
    storage
        .undo_last_transcript_edit(&edited.id, edited.lock_version)
        .unwrap();
    assert!(!storage.minutes_input_updated(&job.id).unwrap());
    drop(storage);
    let storage = Storage::open(root.path().to_owned()).unwrap();
    assert_eq!(storage.load_minutes_input(&old_job.id).unwrap(), old_input);
    assert_eq!(storage.load_minutes_input(&job.id).unwrap(), fixed);
}

#[test]
fn added_and_merged_speakers_materialize_fixed_inputs_without_rewriting_old_references() {
    use super::super::drafts::{ManualSpeaker, SpeakerMerge};
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (mut request, original, draft) = fixture(&storage);
    let before = enqueue(&storage, request.clone());
    let old_fixed = storage.load_minutes_input(&before.id).unwrap();
    let mut payload = draft.payload.clone();
    payload.schema_version = 2;
    let manual = format!("manual:{}", Uuid::new_v4());
    payload.manual_speakers.push(ManualSpeaker {
        id: manual.clone(),
        label: "手動話者 1".into(),
    });
    payload.speaker_names.push(SpeakerNameEdit {
        speaker_id: manual.clone(),
        display_name: Some("確定氏名".into()),
    });
    payload.speaker_merges.push(SpeakerMerge {
        source_id: original.speakers[0].id.clone(),
        target_id: manual.clone(),
    });
    let saved = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    assert!(storage.minutes_input_updated(&before.id).unwrap());
    request.operation_id = "after-speaker-edit".into();
    request.expected_draft = Some(DraftStamp {
        id: saved.id.clone(),
        lock_version: saved.lock_version,
    });
    let after = enqueue(&storage, request.clone());
    let fixed = storage.load_minutes_input(&after.id).unwrap();
    let new = fixed
        .analysis
        .speakers
        .iter()
        .find(|s| s.display_name.as_deref() == Some("確定氏名"))
        .unwrap();
    assert_ne!(new.id, manual);
    assert_eq!(
        fixed.analysis.utterances[0].speaker_ids,
        vec![new.id.clone()]
    );
    assert_eq!(
        fixed.analysis.utterances[0].effective_text,
        "点検日を確定しました"
    );
    assert_eq!(
        fixed.analysis.utterances[0].recognized_text,
        original.utterances[0].recognized_text
    );
    assert!(fixed.analysis.utterances[1].speaker_ids.is_empty());
    assert!(!storage.minutes_input_updated(&after.id).unwrap());
    assert_eq!(
        storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(storage.load_minutes_input(&before.id).unwrap(), old_fixed);
    {
        let db = storage.connection.lock().unwrap();
        let lineage: String = db
            .query_row(
                "SELECT lineage_id FROM speakers WHERE id=?1",
                params![new.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(lineage, manual);
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM speaker_segments WHERE analysis_revision_id=?1",
                params![fixed.analysis.id],
                |r| r.get(0),
            )
            .unwrap();
        let old_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM speaker_segments WHERE analysis_revision_id=?1",
                params![original.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, old_count);
        let config: String = db
            .query_row(
                "SELECT config_json FROM analysis_revisions WHERE id=?1",
                params![fixed.analysis.id],
                |r| r.get(0),
            )
            .unwrap();
        let config: Value = serde_json::from_str(&config).unwrap();
        assert_eq!(
            config["manualSpeakerOperations"]["merges"][0]["target_id"],
            manual
        );
        assert!(db
            .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
            .optional()
            .unwrap()
            .is_none());
    }
    storage
        .undo_last_transcript_edit(&saved.id, saved.lock_version)
        .unwrap();
    assert!(!storage.minutes_input_updated(&before.id).unwrap());
    assert!(storage.minutes_input_updated(&after.id).unwrap());
    drop(storage);
    let storage = Storage::open(root.path().to_owned()).unwrap();
    assert_eq!(storage.load_minutes_input(&before.id).unwrap(), old_fixed);
    assert_eq!(storage.load_minutes_input(&after.id).unwrap(), fixed);
}

#[test]
fn migration_from_v12_preserves_existing_drafts_heads_and_job_hashes() {
    let root = tempfile::tempdir().unwrap();
    let connection = Connection::open(root.path().join("meetings.sqlite3")).unwrap();
    connection.execute_batch("PRAGMA foreign_keys = ON; CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY,checksum TEXT NOT NULL,applied_at INTEGER NOT NULL);").unwrap();
    for (index, sql) in [
        super::super::MIGRATION_1,
        super::super::MIGRATION_2,
        super::super::MIGRATION_3,
        super::super::MIGRATION_4,
        super::super::MIGRATION_5,
        super::super::MIGRATION_6,
        super::super::MIGRATION_7,
        super::super::MIGRATION_8,
        super::super::MIGRATION_9,
        super::super::MIGRATION_10,
        super::super::MIGRATION_11,
        super::super::MIGRATION_12,
    ]
    .into_iter()
    .enumerate()
    {
        connection.execute_batch(sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations VALUES (?1,?2,0)",
                params![
                    index as i64 + 1,
                    format!("{:x}", Sha256::digest(sql.as_bytes()))
                ],
            )
            .unwrap();
    }
    let storage = Storage {
        deletion_fence: crate::storage::deletion::fence::Fence::open(&connection).unwrap(),
        #[cfg(unix)]
        managed_anchor: crate::storage::deletion::fs::RootAnchor::owned_image(
            root.path(),
            "meetings.sqlite3",
        )
        .unwrap(),
        scratch: None,
        #[cfg(target_os = "macos")]
        flac_history: None,
        #[cfg(unix)]
        native_models_root: root.path().join("models"),
        maintenance: std::sync::atomic::AtomicBool::new(false),
        retired: std::sync::atomic::AtomicBool::new(false),
        root: root.path().to_owned(),
        connection: Mutex::new(connection),
        restored_inputs: Mutex::default(),
    };
    let (request, original, draft) = fixture(&storage);
    let spec = JobSpec {
        meeting_id: request.meeting_id.clone(),
        kind: JobKind::Transcription,
        operation_id: "legacy-operation".into(),
        input_analysis_id: None,
        input_hash: "a".repeat(64),
        profile_snapshot: json!({"engine":"legacy"}),
        parameters: json!({}),
        total_units: Some(1),
    };
    // Encode the actual v12 row, without invoking v14 DTO queries on an old DB.
    let mut hash = Sha256::new();
    hash.update(b"mon-job-spec-v1");
    hash.update(serde_json::to_vec(&spec).unwrap());
    let hash = format!("{:x}", hash.finalize());
    storage.connection.lock().unwrap().execute("INSERT INTO processing_jobs (id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,total_units,created_at,updated_at) VALUES ('legacy',?1,'transcription','queued',?2,?3,?4,?5,?6,1,0,0)",params![spec.meeting_id,spec.operation_id,spec.input_hash,spec.profile_snapshot.to_string(),spec.parameters.to_string(),hash]).unwrap();
    drop(storage);
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let legacy = storage.get_job("legacy").unwrap();
    assert_eq!(legacy.spec_hash, hash);
    assert_eq!(legacy.state, "queued");
    assert_eq!(legacy.output_minutes_id, None);
    assert_eq!(storage.fixed_job_input("legacy").unwrap(), (spec, None));
    assert_eq!(
        storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        storage
            .load_transcript_draft(&request.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
    let job = enqueue(&storage, request);
    assert_eq!(
        storage
            .load_minutes_input(&job.id)
            .unwrap()
            .analysis
            .utterances[0]
            .effective_text,
        "点検日を確定しました"
    );
    let db = storage.connection.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        MIGRATIONS.len() as i64
    );
    assert!(db
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap()
        .is_none());
}

#[test]
fn input_update_detection_tracks_content_not_draft_versions_and_handles_no_draft() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (request, _, draft) = fixture(&storage);
    let job = enqueue(&storage, request.clone());
    let saved = storage
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload.clone())
        .unwrap();
    assert!(!storage.minutes_input_updated(&job.id).unwrap());
    let mut payload = saved.payload.clone();
    payload.speaker_names[0].display_name = Some("名前の修正".into());
    let changed = storage
        .save_transcript_draft(&draft.id, saved.lock_version, payload)
        .unwrap();
    assert!(storage.minutes_input_updated(&job.id).unwrap());
    storage
        .undo_last_transcript_edit(&changed.id, changed.lock_version)
        .unwrap();
    assert!(!storage.minutes_input_updated(&job.id).unwrap());
    let meeting = storage.get_meeting(&request.meeting_id).unwrap().unwrap();
    let renamed = storage
        .rename_meeting(&meeting.id, "変更後の会議名", meeting.lock_version)
        .unwrap();
    assert!(storage.minutes_input_updated(&job.id).unwrap());
    storage
        .rename_meeting(&meeting.id, &meeting.title, renamed.lock_version)
        .unwrap();
    assert!(!storage.minutes_input_updated(&job.id).unwrap());

    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let meeting = saved_meeting(&storage);
    let id = storage
        .create_analysis_candidate(&meeting.id, input())
        .unwrap();
    let committed = storage
        .commit_analysis_candidate(&meeting.id, &id, meeting.lock_version, false)
        .unwrap();
    let job = enqueue(
        &storage,
        MinutesJobRequest {
            meeting_id: meeting.id.clone(),
            operation_id: "no-draft".into(),
            expected_analysis_id: id.clone(),
            expected_content_hash: committed.content_hash,
            expected_meeting_lock_version: committed.meeting_lock_version,
            expected_draft: None,
        },
    );
    assert!(!storage.minutes_input_updated(&job.id).unwrap());
    storage.create_transcript_draft(&meeting.id, &id).unwrap();
    assert!(!storage.minutes_input_updated(&job.id).unwrap());
    let fixed = storage.load_minutes_input(&job.id).unwrap();
    assert_eq!(fixed.analysis.utterances[0].speaker_ids.len(), 2);
    assert_eq!(fixed.meeting_started_at_utc_ms, None); // Do not invent a meeting date from creation time.
    assert_eq!(fixed.analysis.utterances[0].attribution_state, "overlap");
}
use super::*;

fn fixture(storage: &Storage) -> (MinutesJobRequest, AnalysisView, TranscriptDraft) {
    let meeting = saved_meeting(storage);
    storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE meetings SET started_at_utc_ms = 1791028800000 WHERE id = ?1",
            params![meeting.id],
        )
        .unwrap();
    let mut input = input();
    input
        .utterances
        .push(super::super::analysis::UtteranceInput {
            track_id: "track".into(),
            start_us: 800_000,
            end_us: 950_000,
            text: "未確認の発言".into(),
            speaker_keys: vec!["b".into()],
            confidence: None,
        });
    let id = storage
        .create_analysis_candidate(&meeting.id, input)
        .unwrap();
    let committed = storage
        .commit_analysis_candidate(&meeting.id, &id, meeting.lock_version, false)
        .unwrap();
    let view = storage.load_current_analysis(&meeting.id).unwrap().unwrap();
    let draft = storage.create_transcript_draft(&meeting.id, &id).unwrap();
    let mut payload = draft.payload.clone();
    payload.speaker_names.push(SpeakerNameEdit {
        speaker_id: view.speakers[0].id.clone(),
        display_name: Some("田中".into()),
    });
    payload.utterances.push(UtteranceEdit {
        utterance_id: view.utterances[0].id.clone(),
        effective_text: "点検日を確定しました".into(),
        speaker_ids: vec![view.speakers[0].id.clone()],
    });
    payload.utterances.push(UtteranceEdit {
        utterance_id: view.utterances[1].id.clone(),
        effective_text: String::new(),
        speaker_ids: vec![],
    });
    let draft = storage
        .save_transcript_draft(&draft.id, 0, payload)
        .unwrap();
    let request = MinutesJobRequest {
        meeting_id: meeting.id,
        operation_id: "minutes-first".into(),
        expected_analysis_id: id,
        expected_content_hash: committed.content_hash,
        expected_meeting_lock_version: committed.meeting_lock_version,
        expected_draft: Some(DraftStamp {
            id: draft.id.clone(),
            lock_version: draft.lock_version,
        }),
    };
    (request, view, draft)
}

fn enqueue(storage: &Storage, request: MinutesJobRequest) -> JobSnapshot {
    storage
        .enqueue_minutes_job(
            request,
            json!({"engine":"llama.cpp", "modelHash":"c".repeat(64)}),
            json!({"templateVersion":"standard-v1"}),
            Some(1),
        )
        .unwrap()
}

#[test]
fn fixed_minutes_input_remaps_edits_and_preserves_public_heads_across_reopen() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (request, original, edited) = fixture(&storage);
    let meeting = storage.get_meeting(&request.meeting_id).unwrap().unwrap();
    let job = enqueue(&storage, request.clone());
    let fixed = storage.load_minutes_input(&job.id).unwrap();
    assert_ne!(fixed.analysis.id, original.id);
    assert_eq!(fixed.meeting_started_at_utc_ms, Some(1791028800000));
    assert_eq!(fixed.analysis.utterances.len(), 2);
    assert_eq!(
        fixed.analysis.utterances[0].effective_text,
        "点検日を確定しました"
    );
    assert_eq!(
        fixed.analysis.utterances[0].recognized_text,
        original.utterances[0].recognized_text
    );
    assert_eq!(fixed.analysis.utterances[0].attribution_state, "assigned");
    assert_eq!(fixed.analysis.utterances[0].speaker_ids.len(), 1);
    assert_eq!(
        fixed.analysis.speakers[0].display_name.as_deref(),
        Some("田中")
    );
    assert_eq!(fixed.analysis.utterances[1].effective_text, "");
    assert_eq!(fixed.analysis.utterances[1].attribution_state, "unknown");
    assert!(fixed.analysis.utterances[1].speaker_ids.is_empty());
    for u in &fixed.analysis.utterances {
        assert!(!original.utterances.iter().any(|old| old.id == u.id));
    }
    for s in &fixed.analysis.speakers {
        assert!(!original.speakers.iter().any(|old| old.id == s.id));
    }
    assert!(storage
        .load_visible_analysis(&request.meeting_id, &fixed.analysis.id)
        .unwrap()
        .is_none());
    assert_eq!(
        storage
            .analysis_versions(&request.meeting_id)
            .unwrap()
            .versions
            .len(),
        1
    );
    assert_eq!(
        storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        storage
            .load_transcript_draft(&request.meeting_id)
            .unwrap()
            .unwrap(),
        edited
    );
    assert_eq!(
        storage.get_meeting(&request.meeting_id).unwrap().unwrap(),
        meeting
    );
    assert!(!storage.minutes_input_updated(&job.id).unwrap());
    let db = storage.connection.lock().unwrap();
    let segments: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM speaker_segments WHERE analysis_revision_id = ?1",
            params![fixed.analysis.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(segments, 2);
    let valid: bool = db.query_row("SELECT NOT EXISTS(SELECT 1 FROM speaker_segments WHERE analysis_revision_id = ?1 AND speaker_id NOT IN (SELECT id FROM speakers WHERE analysis_revision_id = ?1))", params![fixed.analysis.id], |r| r.get(0)).unwrap();
    assert!(valid);
    let lineage: String = db
        .query_row(
            "SELECT lineage_id FROM utterances WHERE id = ?1",
            params![fixed.analysis.utterances[0].id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lineage, original.utterances[0].id);
    drop(db);
    let mut payload = edited.payload.clone();
    payload
        .utterances
        .iter_mut()
        .find(|e| e.utterance_id == original.utterances[0].id)
        .unwrap()
        .effective_text = "生成中に続ける編集".into();
    let next_draft = storage
        .save_transcript_draft(&edited.id, edited.lock_version, payload)
        .unwrap();
    assert!(storage.minutes_input_updated(&job.id).unwrap());
    assert_eq!(storage.load_minutes_input(&job.id).unwrap(), fixed);
    assert_eq!(enqueue(&storage, request.clone()), job); // Same operation despite later edits.
    assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, job.id);
    storage.request_job_pause(&job.id, "user").unwrap();
    storage
        .save_job_checkpoint(&job.id, 0, 0, json!({"stage":"extract","results":[]}))
        .unwrap();
    drop(storage);
    let storage = Storage::open(root.path().to_owned()).unwrap();
    assert_eq!(storage.load_minutes_input(&job.id).unwrap(), fixed);
    assert_eq!(
        storage
            .load_transcript_draft(&request.meeting_id)
            .unwrap()
            .unwrap(),
        next_draft
    );
    storage.resume_job(&job.id).unwrap();
    assert_eq!(storage.acquire_next_job().unwrap().unwrap().id, job.id);
    assert!(storage
        .verify_analysis_content_hash(&fixed.analysis.id)
        .unwrap());
}

#[test]
fn minutes_input_rejects_stale_requests_and_operation_collisions_without_writes() {
    for field in [
        "analysis",
        "hash",
        "meeting",
        "lock",
        "draft",
        "missing_draft",
        "parameters",
        "profile",
    ] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let (mut request, original, edited) = fixture(&storage);
        let mut profile = json!({});
        let mut parameters = json!({});
        match field {
            "analysis" => request.expected_analysis_id = "missing".into(),
            "hash" => request.expected_content_hash = "f".repeat(64),
            "meeting" => request.meeting_id = storage.create_meeting("別会議").unwrap().id,
            "lock" => request.expected_meeting_lock_version += 1,
            "draft" => request.expected_draft.as_mut().unwrap().lock_version += 1,
            "missing_draft" => request.expected_draft = None,
            "parameters" => parameters = json!({"tooLarge":"x".repeat(1024*1024)}),
            "profile" => profile = json!({"tooLarge":"x".repeat(65536)}),
            _ => unreachable!(),
        }
        assert!(
            storage
                .enqueue_minutes_job(request, profile, parameters, Some(1))
                .is_err(),
            "{field}"
        );
        let db = storage.connection.lock().unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM analysis_revisions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "{field}");
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM processing_jobs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(db);
        assert_eq!(
            storage
                .load_current_analysis(&original.meeting_id)
                .unwrap()
                .unwrap(),
            original
        );
        assert_eq!(
            storage
                .load_transcript_draft(&edited.meeting_id)
                .unwrap()
                .unwrap(),
            edited
        );
    }
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (mut request, _, _) = fixture(&storage);
    let job = enqueue(&storage, request.clone());
    request.expected_draft.as_mut().unwrap().lock_version += 1;
    assert!(storage
        .enqueue_minutes_job(request, json!({}), json!({}), Some(1))
        .is_err());
    assert_eq!(storage.list_jobs(&job.meeting_id).unwrap(), vec![job]);
}

#[test]
fn failed_minutes_job_insert_rolls_back_snapshot_and_all_remapped_children() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (request, original, draft) = fixture(&storage);
    storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER simulate_minutes_failure BEFORE INSERT ON processing_jobs WHEN NEW.kind = 'minutes_generation' BEGIN SELECT RAISE(ABORT, 'simulated insert failure'); END;").unwrap();
    assert!(storage
        .enqueue_minutes_job(request.clone(), json!({}), json!({}), Some(1))
        .is_err());
    let db = storage.connection.lock().unwrap();
    for (table, expected) in [
        ("analysis_revisions", 1),
        ("speakers", 2),
        ("utterances", 2),
        ("speaker_segments", 2),
        ("utterance_speakers", 3),
        ("analysis_input_snapshots", 0),
        ("processing_jobs", 0),
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            expected,
            "{table}"
        );
    }
    drop(db);
    assert_eq!(
        storage
            .load_current_analysis(&request.meeting_id)
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        storage
            .load_transcript_draft(&request.meeting_id)
            .unwrap()
            .unwrap(),
        draft
    );
}

#[test]
fn internal_snapshots_are_sql_frozen_and_tampering_is_detected_on_read() {
    for target in ["content", "metadata"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let (request, _, _) = fixture(&storage);
        let job = enqueue(&storage, request.clone());
        let fixed = storage.load_minutes_input(&job.id).unwrap();
        let db = storage.connection.lock().unwrap();
        for sql in [
            "INSERT INTO speakers (id,analysis_revision_id,lineage_id,label,display_name,input_scope,identification_state,created_at) SELECT id || '-new',analysis_revision_id,lineage_id || '-new',label,display_name,input_scope,identification_state,created_at FROM speakers WHERE analysis_revision_id = ?1",
            "INSERT INTO speaker_segments (id,meeting_id,analysis_revision_id,track_id,speaker_id,start_us,end_us,overlap_group_id,confidence) SELECT id || '-new',meeting_id,analysis_revision_id,track_id,speaker_id,start_us,end_us,overlap_group_id,confidence FROM speaker_segments WHERE analysis_revision_id = ?1",
            "INSERT INTO utterances (id,meeting_id,analysis_revision_id,track_id,lineage_id,sequence,start_us,end_us,recognized_text,effective_text,attribution_state,confidence) SELECT id || '-new',meeting_id,analysis_revision_id,track_id,lineage_id || '-new',sequence + 10,start_us,end_us,recognized_text,effective_text,attribution_state,confidence FROM utterances WHERE analysis_revision_id = ?1",
            "INSERT INTO utterance_speakers (utterance_id,speaker_id,analysis_revision_id,role,assignment_source) SELECT u.id,s.id,u.analysis_revision_id,'overlap','manual' FROM utterances u JOIN speakers s ON s.analysis_revision_id = u.analysis_revision_id WHERE u.analysis_revision_id = ?1 AND NOT EXISTS(SELECT 1 FROM utterance_speakers a WHERE a.utterance_id = u.id AND a.speaker_id = s.id)",
        ] {
            let error = db.execute(sql,params![fixed.analysis.id]).unwrap_err();
            assert!(error.to_string().contains("internal analysis content is immutable"));
        }
        for table in [
            "speakers",
            "speaker_segments",
            "utterances",
            "utterance_speakers",
        ] {
            assert!(db.execute(&format!("UPDATE {table} SET analysis_revision_id = analysis_revision_id WHERE analysis_revision_id = ?1"),params![fixed.analysis.id]).is_err());
            assert!(db
                .execute(
                    &format!("DELETE FROM {table} WHERE analysis_revision_id = ?1"),
                    params![fixed.analysis.id]
                )
                .is_err());
        }
        assert!(db
            .execute(
                "UPDATE analysis_revisions SET content_hash = content_hash WHERE id = ?1",
                params![fixed.analysis.id]
            )
            .is_err());
        assert!(db
            .execute(
                "DELETE FROM analysis_revisions WHERE id = ?1",
                params![fixed.analysis.id]
            )
            .is_err());
        assert!(db.execute("UPDATE analysis_input_snapshots SET working_content_hash = working_content_hash WHERE analysis_id = ?1",params![fixed.analysis.id]).is_err());
        assert!(db
            .execute(
                "UPDATE meetings SET current_analysis_id = ?1 WHERE id = ?2",
                params![fixed.analysis.id, request.meeting_id]
            )
            .is_err());
        if target == "content" {
            db.execute_batch("DROP TRIGGER utterances_internal_update;")
                .unwrap();
            db.execute(
                "UPDATE utterances SET effective_text = '改変' WHERE analysis_revision_id = ?1",
                params![fixed.analysis.id],
            )
            .unwrap();
        } else {
            db.execute_batch("DROP TRIGGER input_snapshot_fixed_update;")
                .unwrap();
            db.execute(
                "UPDATE analysis_input_snapshots SET meeting_title = '改変' WHERE analysis_id = ?1",
                params![fixed.analysis.id],
            )
            .unwrap();
        }
        drop(db);
        assert!(storage.load_minutes_input(&job.id).is_err(), "{target}");
        assert!(storage
            .enqueue_minutes_job(
                request,
                json!({"engine":"llama.cpp","modelHash":"c".repeat(64)}),
                json!({"templateVersion":"standard-v1"}),
                Some(1)
            )
            .is_err());
    }
}

#[test]
fn snapshot_revalidates_draft_ids_even_when_modified_json_has_matching_hash() {
    for kind in ["speaker", "utterance", "schema"] {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let (request, original, draft) = fixture(&storage);
        let mut payload = draft.payload.clone();
        match kind {
            "speaker" => payload.utterances[0].speaker_ids = vec!["foreign-speaker".into()],
            "utterance" => payload.utterances[0].utterance_id = "foreign-utterance".into(),
            "schema" => payload.schema_version = 99,
            _ => unreachable!(),
        }
        let serialized = serde_json::to_string(&payload).unwrap();
        let mut hash = Sha256::new();
        hash.update(b"mon-transcript-draft-v1");
        hash.update(serialized.as_bytes());
        storage
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE working_drafts SET payload_json = ?1,content_hash = ?2 WHERE id = ?3",
                params![serialized, format!("{:x}", hash.finalize()), draft.id],
            )
            .unwrap();
        assert!(
            storage
                .enqueue_minutes_job(request, json!({}), json!({}), Some(1))
                .is_err(),
            "{kind}"
        );
        assert_eq!(
            storage
                .load_current_analysis(&original.meeting_id)
                .unwrap()
                .unwrap(),
            original
        );
        let db = storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM analysis_revisions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM processing_jobs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn minutes_evidence_can_reference_hidden_fixed_input_after_source_reanalysis() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().to_owned()).unwrap();
    let (request, original, _) = fixture(&storage);
    let job = enqueue(&storage, request.clone());
    let fixed = storage.load_minutes_input(&job.id).unwrap();
    let evidence = &fixed.analysis.utterances[0];
    let db = storage.connection.lock().unwrap();
    db.execute("INSERT INTO minutes_revisions (id,meeting_id,input_analysis_id,revision_no,publication_state,generated_document_json,effective_document_json,document_schema_version,input_content_hash,content_hash,model_snapshot_json,template_version,created_at) VALUES ('minutes',?1,?2,1,'candidate','{}','{}',1,?3,?4,'{}','standard-v1',0)",params![request.meeting_id,fixed.analysis.id,fixed.analysis.content_hash,"0".repeat(64)]).unwrap();
    db.execute("INSERT INTO minutes_items (id,minutes_revision_id,analysis_revision_id,kind,order_no,body,source_type) VALUES ('item','minutes',?1,'decision',0,'点検日を確定','engine')",params![fixed.analysis.id]).unwrap();
    assert!(db.execute("INSERT INTO evidence_links (id,minutes_revision_id,minutes_item_id,analysis_revision_id,utterance_id,quote_text,start_us,end_us,source_type,verification_state) VALUES ('wrong','minutes','item',?1,?2,'原版',?3,?4,'engine','unverified')",params![fixed.analysis.id,original.utterances[0].id,evidence.start_us,evidence.end_us]).is_err());
    db.execute("INSERT INTO evidence_links (id,minutes_revision_id,minutes_item_id,analysis_revision_id,utterance_id,quote_text,start_us,end_us,source_type,verification_state) VALUES ('evidence','minutes','item',?1,?2,?3,?4,?5,'engine','unverified')",params![fixed.analysis.id,evidence.id,evidence.effective_text,evidence.start_us,evidence.end_us]).unwrap();
    drop(db);
    let lock = storage
        .get_meeting(&request.meeting_id)
        .unwrap()
        .unwrap()
        .lock_version;
    storage
        .seed_stored_minutes_head(&request.meeting_id, "minutes")
        .unwrap();
    assert!(storage.verify_minutes_content_hash("minutes").unwrap());
    let mut next_input = input();
    next_input.utterances[0].text = "再解析の別内容".into();
    let next = storage
        .create_analysis_candidate(&request.meeting_id, next_input)
        .unwrap();
    let versions = storage.analysis_versions(&request.meeting_id).unwrap();
    storage
        .adopt_analysis(super::super::analysis_ui::AdoptRequest {
            meeting_id: request.meeting_id.clone(),
            revision_id: next.clone(),
            expected_content_hash: versions
                .versions
                .iter()
                .find(|v| v.id == next)
                .unwrap()
                .content_hash
                .clone(),
            expected_meeting_lock_version: versions.meeting_lock_version,
            expected_draft: versions.draft,
            confirm_replacing_previous: false,
        })
        .unwrap();
    assert_eq!(storage.load_minutes_input(&job.id).unwrap(), fixed);
    assert!(storage.minutes_input_updated(&job.id).unwrap());
    assert!(storage.verify_minutes_content_hash("minutes").unwrap());
}
