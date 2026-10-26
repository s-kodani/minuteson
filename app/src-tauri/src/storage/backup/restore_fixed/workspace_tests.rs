use super::super::tests::{fixture, read};
use super::*;
use std::cell::Cell;

#[test]
fn corrected_clock_proof_survives_archive_id_rebinding_and_fixed_job_rebuild() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = super::super::tests::fixture_with_clock(&storage, "corrected-backup", true);
    let original = storage.recorded_audio_plan(&meeting).unwrap();
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let workspace =
        FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    let rebuilt = super::super::super::super::transcription::read_audio_plan(
        &workspace.database,
        &new_meeting,
    )
    .unwrap();
    assert_ne!(original.tracks[0].id, rebuilt.tracks[0].id);
    assert_ne!(
        original.tracks[0].chunks[0].asset_id,
        rebuilt.tracks[0].chunks[0].asset_id
    );
    assert_eq!(
        original.tracks[0].chunks[0].hash,
        rebuilt.tracks[0].chunks[0].hash
    );
    assert_eq!(
        original.tracks[0].chunks[0].clock_segments,
        rebuilt.tracks[0].chunks[0].clock_segments
    );
    assert_eq!(
        original.tracks[0].total_frames().unwrap(),
        rebuilt.tracks[0].total_frames().unwrap()
    );
    let parameters:Vec<String>=workspace.database.prepare("SELECT parameters_json FROM processing_jobs WHERE kind IN ('transcription','diarization')").unwrap().query_map([],|row|row.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(parameters.len(), 3);
    for json in parameters {
        let value: Value = serde_json::from_str(&json).unwrap();
        let plan = value
            .get("source")
            .unwrap_or(&value["diarization"]["source"]);
        let mixed = super::super::super::super::transcription::read_analysis_audio_plan(
            &workspace.database,
            &new_meeting,
            None,
        )
        .unwrap();
        assert_eq!(*plan, serde_json::to_value(&mixed).unwrap());
    }
    assert_eq!(storage.recorded_audio_plan(&meeting).unwrap(), original);
}

#[test]
fn file_backed_workspace_rebuilds_fixed_inputs_in_dependency_order_and_preserves_live_data() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "workspace");
    let original_draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let original_content = storage.get_minutes_markdown(&meeting).unwrap().unwrap();
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let mut workspace =
        FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let stage = workspace.stage.path().to_path_buf();
    let database = stage.join("fixed.sqlite3");
    assert!(database.is_file());
    assert_eq!(
        workspace
            .database
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        database.canonicalize().unwrap().to_str().unwrap()
    );
    let new_meeting = ids.get("meetings", "", &meeting).unwrap();
    let tx = workspace.database.transaction().unwrap();
    let rebuilt = crate::storage::minutes_markdown::read(&tx, &new_meeting)
        .unwrap()
        .unwrap();
    assert_eq!(rebuilt.markdown, original_content.markdown);
    assert_eq!(rebuilt.content_hash, original_content.content_hash);
    let original_audio = storage.recorded_audio_plan(&meeting).unwrap();
    let audio =
        super::super::super::super::transcription::read_audio_plan(&tx, &new_meeting).unwrap();
    assert_ne!(audio.manifest_asset_id, original_audio.manifest_asset_id);
    assert_ne!(audio.manifest_hash, original_audio.manifest_hash);
    assert_eq!(
        audio.tracks[0].chunks[0].hash,
        original_audio.tracks[0].chunks[0].hash
    );
    assert_eq!(
        audio.tracks[0].chunks[0].frames,
        original_audio.tracks[0].chunks[0].frames
    );
    let mut query = tx.prepare("SELECT id,input_hash,parameters_json,spec_hash,retry_of_job_id,attempt,input_analysis_id,profile_snapshot_json FROM processing_jobs WHERE kind IN ('transcription','diarization')").unwrap();
    let mut rows = query.query([]).unwrap();
    while let Some(row) = rows.next().unwrap() {
        let id: String = row.get(0).unwrap();
        let job = super::super::super::super::jobs::load_job(&tx, &id)
            .unwrap()
            .unwrap();
        let parameters: Value = serde_json::from_str(&row.get::<_, String>(2).unwrap()).unwrap();
        let spec = JobSpec {
            meeting_id: new_meeting.clone(),
            kind: serde_json::from_value(json!(job.kind)).unwrap(),
            operation_id: job.operation_id,
            input_analysis_id: row.get(6).unwrap(),
            input_hash: row.get(1).unwrap(),
            profile_snapshot: serde_json::from_str(&row.get::<_, String>(7).unwrap()).unwrap(),
            parameters,
            total_units: job.total_units,
        };
        let retry: Option<String> = row.get(4).unwrap();
        assert_eq!(
            row.get::<_, String>(3).unwrap(),
            fixed_spec_hash(&spec, retry.as_deref(), row.get(5).unwrap()).unwrap()
        );
        if spec.kind == JobKind::Diarization {
            super::super::super::super::diarization::read_input(&tx, &spec).unwrap();
        }
    }
    drop(rows);
    drop(query);
    tx.commit().unwrap();
    let assets: Vec<(String, i64, String)> = workspace
        .database
        .prepare(
            "SELECT relative_path,size_bytes,content_hash FROM file_assets WHERE state='ready'",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(assets.len(), 2);
    for (relative, size, hash) in assets {
        assert!(relative.starts_with(&format!("meetings/{new_meeting}/restored/")));
        assert!(!relative.contains(&meeting));
        assert_eq!(
            digest_file(&stage.join(relative), &crate::test_support::never_cancelled).unwrap(),
            (size as u64, hash)
        );
    }
    assert!(workspace
        .database
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap()
        .is_none());
    assert!(!workspace.triggers.is_empty());
    assert_eq!(
        workspace
            .database
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    // This current human-Markdown fixture has no pending worker checkpoint.
    super::super::super::validate_database(
        &mut workspace.database,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap().unwrap(),
        original_draft
    );
    assert_eq!(
        storage
            .get_minutes_markdown(&meeting)
            .unwrap()
            .unwrap()
            .content_hash,
        original_content.content_hash
    );
    drop(workspace);
    assert!(!stage.exists());
    assert!(source.assets.values().all(|p| p.is_file()));
}

#[test]
fn workspace_rebuilds_an_output_analysis_only_after_its_fixed_input_and_job() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "workspace-job-output");
    let job: String = storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT id FROM processing_jobs WHERE meeting_id=?1 AND kind='diarization'",
            params![meeting],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot = storage.get_job(&job).unwrap();
    let (spec, _) = storage.fixed_job_input(&job).unwrap();
    let audio = storage.recorded_audio_plan(&meeting).unwrap();
    let result=storage.create_analysis_candidate(&meeting,AnalysisInput {
        audio_manifest_hash:audio.manifest_hash.clone(),profile_snapshot:json!({"engine":"synthetic-output"}),
        config:json!({"jobId":job,"specHash":snapshot.spec_hash,"inputAnalysisId":spec.input_analysis_id,"inputContentHash":spec.input_hash}),
        speakers:vec![],segments:vec![],utterances:vec![UtteranceInput{track_id:audio.tracks[0].id.clone(),start_us:0,end_us:1_000_000,text:"合成の後続解析結果".into(),speaker_keys:vec![],confidence:None}],
    }).unwrap();
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let workspace =
        FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let new_job = ids.get("processing_jobs", &meeting, &job).unwrap();
    let new_result = ids.get("analysis_revisions", &meeting, &result).unwrap();
    let (config, hash): (String, String) = workspace
        .database
        .query_row(
            "SELECT config_json,content_hash FROM analysis_revisions WHERE id=?1",
            params![new_result],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let config: Value = serde_json::from_str(&config).unwrap();
    assert_eq!(config["jobId"], new_job);
    assert_eq!(
        config["inputAnalysisId"],
        ids.get(
            "analysis_revisions",
            &meeting,
            spec.input_analysis_id.as_deref().unwrap()
        )
        .unwrap()
    );
    assert_ne!(config["inputContentHash"], spec.input_hash);
    assert_ne!(config["specHash"], snapshot.spec_hash);
    assert_eq!(
        config["specHash"],
        workspace
            .database
            .query_row(
                "SELECT spec_hash FROM processing_jobs WHERE id=?1",
                params![new_job],
                |r| r.get::<_, String>(0)
            )
            .unwrap()
    );
    let mut db = read_only(&workspace.stage.path().join("fixed.sqlite3")).unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        hash,
        super::super::super::super::revisions::analysis_content_hash(&tx, &new_result).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn a_substituted_archive_audio_symlink_is_rejected_without_removing_its_target() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "workspace-symlink");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let audio = storage.recorded_audio_plan(&meeting).unwrap();
    let staged = &source.assets[&audio.tracks[0].chunks[0].asset_id];
    let substitute = output.path().join("substitute-synthetic.flac");
    std::fs::copy(staged, &substitute).unwrap();
    std::fs::remove_file(staged).unwrap();
    std::os::unix::fs::symlink(&substitute, staged).unwrap();
    assert!(
        FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .is_err()
    );
    assert!(substitute.is_file());
    assert!(std::fs::symlink_metadata(staged)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(storage.recorded_audio_plan(&meeting).is_ok());
}

#[test]
fn cancellation_and_changed_verified_audio_remove_only_the_owned_workspace() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = fixture(&storage, "workspace-failure");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let before: std::collections::BTreeSet<_> = std::fs::read_dir(output.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let checks = Cell::new(0);
    let cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() > 30
    };
    assert!(FixedWorkspace::prepare(&fixed, output.path(), &cancelled).is_err());
    let after: std::collections::BTreeSet<_> = std::fs::read_dir(output.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(after, before);
    let reached_copy = Cell::new(false);
    let cancel_copy = || {
        for entry in std::fs::read_dir(output.path()).unwrap() {
            let candidate = entry.unwrap().path();
            if !before.contains(&candidate) && candidate.join("meetings").is_dir() {
                reached_copy.set(true);
                return true;
            }
        }
        false
    };
    assert!(FixedWorkspace::prepare(&fixed, output.path(), &cancel_copy).is_err());
    assert!(reached_copy.get());
    let after: std::collections::BTreeSet<_> = std::fs::read_dir(output.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(after, before);
    let audio = storage.recorded_audio_plan(&meeting).unwrap();
    let id = &audio.tracks[0].chunks[0].asset_id;
    std::fs::write(&source.assets[id], b"changed verified synthetic audio").unwrap();
    let result =
        FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled);
    assert!(matches!(result,Err(e) if e.contains("変更")));
    let after: std::collections::BTreeSet<_> = std::fs::read_dir(output.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(after, before);
    assert!(storage.recorded_audio_plan(&meeting).is_ok());
}

#[test]
fn every_meeting_is_reconstructed_without_cross_meeting_audio_or_markdown() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let first = fixture(&storage, "workspace-first");
    let second = fixture(&storage, "workspace-second");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let mut workspace =
        FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    assert_eq!(
        workspace
            .database
            .query_row("SELECT COUNT(*) FROM meetings", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let mut restored = std::collections::BTreeSet::new();
    for original in [&first, &second] {
        let meeting = ids.get("meetings", "", original).unwrap();
        assert!(restored.insert(meeting.clone()));
        let audio = super::super::super::super::transcription::read_audio_plan(
            &workspace.database,
            &meeting,
        )
        .unwrap();
        let path: String = workspace
            .database
            .query_row(
                "SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2",
                params![audio.manifest_asset_id, meeting],
                |r| r.get(0),
            )
            .unwrap();
        let manifest: ImportManifest =
            serde_json::from_slice(&std::fs::read(workspace.stage.path().join(&path)).unwrap())
                .unwrap();
        assert_eq!(manifest.meeting_id, meeting);
        let source_body = storage.get_minutes_markdown(original).unwrap().unwrap();
        let tx = workspace.database.transaction().unwrap();
        let content = crate::storage::minutes_markdown::read(&tx, &meeting)
            .unwrap()
            .unwrap();
        assert_eq!(content.meeting_id, meeting);
        assert_eq!(content.markdown, source_body.markdown);
        tx.commit().unwrap();
    }
    assert_eq!(storage.list_meetings().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn workspace_and_its_database_and_audio_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    fixture(&storage, "workspace-private");
    let source = read(&storage, output.path());
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let workspace =
        FixedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    assert_eq!(
        std::fs::metadata(workspace.stage.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(workspace.stage.path().join("fixed.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let mut query = workspace
        .database
        .prepare("SELECT relative_path FROM file_assets WHERE state='ready'")
        .unwrap();
    for relative in query.query_map([], |r| r.get::<_, String>(0)).unwrap() {
        assert_eq!(
            std::fs::metadata(workspace.stage.path().join(relative.unwrap()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
