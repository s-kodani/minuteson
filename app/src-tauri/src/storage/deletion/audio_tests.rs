use super::*;
use crate::storage::{analysis::tests as analysis_fixture, minutes_ui::tests as minutes_fixture};
use std::{cell::Cell, path::PathBuf};

pub(crate) struct Fixture {
    pub(super) base: tempfile::TempDir,
    pub(crate) storage: Storage,
    pub(crate) meeting: String,
    pub(super) session: String,
    pub(super) track: String,
    pub(super) asset: String,
    pub(crate) path: PathBuf,
}
impl Fixture {
    pub(crate) fn corrupt_operation_header(&self, operation: &str, header: &str) {
        let db = self.storage.connection.lock().unwrap();
        // Fault injection on this fixture's private DB only. Restore the real
        // readonly fence before exposing the corrupt record to product code.
        db.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        )
        .unwrap();
        db.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            false,
        )
        .unwrap();
        db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        db.execute(
            "UPDATE maintenance_operations SET target_manifest_json=?2 WHERE id=?1",
            params![operation, header],
        )
        .unwrap();
        db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        db.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            true,
        )
        .unwrap();
        self.storage.deletion_fence.arm(&db).unwrap();
    }
    pub(crate) fn reopen(self) -> Self {
        let Self {
            base,
            storage,
            meeting,
            session,
            track,
            asset,
            path,
        } = self;
        let root = storage.root().to_path_buf();
        drop(storage);
        let storage = Storage::open(root).unwrap();
        Self {
            base,
            storage,
            meeting,
            session,
            track,
            asset,
            path,
        }
    }
    pub(crate) fn unregistered_audio(&self) -> PathBuf {
        self.file(&format!("audio/{}/system.caf", self.session), None)
    }
    pub(crate) fn operation_count(&self) -> i64 {
        self.storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM maintenance_operations WHERE kind='delete_assets'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    pub(crate) fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let storage = Storage::open(base.path().join("root")).unwrap();
        let meeting = storage.create_meeting("PRIVATE MEETING TITLE").unwrap().id;
        let session = uuid::Uuid::new_v4().to_string();
        let track = uuid::Uuid::new_v4().to_string();
        let relative = PathBuf::from(format!("meetings/{meeting}/audio/{session}/microphone.caf"));
        let path = storage.root().join(&relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"PRIVATE AUDIO BODY").unwrap();
        let asset = storage
            .register_ready_meeting_asset(&meeting, &relative, "audio/x-caf")
            .unwrap()
            .id;
        {
            let db = storage.connection.lock().unwrap();
            db.execute("UPDATE meetings SET source_type='recording',lifecycle='recorded',duration_us=1000000 WHERE id=?1",params![meeting]).unwrap();
            db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,audio_manifest_hash,created_at,updated_at) VALUES(?1,?2,'saved','system','{}',?1,?3,0,0)",params![session,meeting,"a".repeat(64)]).unwrap();
            db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES(?1,?2,?3,'mic','caf',48000,1,'ready',0,0)",params![track,meeting,session]).unwrap();
            db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,1000000,48000,48000,1,'ready',0)",params![uuid::Uuid::new_v4().to_string(),meeting,track,asset]).unwrap();
        }
        Self {
            base,
            storage,
            meeting,
            session,
            track,
            asset,
            path,
        }
    }
    pub(crate) fn directory(&self) -> PathBuf {
        self.storage.root().join("meetings").join(&self.meeting)
    }
    pub(crate) fn file(&self, path: &str, media: Option<&str>) -> PathBuf {
        let full = self.directory().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, b"PRIVATE FILE CONTENT").unwrap();
        if let Some(media) = media {
            self.storage
                .register_ready_meeting_asset(
                    &self.meeting,
                    full.strip_prefix(self.storage.root()).unwrap(),
                    media,
                )
                .unwrap();
        }
        full
    }
    pub(super) fn job(&self, kind: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.storage.connection.lock().unwrap().execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,created_at,updated_at) VALUES(?1,?2,?3,'failed',?1,?4,'{}','{}',?4,1,1)",params![id,self.meeting,kind,"a".repeat(64)]).unwrap();
        id
    }
    pub(super) fn census(&self) -> Census {
        Census::capture(&self.storage, &self.meeting, &|| false).unwrap()
    }
    pub(super) fn preview(&self) -> Preview {
        self.census()
            .preview(&uuid::Uuid::new_v4().to_string())
            .unwrap()
    }
    pub(crate) fn documents(&self) {
        let mut input = analysis_fixture::input();
        for segment in &mut input.segments {
            segment.track_id = self.track.clone();
        }
        for utterance in &mut input.utterances {
            utterance.track_id = self.track.clone();
            utterance.text = "PRIVATE TRANSCRIPT BODY".into();
        }
        let analysis = self
            .storage
            .create_analysis_candidate(&self.meeting, input)
            .unwrap();
        self.storage
            .commit_analysis_candidate(&self.meeting, &analysis, 0, false)
            .unwrap();
        self.storage
            .create_transcript_draft(&self.meeting, &analysis)
            .unwrap();
        for _ in 0..2 {
            let candidate = minutes_fixture::candidate(&self.storage, &self.meeting);
            self.storage
                .adopt_minutes(minutes_fixture::request(
                    &self.storage,
                    &self.meeting,
                    &candidate,
                ))
                .unwrap();
            self.storage
                .create_minutes_draft(&self.meeting, &candidate)
                .unwrap();
        }
    }
}

fn entry<'a>(preview: &'a Preview, suffix: &str) -> &'a Entry {
    preview
        .entries
        .iter()
        .find(|entry| entry.relative_path.ends_with(suffix))
        .unwrap()
}

#[test]
fn audio_only_census_keeps_documents_history_external_originals_and_neighbors_without_payloads() {
    let f = Fixture::new();
    f.documents();
    let manifest = f.file(
        &format!("audio/{}/manifest.json", f.session),
        Some("application/json"),
    );
    let external = f.base.path().join("external.md");
    let original = f.base.path().join("original.m4a");
    std::fs::write(&external, b"PRIVATE EXTERNAL EXPORT").unwrap();
    std::fs::write(&original, b"PRIVATE EXTERNAL ORIGINAL").unwrap();
    let other = f.storage.create_meeting("NEIGHBOR").unwrap().id;
    let other_file = f
        .storage
        .root()
        .join(format!("meetings/{other}/untouched.caf"));
    std::fs::create_dir_all(other_file.parent().unwrap()).unwrap();
    std::fs::write(&other_file, b"PRIVATE NEIGHBOR AUDIO").unwrap();
    // Export destinations are counted but never copied into the review or opened.
    f.storage.connection.lock().unwrap().execute("INSERT INTO export_records(id,meeting_id,request_hash,exported_content_hash,output_content_hash,output_size_bytes,format,destination_path,state,created_at,updated_at,exported_at) VALUES(?1,?2,?3,?3,?3,1,'markdown',?4,'succeeded',1,1,1)",params![uuid::Uuid::new_v4().to_string(),f.meeting,"b".repeat(64),external.to_str().unwrap()]).unwrap();
    let census = f.census();
    let preview = census.preview(&uuid::Uuid::new_v4().to_string()).unwrap();
    assert!(!preview.execute_allowed);
    assert_eq!(preview.audio_files, 1);
    assert_eq!(preview.tracks[0].id, f.track);
    assert_eq!(
        (preview.tracks[0].chunks, preview.tracks[0].duration_us),
        (1, 1_000_000)
    );
    assert_eq!(preview.retained.minutes_revisions, 2);
    // Each generated minutes input keeps its own fixed analysis snapshot.
    assert_eq!(preview.retained.analysis_revisions, 3);
    assert_eq!(preview.retained.utterances, 3);
    assert_eq!(preview.retained.drafts, 3);
    assert_eq!(preview.retained.jobs, 2);
    assert!(preview.retained.evidence_links > 0);
    assert_eq!(preview.retained.external_export_records, 1);
    assert_eq!(
        entry(&preview, "manifest.json").disposition,
        Disposition::Retain
    );
    let encoded = serde_json::to_string(&preview).unwrap();
    for secret in [
        "PRIVATE",
        external.to_str().unwrap(),
        original.to_str().unwrap(),
        other.as_str(),
    ] {
        assert!(!encoded.contains(secret));
    }
    census.verify(&f.storage, &|| false).unwrap();
    assert_eq!(f.preview().census_hash, preview.census_hash);
    for file in [&f.path, &manifest, &external, &original, &other_file] {
        assert!(file.is_file());
    }
    assert_eq!(f.storage.list_meetings().unwrap().len(), 2);
    assert_eq!(
        f.storage
            .minutes_versions(&f.meeting)
            .unwrap()
            .current_id
            .is_some(),
        true
    );
}

#[test]
fn audio_only_census_includes_processed_tracks_and_registered_audio_outside_producer_layout() {
    let f = Fixture::new();
    let file = f.file("arbitrary/nested/processed.flac", Some("audio/flac"));
    let db = f.storage.connection.lock().unwrap();
    let asset: String = db
        .query_row(
            "SELECT id FROM file_assets WHERE relative_path=?1",
            params![file
                .strip_prefix(f.storage.root())
                .unwrap()
                .to_str()
                .unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let track = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES(?1,?2,?3,'processed','flac',48000,1,'ready',0,0)",params![track,f.meeting,f.session]).unwrap();
    db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,1000000,48000,48000,1,'ready',0)",params![uuid::Uuid::new_v4().to_string(),f.meeting,track,asset]).unwrap();
    db.execute(
        "INSERT INTO track_inputs VALUES(?1,?2,?3,'source')",
        params![f.meeting, track, f.track],
    )
    .unwrap();
    drop(db);
    f.file("not-a-chunk/orphan.wav", Some("audio/wav"));
    let preview = f.preview();
    assert_eq!(preview.tracks.len(), 2);
    assert_eq!(preview.audio_files, 3);
    assert_eq!(
        entry(&preview, "processed.flac").disposition,
        Disposition::RegisteredAudio
    );
    assert_eq!(
        entry(&preview, "orphan.wav").disposition,
        Disposition::RegisteredAudio
    );
}

#[test]
fn audio_only_census_accounts_for_recording_leftovers_and_pcm_but_not_result_text_or_logs() {
    let f = Fixture::new();
    let job = f.job("transcription");
    let diarization = f.job("diarization");
    for id in [&job, &diarization] {
        let unit = format!("analysis/{id}/unit-0-{}", uuid::Uuid::new_v4());
        for leaf in ["input.wav", "result.json", "stdout.log", "stderr.log"] {
            f.file(&format!("{unit}/{leaf}"), None);
        }
    }
    f.file(&format!("audio/{}/system.caf", f.session), None);
    f.file(&format!("audio/{}/manifest.json", f.session), None);
    f.file("audio/unknown-speaker.wav", None);
    let preview = f.preview();
    assert_eq!(preview.audio_files, 4);
    assert_eq!(preview.unregistered_audio_files, 3);
    assert_eq!(
        preview
            .entries
            .iter()
            .filter(|e| e.disposition == Disposition::AnalysisPcm)
            .count(),
        2
    );
    assert_eq!(
        entry(&preview, "system.caf").disposition,
        Disposition::ProducerAudioLeftover
    );
    assert_eq!(
        entry(&preview, "manifest.json").disposition,
        Disposition::Retain
    );
    for e in &preview.entries {
        if e.relative_path.ends_with("result.json") || e.relative_path.ends_with(".log") {
            assert_eq!(e.disposition, Disposition::Retain);
        }
    }
    assert_eq!(
        entry(&preview, "unknown-speaker.wav").disposition,
        Disposition::Unclassified
    );
    assert_eq!(preview.unclassified_files, 1);
}

#[test]
fn audio_only_census_accounts_for_managed_import_original_conversion_and_decoded_copy() {
    let f = Fixture::new();
    // An independent import-source meeting; the external source is not in DB.
    let meeting = f.storage.create_meeting("IMPORT").unwrap().id;
    let import = uuid::Uuid::new_v4().to_string();
    let db = f.storage.connection.lock().unwrap();
    db.execute(
        "UPDATE meetings SET source_type='audio_import',lifecycle='imported' WHERE id=?1",
        params![meeting],
    )
    .unwrap();
    db.execute("INSERT INTO audio_imports(id,meeting_id,state,original_file_name,detected_format,created_at,updated_at) VALUES(?1,?2,'failed','PRIVATE ORIGINAL NAME','m4a',1,1)",params![import,meeting]).unwrap();
    drop(db);
    let directory = f
        .storage
        .root()
        .join(format!("meetings/{meeting}/imports/{import}"));
    std::fs::create_dir_all(&directory).unwrap();
    for leaf in ["original.m4a", "audio.flac", "decoded.caf", "manifest.json"] {
        std::fs::write(directory.join(leaf), b"PRIVATE IMPORT").unwrap();
    }
    let original = f
        .storage
        .register_ready_meeting_asset(
            &meeting,
            &directory
                .join("original.m4a")
                .strip_prefix(f.storage.root())
                .unwrap(),
            "audio/mp4",
        )
        .unwrap();
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE audio_imports SET original_asset_id=?2 WHERE id=?1",
            params![import, original.id],
        )
        .unwrap();
    let preview = Census::capture(&f.storage, &meeting, &|| false)
        .unwrap()
        .preview(&uuid::Uuid::new_v4().to_string())
        .unwrap();
    assert_eq!(preview.audio_files, 3);
    assert_eq!(
        entry(&preview, "original.m4a").disposition,
        Disposition::RegisteredAudio
    );
    assert_eq!(
        entry(&preview, "decoded.caf").disposition,
        Disposition::ProducerAudioLeftover
    );
    assert_eq!(
        entry(&preview, "manifest.json").disposition,
        Disposition::Retain
    );
    assert!(!serde_json::to_string(&preview).unwrap().contains("PRIVATE"));
}

#[test]
fn audio_only_census_distinguishes_missing_and_changed_audio_and_keeps_the_original_registration() {
    let f = Fixture::new();
    let missing = f.file("missing.flac", Some("audio/flac"));
    std::fs::remove_file(&missing).unwrap();
    std::fs::write(&f.path, b"CHANGED AUDIO").unwrap();
    let preview = f.preview();
    assert_eq!(
        (
            preview.audio_files,
            preview.missing_audio_files,
            preview.changed_audio_files
        ),
        (2, 1, 1)
    );
    let missing = entry(&preview, "missing.flac");
    assert!(!missing.present && !missing.changed);
    assert!(missing.bytes.is_none() && missing.sha256.is_none() && missing.asset_id.is_some());
    let changed = entry(&preview, "microphone.caf");
    assert!(changed.present && changed.changed);
    let state: String = f
        .storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM file_assets WHERE id=?1",
            params![f.asset],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "ready");
}

#[test]
fn audio_only_census_never_promotes_unknown_files_from_extensions_or_neighbor_session_ids() {
    let f = Fixture::new();
    f.file("notes/audio.wav", None);
    f.file(
        &format!("audio/{}/microphone.caf", uuid::Uuid::new_v4()),
        None,
    );
    f.file(
        &format!(
            "analysis/{}/unit-0-{}/input.wav",
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4()
        ),
        None,
    );
    f.file("registered-document.wav", Some("text/plain"));
    let preview = f.preview();
    assert_eq!(preview.audio_files, 1);
    assert_eq!(preview.unclassified_files, 3);
    assert_eq!(
        entry(&preview, "registered-document.wav").disposition,
        Disposition::Retain
    );
}

#[test]
fn audio_only_census_rejects_audio_reference_to_non_audio_asset() {
    let f = Fixture::new();
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE file_assets SET media_type='text/plain' WHERE id=?1",
            params![f.asset],
        )
        .unwrap();
    assert!(Census::capture(&f.storage, &f.meeting, &|| false)
        .unwrap_err_string()
        .contains("種類"));
    assert!(f.path.is_file());
}

#[test]
fn audio_only_census_rejects_manifest_misclassification_instead_of_erasing_provenance() {
    let f = Fixture::new();
    let manifest = f.file(
        &format!("audio/{}/manifest.json", f.session),
        Some("application/json"),
    );
    let db = f.storage.connection.lock().unwrap();
    let (id, hash): (String, String) = db
        .query_row(
            "SELECT id,content_hash FROM file_assets WHERE relative_path=?1",
            params![manifest
                .strip_prefix(f.storage.root())
                .unwrap()
                .to_str()
                .unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    db.execute(
        "UPDATE recording_sessions SET audio_manifest_hash=?2 WHERE id=?1",
        params![f.session, hash],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        entry(&f.preview(), "manifest.json").disposition,
        Disposition::Retain
    );
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE file_assets SET media_type='audio/wav' WHERE id=?1",
            params![id],
        )
        .unwrap();
    assert!(Census::capture(&f.storage, &f.meeting, &|| false)
        .unwrap_err_string()
        .contains("manifest"));
    assert!(manifest.is_file() && f.path.is_file());
}

#[test]
fn audio_only_census_makes_no_sql_changes_even_with_saved_document_edits_and_failed_jobs() {
    let f = Fixture::new();
    f.documents();
    f.job("diarization");
    let changes = f.storage.connection.lock().unwrap().total_changes();
    let census = f.census();
    let preview = census.preview(&uuid::Uuid::new_v4().to_string()).unwrap();
    census.verify(&f.storage, &|| false).unwrap();
    assert_eq!(
        f.storage.connection.lock().unwrap().total_changes(),
        changes
    );
    assert!(!preview.execute_allowed);
    assert!(f.path.is_file());
}

#[test]
fn audio_only_census_invalidates_changed_audio_retained_document_unknown_file_and_track_state() {
    for mode in 0..4 {
        let f = Fixture::new();
        let retained = f.file("notes.json", Some("application/json"));
        let census = f.census();
        match mode {
            0 => std::fs::write(&f.path, b"CHANGED AUDIO").unwrap(),
            1 => std::fs::write(&retained, b"CHANGED RETAINED NOTES").unwrap(),
            2 => {
                f.file("new-unknown.wav", None);
            }
            _ => {
                f.storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE audio_tracks SET state='interrupted' WHERE id=?1",
                        params![f.track],
                    )
                    .unwrap();
            }
        }
        assert!(census.verify(&f.storage, &|| false).is_err());
        assert!(f.path.is_file() && retained.is_file());
    }
}

#[test]
fn audio_only_census_requires_new_review_after_edits_to_retained_text() {
    let f = Fixture::new();
    f.documents();
    let census = f.census();
    let draft = f
        .storage
        .load_transcript_draft(&f.meeting)
        .unwrap()
        .unwrap();
    let mut payload = draft.payload.clone();
    let analysis = f
        .storage
        .load_current_analysis(&f.meeting)
        .unwrap()
        .unwrap();
    payload
        .utterances
        .push(crate::storage::drafts::UtteranceEdit {
            utterance_id: analysis.utterances[0].id.clone(),
            effective_text: "CHANGED PRIVATE TRANSCRIPT".into(),
            speaker_ids: vec![],
        });
    f.storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    assert!(census.verify(&f.storage, &|| false).is_err());
}

#[test]
fn audio_only_census_rejects_links_aliases_special_objects_and_registered_path_escape() {
    use std::os::unix::fs::symlink;
    for mode in 0..4 {
        let f = Fixture::new();
        let external = f.base.path().join("external.caf");
        std::fs::write(&external, b"EXTERNAL AUDIO").unwrap();
        match mode {
            0 => {
                std::fs::remove_file(&f.path).unwrap();
                symlink(&external, &f.path).unwrap();
            }
            1 => {
                std::fs::hard_link(&external, f.directory().join("alias.caf")).unwrap();
            }
            2 => {
                std::fs::remove_file(&f.path).unwrap();
                std::fs::create_dir(&f.path).unwrap();
            }
            _ => {
                f.storage
                    .connection
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE file_assets SET relative_path='../external.caf' WHERE id=?1",
                        params![f.asset],
                    )
                    .unwrap();
            }
        }
        assert!(Census::capture(&f.storage, &f.meeting, &|| false).is_err());
        assert_eq!(std::fs::read(&external).unwrap(), b"EXTERNAL AUDIO");
    }
}

#[test]
fn audio_only_census_rejects_replaced_root_and_database_without_adopting_them() {
    for database in [false, true] {
        let f = Fixture::new();
        if database {
            let path = f.storage.root().join("meetings.sqlite3");
            std::fs::rename(&path, f.storage.root().join("original.sqlite3")).unwrap();
            std::fs::write(&path, b"REPLACEMENT DATABASE").unwrap();
        } else {
            std::fs::rename(f.storage.root(), f.base.path().join("old-root")).unwrap();
            std::fs::create_dir(f.storage.root()).unwrap();
        }
        assert!(Census::capture(&f.storage, &f.meeting, &|| false).is_err());
    }
}

#[test]
fn audio_only_census_blocks_active_work_pending_maintenance_and_invalid_ids() {
    let f = Fixture::new();
    let job = f.job("transcription");
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE processing_jobs SET state='running' WHERE id=?1",
            params![job],
        )
        .unwrap();
    assert!(Census::capture(&f.storage, &f.meeting, &|| false).is_err());
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE processing_jobs SET state='failed' WHERE id=?1",
            params![job],
        )
        .unwrap();
    let census = f.census();
    assert!(census.preview("inspection").is_err());
    assert!(Census::capture(&f.storage, "../meeting", &|| false).is_err());
    f.storage.connection.lock().unwrap().execute("INSERT INTO maintenance_operations VALUES(?1,?2,'delete_meeting','prepared',NULL,'{}',?3,1,'{}',NULL,1,1)",params![uuid::Uuid::new_v4().to_string(),f.meeting,"a".repeat(64)]).unwrap();
    assert!(Census::capture(&f.storage, &f.meeting, &|| false).is_err());
    assert!(f.path.is_file());
}

#[test]
fn audio_only_census_cancellation_and_mid_inspection_mutation_leave_all_rows_and_files() {
    let f = Fixture::new();
    assert!(Census::capture(&f.storage, &f.meeting, &|| true).is_err());
    let calls = Cell::new(0);
    let check = || {
        calls.set(calls.get() + 1);
        if calls.get() == 60 {
            std::fs::write(&f.path, b"CHANGED DURING CENSUS").unwrap();
        }
        false
    };
    assert!(Census::capture(&f.storage, &f.meeting, &check).is_err());
    assert!(calls.get() >= 60);
    assert!(f.storage.get_meeting(&f.meeting).unwrap().is_some());
    assert_eq!(std::fs::read(&f.path).unwrap(), b"CHANGED DURING CENSUS");
}

// Result::unwrap_err requires Debug on the private observation. Avoid exposing
// internal file proofs merely to render an error in this test.
trait ErrorString {
    fn unwrap_err_string(self) -> String;
}
impl ErrorString for Result<Census, String> {
    fn unwrap_err_string(self) -> String {
        match self {
            Err(error) => error,
            Ok(_) => panic!("expected rejection"),
        }
    }
}
