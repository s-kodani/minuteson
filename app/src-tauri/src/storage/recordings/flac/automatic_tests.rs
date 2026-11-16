use super::*;

fn fail_manifest(f: &Fixture) {
    let path = f.base.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["state"] = json!("failed");
    manifest["errors"] = json!(["synthetic delayed endpoint"]);
    write(&path, &serde_json::to_vec(&manifest).unwrap());
}

#[test]
fn automatic_save_recovers_failed_and_interrupted_sessions_without_duplicate_rows() {
    for failed in [true, false] {
        let f = Fixture::new(2, 2000, true);
        fail_manifest(&f);
        if failed {
            f.storage.fail_recording(&f.session.id).unwrap();
        } else {
            f.storage.interrupt_recording(&f.session.id).unwrap();
        }
        let observed = std::sync::Mutex::new(Vec::new());
        assert_eq!(
            f.storage
                .recover_recordings_observed(&|_| {}, &|item| observed.lock().unwrap().push(item))
                .unwrap(),
            vec![f.session.id.clone()]
        );
        assert_eq!(observed.lock().unwrap().last().unwrap().phase, "succeeded");
        let plan = f
            .storage
            .verify_canonical_audio(&f.session.meeting_id, &|| false)
            .unwrap();
        assert!(plan.tracks[0].end_us >= 4_250_000);
        let (_, known) = f
            .storage
            .recorded_analysis_input(&f.session.meeting_id)
            .unwrap();
        assert!(!known.unwrap().gaps.is_empty());
        f.storage
            .finalize_available_recording(&f.session.id, &|| false)
            .unwrap();
        assert!(f.storage.recover_completed_recordings().unwrap().is_empty());
        let db = f.storage.connection.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM recording_sessions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT lifecycle FROM meetings WHERE id=?1",
                [&f.session.meeting_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "recorded"
        );
        let saved = load_session(&db, &f.session.id).unwrap().unwrap();
        assert_eq!(saved.state, "interrupted");
        assert_eq!(saved.ended_at_utc_ms, None);
    }
}

#[test]
fn automatic_save_keeps_valid_lane_and_never_calls_corrupt_audio_empty() {
    for corrupt_both in [false, true] {
        let f = Fixture::new(1, 2000, false);
        fail_manifest(&f);
        f.storage.fail_recording(&f.session.id).unwrap();
        write(&f.base.join("raw-system-00000001.caf"), b"corrupt");
        if corrupt_both {
            write(&f.base.join("raw-microphone-00000001.caf"), b"corrupt");
        }
        let result = f
            .storage
            .finalize_available_recording(&f.session.id, &|| false);
        if corrupt_both {
            assert_ne!(result.unwrap_err(), NO_CAPTURED_AUDIO);
            assert!(f.base.join("raw-microphone-00000001.caf").exists());
            assert_eq!(f.registered_tracks(), 0);
        } else {
            result.unwrap();
            f.storage
                .verify_canonical_audio(&f.session.meeting_id, &|| false)
                .unwrap();
        }
    }
}

#[test]
fn automatic_save_reports_zero_samples_only_from_verified_empty_prefixes() {
    let f = Fixture::new(1, 2000, false);
    fail_manifest(&f);
    f.storage.fail_recording(&f.session.id).unwrap();
    for lane in ["microphone", "system"] {
        let tip = json!({"schemaVersion":2,"ownerId":f.session.id,"lane":lane,"sealedSegments":0,
            "sealedFrames":0,"lastEntrySHA256":null,"producerClosed":true});
        write(
            &f.base.join(format!("raw-{lane}.json")),
            &serde_json::to_vec(&tip).unwrap(),
        );
    }
    let observed = std::sync::Mutex::new(Vec::new());
    assert!(f
        .storage
        .recover_recordings_observed(&|_| {}, &|item| observed.lock().unwrap().push(item))
        .unwrap()
        .is_empty());
    let observed = observed.lock().unwrap();
    let item = observed.last().unwrap();
    assert_eq!(item.phase, "failed");
    assert!(item.no_audio);
    assert_eq!(item.error.as_deref(), Some(NO_CAPTURED_AUDIO));
    assert_eq!(f.registered_tracks(), 0);
    assert!(
        f.base.join("raw-system-00000001.caf").exists(),
        "unadvertised tail must be retained, not adopted"
    );
}

#[test]
fn automatic_save_retry_after_rollback_and_cancellation_keeps_original_identity() {
    let f = Fixture::new(1, 2000, false);
    fail_manifest(&f);
    f.storage.fail_recording(&f.session.id).unwrap();
    assert!(f
        .storage
        .finalize_available_recording(&f.session.id, &|| true)
        .is_err());
    f.reject_partial_commit(true);
    assert!(f
        .storage
        .finalize_available_recording(&f.session.id, &|| false)
        .is_err());
    assert_eq!(f.registered_tracks(), 0);
    f.reject_partial_commit(false);
    let saved = f
        .storage
        .finalize_available_recording(&f.session.id, &|| false)
        .unwrap();
    assert_eq!(saved.id, f.session.id);
    assert_eq!(saved.meeting_id, f.session.meeting_id);
    f.storage
        .verify_canonical_audio(&saved.meeting_id, &|| false)
        .unwrap();
}

#[test]
fn automatic_save_retries_registered_audio_after_canonical_publication_failure() {
    let f = Fixture::new(1, 2000, false);
    fail_manifest(&f);
    f.storage.fail_recording(&f.session.id).unwrap();
    f.storage.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_wav BEFORE INSERT ON meeting_audio_commits BEGIN SELECT RAISE(ABORT,'synthetic canonical save failure'); END").unwrap();
    assert!(f
        .storage
        .finalize_available_recording(&f.session.id, &|| false)
        .is_err());
    // Import startup work must not silently retry a recording after the
    // recording controller has already published its failed save snapshot.
    f.storage.finish_pending_import_audio().unwrap();
    f.storage
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_wav")
        .unwrap();
    let recovered = f.storage.recover_completed_recordings().unwrap();
    assert_eq!(recovered, vec![f.session.id.clone()]);
    f.storage
        .verify_canonical_audio(&f.session.meeting_id, &|| false)
        .unwrap();
}
