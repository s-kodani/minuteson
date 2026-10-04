use super::super::archive::VerifiedArchive;
use super::*;

fn fixture(storage: &Storage, operation: &str) -> String {
    let meeting = super::super::restore_ids::tests::fixture(storage, operation);
    let global = storage.vocabulary_book(None).unwrap();
    storage
        .save_vocabulary_book(
            None,
            global.lock_version,
            vec![VocabularyEntry {
                id: Uuid::new_v4().to_string(),
                term: "設計審査".into(),
                reading: Some("せっけいしんさ".into()),
                note: Some(format!("自然文の元会議ID {meeting}")),
                enabled: true,
            }],
        )
        .unwrap();
    let audio = storage.recorded_audio_plan(&meeting).unwrap();
    let transcription = storage.enqueue_with_vocabulary(&meeting, &format!("{operation}-transcription"), "固有名詞は文脈で確認", None, |vocabulary| {
        Ok(JobSpec {
            meeting_id:meeting.clone(),kind:JobKind::Transcription,operation_id:format!("{operation}-transcription"),input_analysis_id:None,input_hash:audio.manifest_hash.clone(),
            profile_snapshot:json!({"profile":"high_accuracy","model":{"path":"/Users/source-machine/models/model.bin","size":7,"sha256":"b".repeat(64)}}),
            parameters:json!({"schemaVersion":2,"source":audio,"engine":{"executable":{"path":"/Users/source-machine/bin/whisper","size":1,"sha256":"d".repeat(64)},"model":{"path":"/Users/source-machine/models/model.bin","size":7,"sha256":"b".repeat(64)},"vadModel":null,"threads":4,"useGpu":false,"prompt":vocabulary.actual_prompt},"units":[{"trackIndex":0,"coreFirst":0,"coreEnd":64000,"windowFirst":0,"windowEnd":64000}],"vocabulary":vocabulary}),total_units:Some(1),
        })
    }).unwrap();
    let cancelled = storage
        .finish_job(&transcription.id, 0, "cancelled", None)
        .unwrap();
    let (spec, checkpoint) = storage.fixed_job_input(&cancelled.id).unwrap();
    let retry = storage
        .enqueue_retry_job(
            &cancelled,
            &spec,
            checkpoint.as_ref(),
            &format!("{operation}-retry"),
        )
        .unwrap();
    storage.finish_job(&retry.id, 0, "cancelled", None).unwrap();
    let analysis = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let diarization=storage.enqueue_diarization_job(MinutesJobRequest {
        meeting_id:meeting.clone(),operation_id:format!("{operation}-diarization"),expected_analysis_id:analysis.id,expected_content_hash:analysis.content_hash,
        expected_meeting_lock_version:storage.get_meeting(&meeting).unwrap().unwrap().lock_version,expected_draft:Some(DraftStamp{id:draft.id,lock_version:draft.lock_version}),
    },json!({"profile":"high_accuracy"}),|input|Ok((json!({"source":audio,"engine":{"executable":{"path":"/Users/source-machine/bin/sherpa","size":1,"sha256":"d".repeat(64)},"segmentationModel":{"path":"/Users/source-machine/models/segmentation.onnx","size":9,"sha256":"c".repeat(64)},"embeddingModel":{"path":"/Users/source-machine/models/speaker.onnx","size":9,"sha256":"c".repeat(64)},"runtimeLibraries":[],"threads":4,"clusterThresholdMilli":500,"windowShiftMilli":500,"minDurationOnMs":200,"minDurationOffMs":100},"tracks":[{"trackId":audio.tracks[0].id,"numSpeakers":3}],"manualAssignmentHash":input.manual_assignment_hash}),1))).unwrap();
    storage
        .finish_job(&diarization.id, 0, "cancelled", None)
        .unwrap();
    // Fixed copies and import options must survive deletion of current terms.
    for scope in [None, Some(meeting.as_str())] {
        let current = storage.vocabulary_book(scope).unwrap();
        storage
            .save_vocabulary_book(scope, current.lock_version, vec![])
            .unwrap();
    }
    meeting
}

fn read(storage: &Storage, output: &Path) -> VerifiedArchive {
    let path = output.join("fixed.monbackup");
    storage.create_backup_archive(&path, &|| false).unwrap();
    archive::read(&mut File::open(path).unwrap(), output, &|| false).unwrap()
}

fn strings(db: &Connection, sql: &str) -> Vec<String> {
    db.prepare(sql)
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

// A test-only projection into a trusted, isolated schema. No triggers are ever
// removed from the app DB. The actual additive publication path remains gated.
fn project(fixed: &FixedData<'_, '_>) -> Connection {
    let db = portable::schema(MIGRATIONS.len()).unwrap();
    for trigger in strings(&db, "SELECT name FROM sqlite_schema WHERE type='trigger'") {
        db.execute_batch(&format!("DROP TRIGGER \"{trigger}\";"))
            .unwrap();
    }
    db.execute_batch("BEGIN; PRAGMA defer_foreign_keys=ON;")
        .unwrap();
    for table in portable::tables(&fixed.ids.source().database).unwrap() {
        let rowids: Vec<i64> = fixed
            .ids
            .source()
            .database
            .prepare(&format!("SELECT rowid FROM {} ORDER BY rowid", table.name))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let columns = table
            .columns
            .iter()
            .map(|c| format!("\"{}\"", c.name))
            .collect::<Vec<_>>()
            .join(",");
        let placeholders = vec!["?"; table.columns.len()].join(",");
        for rowid in rowids {
            let row = fixed.typed_row(&table, rowid).unwrap();
            db.execute(
                &format!(
                    "INSERT INTO {}({columns}) VALUES({placeholders})",
                    table.name
                ),
                rusqlite::params_from_iter(row),
            )
            .unwrap();
        }
    }
    db.execute_batch("COMMIT;").unwrap();
    db
}

fn register_assets(fixed: &FixedData<'_, '_>, target: &Connection) {
    let source = fixed.ids.source();
    for (id, path) in &source.assets {
        let meeting: String = source
            .database
            .query_row(
                "SELECT meeting_id FROM file_assets WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        let is_manifest: bool = source
            .database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM audio_import_commits WHERE manifest_asset_id=?1)",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        let (size, hash) = if is_manifest {
            let value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let mapped = fixed.import_manifest(&meeting, &value).unwrap();
            fixed
                .asset_bytes(&meeting, id, &serde_json::to_vec(&mapped).unwrap())
                .unwrap()
        } else {
            fixed.asset_file(&meeting, id, path, &|| false).unwrap()
        };
        target
            .execute(
                "UPDATE file_assets SET size_bytes=?1,content_hash=?2 WHERE id=?3",
                params![
                    size as i64,
                    hash,
                    fixed.ids.get("file_assets", &meeting, id).unwrap()
                ],
            )
            .unwrap();
    }
    for meeting in strings(
        &source.database,
        "SELECT meeting_id FROM audio_import_commits",
    ) {
        let (asset,old):(String,String)=source.database.query_row("SELECT manifest_asset_id,manifest_hash FROM audio_import_commits WHERE meeting_id=?1",params![meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let hash = fixed
            .resolve("file_assets", &meeting, &asset, &old)
            .unwrap();
        target
            .execute(
                "UPDATE audio_import_commits SET manifest_hash=?1 WHERE meeting_id=?2",
                params![hash, fixed.ids.get("meetings", "", &meeting).unwrap()],
            )
            .unwrap();
    }
}

fn rebuild_analyses(fixed: &FixedData<'_, '_>, target: &mut Connection) {
    for id in strings(
        &fixed.ids.source().database,
        "SELECT id FROM analysis_revisions ORDER BY meeting_id,revision_no",
    ) {
        let (meeting,parent,parent_hash,audio):(String,Option<String>,Option<String>,String)=fixed.ids.source().database.query_row("SELECT meeting_id,parent_revision_id,parent_content_hash,audio_manifest_hash FROM analysis_revisions WHERE id=?1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        let new_id = fixed.ids.get("analysis_revisions", &meeting, &id).unwrap();
        let hash = parent.as_deref().map(|parent| {
            fixed
                .resolve(
                    "analysis_revisions",
                    &meeting,
                    parent,
                    parent_hash.as_deref().unwrap(),
                )
                .unwrap()
        });
        let (config, snapshot) = fixed.analysis_config(&meeting, &id, target).unwrap();
        let audio = fixed.audio_manifest_hash(&meeting, &audio).unwrap();
        let tx = target.transaction().unwrap();
        tx.execute("UPDATE analysis_revisions SET config_json=?1,parent_content_hash=?2,audio_manifest_hash=?3 WHERE id=?4",params![config.to_string(),hash,audio,new_id]).unwrap();
        if let Some(snapshot) = snapshot {
            tx.execute("UPDATE analysis_input_snapshots SET source_content_hash=?1,source_draft_json=?2,working_content_hash=?3 WHERE analysis_id=?4",params![snapshot.source_content_hash,snapshot.source_draft.as_ref().map(|s|serde_json::to_string(s).unwrap()),snapshot.working_content_hash,new_id]).unwrap();
        }
        let hash = fixed
            .remember_revision(&tx, "analysis_revisions", &meeting, &id)
            .unwrap();
        tx.execute(
            "UPDATE analysis_revisions SET content_hash=?1 WHERE id=?2",
            params![hash, new_id],
        )
        .unwrap();
        tx.commit().unwrap();
    }
}

fn rebuild_jobs(fixed: &FixedData<'_, '_>, target: &Connection) {
    for id in strings(
        &fixed.ids.source().database,
        "SELECT id FROM processing_jobs ORDER BY attempt,id",
    ) {
        let (meeting, retry, attempt): (String, Option<String>, i64) = fixed
            .ids
            .source()
            .database
            .query_row(
                "SELECT meeting_id,retry_of_job_id,attempt FROM processing_jobs WHERE id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let spec = fixed.job_spec(&meeting, &id).unwrap();
        let new_id = fixed.ids.get("processing_jobs", &meeting, &id).unwrap();
        let retry = retry
            .as_deref()
            .map(|r| fixed.ids.get("processing_jobs", &meeting, r).unwrap());
        let hash = fixed
            .job_hash(&meeting, &id, &spec, retry.as_deref(), attempt)
            .unwrap();
        target.execute("UPDATE processing_jobs SET input_hash=?1,profile_snapshot_json=?2,parameters_json=?3,spec_hash=?4 WHERE id=?5",params![spec.input_hash,spec.profile_snapshot.to_string(),spec.parameters.to_string(),hash,new_id]).unwrap();
        // This fixture has a generic {fixture:true} checkpoint, not native
        // artifacts. Native checkpoint/artifact reconstruction is a separate gate.
        let checkpoints: Vec<(String, String, i64)> = target
            .prepare(
                "SELECT id,checkpoint_json,completed_units FROM job_checkpoints WHERE job_id=?1",
            )
            .unwrap()
            .query_map(params![new_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (checkpoint, json, units) in checkpoints {
            assert_eq!(
                serde_json::from_str::<Value>(&json).unwrap(),
                json!({"fixture":true})
            );
            target
                .execute(
                    "UPDATE job_checkpoints SET checkpoint_hash=?1 WHERE id=?2",
                    params![
                        super::super::super::jobs::checkpoint_hash(&hash, units, &json),
                        checkpoint
                    ],
                )
                .unwrap();
        }
        let (actual, _) = super::super::super::jobs::read_fixed_input(target, &new_id).unwrap();
        assert_eq!(actual, spec);
    }
}

fn rebuild_minutes(fixed: &FixedData<'_, '_>, target: &mut Connection) {
    for id in strings(
        &fixed.ids.source().database,
        "SELECT id FROM minutes_revisions ORDER BY meeting_id,revision_no",
    ) {
        let (meeting,input,input_hash,parent,parent_hash):(String,String,String,Option<String>,Option<String>)=fixed.ids.source().database.query_row("SELECT meeting_id,input_analysis_id,input_content_hash,parent_revision_id,parent_content_hash FROM minutes_revisions WHERE id=?1",params![id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        let new_id = fixed.ids.get("minutes_revisions", &meeting, &id).unwrap();
        let input_hash = fixed
            .resolve("analysis_revisions", &meeting, &input, &input_hash)
            .unwrap();
        let parent_hash = parent.as_deref().map(|p| {
            fixed
                .resolve(
                    "minutes_revisions",
                    &meeting,
                    p,
                    parent_hash.as_deref().unwrap(),
                )
                .unwrap()
        });
        let tx = target.transaction().unwrap();
        tx.execute(
            "UPDATE minutes_revisions SET input_content_hash=?1,parent_content_hash=?2 WHERE id=?3",
            params![input_hash, parent_hash, new_id],
        )
        .unwrap();
        let hash = fixed
            .remember_revision(&tx, "minutes_revisions", &meeting, &id)
            .unwrap();
        tx.execute(
            "UPDATE minutes_revisions SET content_hash=?1 WHERE id=?2",
            params![hash, new_id],
        )
        .unwrap();
        tx.commit().unwrap();
    }
}

#[test]
fn typed_fixed_data_rebuilds_native_inputs_and_hashes_without_changing_audio_or_text() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "fixed");
    let original_draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let minutes_id = storage
        .minutes_versions(&meeting)
        .unwrap()
        .current_id
        .unwrap();
    let original_minutes = storage
        .load_minutes_content(&meeting, &minutes_id)
        .unwrap()
        .unwrap();
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let mut target = project(&fixed);
    register_assets(&fixed, &target);
    for scope in strings(&source.database, "SELECT id FROM vocabulary_scopes") {
        let hash = fixed.vocabulary_book_hash(&target, &scope).unwrap();
        let new_scope = if scope == "global" {
            scope
        } else {
            format!("meeting:{}", ids.get("meetings", "", &meeting).unwrap())
        };
        target
            .execute(
                "UPDATE vocabulary_scopes SET content_hash=?1 WHERE id=?2",
                params![hash, new_scope],
            )
            .unwrap();
    }
    rebuild_analyses(&fixed, &mut target);
    rebuild_jobs(&fixed, &target);
    rebuild_minutes(&fixed, &mut target);
    super::super::validate_database(&mut target, &|| false).unwrap();
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    let tx = target.transaction().unwrap();
    let new_minutes = super::super::super::minutes::read_content(
        &tx,
        &new_meeting,
        &ids.get("minutes_revisions", &meeting, &original_minutes.id)
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_ne!(new_minutes.content_hash, original_minutes.content_hash);
    assert_eq!(
        new_minutes.content.items[0].body,
        original_minutes.content.items[0].body
    );
    assert_eq!(
        new_minutes.content.evidence[0].quote_text,
        original_minutes.content.evidence[0].quote_text
    );
    assert_eq!(
        new_minutes.content.evidence[0].start_us,
        original_minutes.content.evidence[0].start_us
    );
    assert_eq!(
        new_minutes.content.evidence[0].end_us,
        original_minutes.content.evidence[0].end_us
    );
    for id in strings(
        &tx,
        "SELECT id FROM processing_jobs WHERE kind='diarization'",
    ) {
        let (spec, _) = super::super::super::jobs::read_fixed_input(&tx, &id).unwrap();
        let input = super::super::super::diarization::read_input(&tx, &spec).unwrap();
        assert_eq!(input.manual_utterances.len(), 2);
        assert_eq!(
            spec.parameters["diarization"]["tracks"][0]["numSpeakers"],
            3
        );
        assert_eq!(
            spec.parameters["diarization"]["manualAssignmentHash"],
            input.manual_assignment_hash
        );
    }
    for id in strings(
        &tx,
        "SELECT id FROM processing_jobs WHERE kind='minutes_generation'",
    ) {
        let (spec, _) = super::super::super::jobs::read_fixed_input(&tx, &id).unwrap();
        super::super::super::minutes_input::read_input(&tx, &spec).unwrap();
    }
    for id in strings(
        &tx,
        "SELECT id FROM processing_jobs WHERE kind='transcription'",
    ) {
        let (spec, _) = super::super::super::jobs::read_fixed_input(&tx, &id).unwrap();
        let vocabulary: VocabularySnapshot =
            serde_json::from_value(spec.parameters["vocabulary"].clone()).unwrap();
        vocabulary.validate().unwrap();
        assert!(vocabulary.entries.iter().any(|e| e.entry.term == "点検日"));
        assert!(vocabulary.entries.iter().any(|e| e
            .entry
            .note
            .as_ref()
            .is_some_and(|n| n.contains(&meeting))));
        assert!(spec.profile_snapshot["model"]["path"]
            .as_str()
            .unwrap()
            .starts_with("mon-unavailable:"));
        let audio: RecordedAudioPlan =
            serde_json::from_value(spec.parameters["source"].clone()).unwrap();
        assert_eq!(
            audio,
            super::super::super::transcription::read_audio_plan(&tx, &new_meeting).unwrap()
        );
    }
    tx.commit().unwrap();
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
        original_draft
    );
    assert_eq!(
        storage
            .load_minutes_content(&meeting, &minutes_id)
            .unwrap()
            .unwrap()
            .content_hash,
        original_minutes.content_hash
    );
    assert!(source
        .manifest
        .file_references
        .iter()
        .any(|f| f.name == "model.bin"));
    assert_eq!(
        strings(&target, "SELECT id FROM meetings"),
        vec![new_meeting]
    );
}

#[test]
fn fixed_audio_rejects_changed_time_track_owner_and_waveform_before_registration() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "audio");
    let other = archive::tests::meeting(&storage, "other");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let target = project(&fixed);
    register_assets(&fixed, &target);
    let original = serde_json::to_value(storage.recorded_audio_plan(&meeting).unwrap()).unwrap();
    for pointer in [
        "/tracks/0/startUs",
        "/tracks/0/endUs",
        "/tracks/0/chunks/0/startUs",
        "/tracks/0/chunks/0/endUs",
        "/tracks/0/chunks/0/frames",
        "/tracks/0/chunks/0/sampleRate",
        "/tracks/0/chunks/0/channels",
    ] {
        let mut bad = original.clone();
        let value = bad.pointer_mut(pointer).unwrap();
        *value = json!(value.as_i64().unwrap() + 1);
        assert!(
            fixed
                .audio_plan(&meeting, &bad)
                .unwrap_err()
                .contains("元版"),
            "{pointer}"
        );
    }
    assert!(fixed.audio_plan(&other, &original).is_err());
    let asset = original["tracks"][0]["chunks"][0]["assetId"]
        .as_str()
        .unwrap();
    assert!(fixed
        .asset_bytes(&meeting, asset, b"changed waveform")
        .unwrap_err()
        .contains("音声本体"));
    assert!(fixed
        .asset_file(&other, asset, &source.assets[asset], &|| false)
        .is_err());
    assert!(fixed
        .asset_file(&meeting, asset, &source.assets[asset], &|| true)
        .is_err());
    assert!(fixed.audio_plan(&meeting, &original).is_ok());
    let manifest_id = original["manifestAssetId"].as_str().unwrap();
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&source.assets[manifest_id]).unwrap()).unwrap();
    manifest["originalFileName"] = json!("wrong.wav");
    assert!(fixed
        .import_manifest(&meeting, &manifest)
        .unwrap_err()
        .contains("音声由来"));
    let actual = storage.recorded_audio_plan(&meeting).unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), original);
}

#[test]
fn fixed_vocabulary_rejects_hash_forgery_and_retains_deleted_global_and_meeting_terms() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "vocabulary");
    let source = read(&storage, output.path());
    assert_eq!(
        source
            .database
            .query_row("SELECT COUNT(*) FROM vocabulary_entries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let text: String = source
        .database
        .query_row("SELECT payload_json FROM vocabulary_snapshots", [], |r| {
            r.get(0)
        })
        .unwrap();
    let original: Value = serde_json::from_str(&text).unwrap();
    let new: VocabularySnapshot =
        serde_json::from_value(fixed.vocabulary(&meeting, &original).unwrap()).unwrap();
    let old: VocabularySnapshot = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(new.actual_prompt, old.actual_prompt);
    assert_eq!(new.stamp, old.stamp);
    assert_ne!(new.content_hash, old.content_hash);
    assert_ne!(new.id, old.id);
    for (old, new) in old.entries.iter().zip(&new.entries) {
        assert_ne!(new.entry.id, old.entry.id);
        assert_eq!(new.scope, old.scope);
        assert_eq!(new.entry.term, old.entry.term);
        assert_eq!(new.entry.note, old.entry.note);
        assert_eq!(new.entry.reading, old.entry.reading);
        assert_eq!(new.entry_version, old.entry_version);
    }
    let mut bad = old.clone();
    bad.entries[0].entry.note = Some("forged note".into());
    assert!(fixed
        .vocabulary(&meeting, &serde_json::to_value(&bad).unwrap())
        .is_err());
    bad.actual_prompt =
        super::super::super::vocabulary::compile(&bad.entries, &bad.extra_prompt).unwrap();
    bad.content_hash = bad.digest().unwrap();
    bad.validate().unwrap();
    assert!(fixed
        .vocabulary(&meeting, &serde_json::to_value(&bad).unwrap())
        .is_err());
    assert!(fixed.vocabulary("other-meeting", &original).is_err());
    assert!(fixed
        .resolve("vocabulary_snapshots", &meeting, &old.id, &"0".repeat(64))
        .is_err());
}

#[test]
fn assignment_and_retry_hashes_reject_wrong_inputs_and_preserve_fixed_conditions() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "jobs");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let mut target = project(&fixed);
    register_assets(&fixed, &target);
    rebuild_analyses(&fixed, &mut target);
    let id: String = source
        .database
        .query_row(
            "SELECT id FROM processing_jobs WHERE kind='diarization'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (old_input, parameters): (String, String) = source
        .database
        .query_row(
            "SELECT input_analysis_id,parameters_json FROM processing_jobs WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let original: Value = serde_json::from_str(&parameters).unwrap();
    let good = fixed
        .parameters(&meeting, Some(&old_input), &original)
        .unwrap();
    assert_ne!(
        good["diarization"]["manualAssignmentHash"],
        original["diarization"]["manualAssignmentHash"]
    );
    assert_eq!(
        good["diarization"]["engine"],
        original["diarization"]["engine"]
    );
    let mut bad = original.clone();
    bad["diarization"]["manualAssignmentHash"] = json!("0".repeat(64));
    assert!(fixed
        .parameters(&meeting, Some(&old_input), &bad)
        .unwrap_err()
        .contains("固定割り当て"));
    let parent: String = source
        .database
        .query_row(
            "SELECT source_analysis_id FROM analysis_input_snapshots WHERE analysis_id=?1",
            params![old_input],
            |r| r.get(0),
        )
        .unwrap();
    assert!(fixed
        .parameters(&meeting, Some(&parent), &original)
        .is_err());
    assert!(fixed.parameters(&meeting, None, &original).is_err());
    let retry: String = source
        .database
        .query_row("SELECT id FROM processing_jobs WHERE attempt=2", [], |r| {
            r.get(0)
        })
        .unwrap();
    let old_parent: String = source
        .database
        .query_row(
            "SELECT retry_of_job_id FROM processing_jobs WHERE id=?1",
            params![retry],
            |r| r.get(0),
        )
        .unwrap();
    let parent = ids.get("processing_jobs", &meeting, &old_parent).unwrap();
    let spec = fixed.job_spec(&meeting, &retry).unwrap();
    let hash = fixed
        .job_hash(&meeting, &retry, &spec, Some(&parent), 2)
        .unwrap();
    assert_eq!(hash, fixed_spec_hash(&spec, Some(&parent), 2).unwrap());
    assert!(fixed
        .job_hash(&meeting, &retry, &spec, Some(&old_parent), 2)
        .is_err());
    assert!(fixed.job_hash(&meeting, &retry, &spec, None, 1).is_err());
    let mut changed = spec.clone();
    changed.parameters["units"][0]["coreEnd"] = json!(1);
    assert!(fixed
        .job_hash(&meeting, &retry, &changed, Some(&parent), 2)
        .is_err());
}

#[test]
fn hash_registry_is_private_cancellable_and_resolves_exact_historical_draft_hashes() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "history");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let before: usize = std::fs::read_dir(output.path()).unwrap().count();
    assert!(FixedData::new(&ids, output.path(), &|| true).is_err());
    let calls = std::cell::Cell::new(0);
    assert!(FixedData::new(&ids, output.path(), &|| {
        calls.set(calls.get() + 1);
        calls.get() > 2
    })
    .is_err());
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let (draft,kind,text,hash):(String,String,String,String)=source.database.query_row("SELECT d.id,d.target_kind,e.inverse_json,e.before_hash FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id WHERE d.target_kind='minutes' ORDER BY e.sequence LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let (json, new) = ids.draft_json(&kind, &meeting, &text).unwrap();
    assert_eq!(
        fixed
            .resolve("working_drafts", &meeting, &draft, &hash)
            .unwrap(),
        new
    );
    assert_ne!(json, text);
    assert_ne!(new, hash);
    assert!(fixed
        .resolve("working_drafts", &meeting, &draft, &"a".repeat(64))
        .is_err());
    assert!(fixed
        .remember("working_drafts", &meeting, &draft, &hash, &"f".repeat(64))
        .is_err());
    assert_eq!(
        fixed
            .resolve("working_drafts", &meeting, &draft, &hash)
            .unwrap(),
        new
    );
    let stage = fixed.stage.path().to_owned();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&stage).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(stage.join("fixed-hashes.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(fixed);
    assert!(!stage.exists());
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
    assert!(source.assets.values().all(|path| path.exists()));
    assert!(storage.create_meeting("元DBへの書き込みは継続").is_ok());
}

#[test]
fn cross_domain_hash_order_places_input_before_job_and_job_before_output() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "order");
    let source = read(&storage, output.path());
    let (job, input, hash): (String, String, String) = source
        .database
        .query_row(
            "SELECT id,input_analysis_id,spec_hash FROM processing_jobs WHERE kind='diarization'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let result = Uuid::new_v4().to_string();
    source
        .database
        .execute_batch("PRAGMA query_only=OFF;")
        .unwrap();
    source.database.execute("INSERT INTO analysis_revisions(id,meeting_id,parent_revision_id,revision_no,parent_content_hash,origin,publication_state,audio_manifest_hash,profile_snapshot_json,config_json,content_hash,schema_version,created_at) SELECT ?1,meeting_id,id,revision_no+1,content_hash,'engine','candidate',audio_manifest_hash,profile_snapshot_json,?2,content_hash,schema_version,created_at FROM analysis_revisions WHERE id=?3",params![result,json!({"jobId":job,"specHash":hash,"inputAnalysisId":input,"inputContentHash":"a".repeat(64)}).to_string(),input]).unwrap();
    source
        .database
        .execute_batch("PRAGMA query_only=ON;")
        .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let position = |domain: &str, id: &str| {
        fixed
            .registry
            .query_row(
                "SELECT position FROM hash_order WHERE domain=?1 AND meeting=?2 AND old_id=?3",
                params![domain, meeting, id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert!(position("analysis_revisions", &input) < position("processing_jobs", &job));
    assert!(position("processing_jobs", &job) < position("analysis_revisions", &result));
    let violations:i64=fixed.registry.query_row("SELECT COUNT(*) FROM hash_edges e JOIN hash_order c ON c.domain=e.child_domain AND c.meeting=e.meeting AND c.old_id=e.child_id JOIN hash_order p ON p.domain=e.parent_domain AND p.meeting=e.meeting AND p.old_id=e.parent_id WHERE c.position<=p.position",[],|r|r.get(0)).unwrap();
    assert_eq!(violations, 0);
    let count: i64 = fixed
        .registry
        .query_row("SELECT COUNT(*) FROM hash_nodes", [], |r| r.get(0))
        .unwrap();
    let mut steps = std::collections::BTreeSet::new();
    for position in 0..count {
        let step = fixed.ordered_step(position).unwrap().unwrap();
        assert!(steps.insert((step.domain, step.meeting, step.old_id)));
    }
    assert!(fixed.ordered_step(count).unwrap().is_none());
    let live_config:String=storage.connection.lock().unwrap().query_row("SELECT config_json FROM analysis_revisions WHERE id=(SELECT current_analysis_id FROM meetings WHERE id=?1)",params![meeting],|r|r.get(0)).unwrap();
    assert!(serde_json::from_str::<Value>(&live_config)
        .unwrap()
        .get("jobId")
        .is_none());
}

#[test]
fn cross_domain_cycles_and_foreign_config_job_references_are_rejected_with_private_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "cycle");
    let other = fixture(&storage, "other-cycle");
    let source = read(&storage, output.path());
    let base = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let text: String = source
        .database
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            params![base.id],
            |r| r.get(0),
        )
        .unwrap();
    let original_config: Value = serde_json::from_str(&text).unwrap();
    let job: String = source
        .database
        .query_row(
            "SELECT id FROM processing_jobs WHERE meeting_id=?1 AND kind='diarization'",
            params![meeting],
            |r| r.get(0),
        )
        .unwrap();
    let foreign: String = source
        .database
        .query_row(
            "SELECT id FROM processing_jobs WHERE meeting_id=?1 AND kind='diarization'",
            params![other],
            |r| r.get(0),
        )
        .unwrap();
    for (reference, expected) in [
        (job, "循環"),
        (foreign, "未定義"),
        (Uuid::new_v4().to_string(), "未定義"),
    ] {
        source
            .database
            .execute_batch("PRAGMA query_only=OFF;")
            .unwrap();
        let mut config = original_config.clone();
        config["jobId"] = json!(reference);
        config["specHash"] = json!("b".repeat(64));
        source
            .database
            .execute(
                "UPDATE analysis_revisions SET config_json=?1 WHERE id=?2",
                params![config.to_string(), base.id],
            )
            .unwrap();
        source
            .database
            .execute_batch("PRAGMA query_only=ON;")
            .unwrap();
        let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
            false
        })
        .unwrap();
        let before = std::fs::read_dir(output.path()).unwrap().count();
        let error = FixedData::new(&ids, output.path(), &|| false)
            .err()
            .unwrap();
        assert!(error.contains(expected), "{error}");
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), before);
        assert_eq!(
            storage
                .load_current_analysis(&meeting)
                .unwrap()
                .unwrap()
                .content_hash,
            base.content_hash
        );
    }
}

#[test]
fn shared_job_spec_hash_keeps_the_original_domains_and_retry_shape() {
    let spec = JobSpec {
        meeting_id: "m".into(),
        kind: crate::storage::JobKind::Transcription,
        operation_id: "o".into(),
        input_analysis_id: None,
        input_hash: "a".repeat(64),
        profile_snapshot: json!({"profile":"standard"}),
        parameters: json!({"value":"natural text"}),
        total_units: Some(1),
    };
    let mut hash = Sha256::new();
    hash.update(b"mon-job-spec-v1");
    hash.update(serde_json::to_vec(&spec).unwrap());
    assert_eq!(
        fixed_spec_hash(&spec, None, 1).unwrap(),
        format!("{:x}", hash.finalize())
    );
    let mut hash = Sha256::new();
    hash.update(b"mon-job-retry-spec-v1");
    hash.update(serde_json::to_vec(&(&spec, "parent", 2_i64)).unwrap());
    assert_eq!(
        fixed_spec_hash(&spec, Some("parent"), 2).unwrap(),
        format!("{:x}", hash.finalize())
    );
    assert!(fixed_spec_hash(&spec, None, 2).is_err());
    assert!(fixed_spec_hash(&spec, Some("parent"), 1).is_err());
}
