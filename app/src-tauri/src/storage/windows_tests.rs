//! Native Windows smoke tests; Unix shell/audio fixtures do not substitute here.
use super::*;
#[test]
fn windows_parity_interrupted_meeting_deletion_reopens_and_continues_original_scope() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let storage = Storage::open(root.join("data")).unwrap();
    let meeting = storage.create_meeting("中断した削除").unwrap();
    let keep = storage.create_meeting("保持する会議").unwrap();
    for index in 0..3 {
        let relative = PathBuf::from(format!("meetings/{}/audio/{index}.flac", meeting.id));
        let file = storage.managed_path(&relative).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"owned deletion fixture").unwrap();
        storage
            .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
            .unwrap();
    }
    let controller = crate::analysis_worker::AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut prepared =
        deletion::operations::Prepared::fresh(&storage, &meeting.id, &|| false).unwrap();
    let preview = prepared.preview();
    let stop = std::cell::Cell::new(false);
    assert!(prepared
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &|| stop.get(),
            &|phase| {
                if phase == "deleting_files" {
                    stop.set(true);
                }
            }
        )
        .is_err());
    drop(prepared);
    drop(lease);
    drop(storage);
    let storage = Storage::open(root.join("data")).unwrap();
    let mut prepared =
        deletion::operations::Prepared::pending(&storage, &preview.operation_id, &|| false)
            .unwrap();
    let mut preview = prepared.preview();
    assert!(preview.pending);
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    if !preview.execute_allowed {
        assert!(
            preview.reconfirmation_allowed,
            "{:?}",
            preview.resume_reason
        );
        prepared
            .review_remaining(&storage, &lease, &preview.confirmation_id, &|| false)
            .unwrap();
        preview = prepared.preview();
    }
    assert!(preview.execute_allowed, "{:?}", preview.resume_reason);
    assert!(
        prepared
            .execute(
                &storage,
                &lease,
                &preview.confirmation_id,
                false,
                &|| false,
                &|_| {}
            )
            .unwrap()
            .deleted
    );
    assert!(storage.get_meeting(&meeting.id).unwrap().is_none());
    assert!(storage.get_meeting(&keep.id).unwrap().is_some());
}

#[test]
fn windows_parity_audio_deletion_and_deleted_audio_backup_roundtrip_keep_meeting_and_external_original(
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("外部原本.wav");
    std::fs::write(&source, b"external original remains").unwrap();
    let storage = Storage::open(root.join("data")).unwrap();
    let (import, base, _) = imports::tests::pending(&storage, "audio-delete");
    storage.finalize_audio_import(&import.id).unwrap();
    let controller = crate::analysis_worker::AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut deletion =
        deletion::audio::operations::Prepared::fresh(&storage, &import.meeting_id, &lease, &|| {
            false
        })
        .unwrap();
    let preview = deletion.preview();
    assert!(preview.execute_allowed, "{:?}", preview.resume_reason);
    assert!(deletion
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &|| true,
            &|_| {}
        )
        .is_err());
    storage
        .verify_canonical_audio(&import.meeting_id, &|| false)
        .unwrap();
    let result = deletion
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(result.audio_deleted);
    assert!(storage.get_meeting(&import.meeting_id).unwrap().is_some());
    assert!(storage.recorded_audio_plan(&import.meeting_id).is_err());
    assert_eq!(
        std::fs::read(&source).unwrap(),
        b"external original remains"
    );
    drop(deletion);
    drop(lease);
    let archive = root.join("音声削除済み.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let target = Storage::open(root.join("restored")).unwrap();
    let restore = target
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let result = restore
        .publish(
            &target,
            &restore.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(result.meeting_ids.len(), 1);
    assert!(target
        .get_meeting(&result.meeting_ids[0])
        .unwrap()
        .is_some());
    assert!(target.recorded_audio_plan(&result.meeting_ids[0]).is_err());
    assert_eq!(std::fs::read(source).unwrap(), b"external original remains");
}

#[test]
fn japanese_paths_and_current_meetings_survive_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("会議 データ");
    let id = {
        let storage = Storage::open(root.clone()).unwrap();
        storage.create_meeting("Windowsの会議").unwrap().id
    };
    let storage = Storage::open(root).unwrap();
    let meeting = storage.get_meeting(&id).unwrap().unwrap();
    assert_eq!(meeting.title, "Windowsの会議");
    assert_eq!(storage.list_meetings().unwrap().len(), 1);
}

#[test]
fn unknown_database_is_refused_without_replacing_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("meetings.sqlite3");
    let marker = temp.path().join("existing-audio.flac");
    {
        let db = Connection::open(&database).unwrap();
        db.execute_batch("CREATE TABLE unknown_windows_data(value TEXT);")
            .unwrap();
    }
    std::fs::write(&marker, b"SYNTHETIC EXISTING AUDIO").unwrap();
    let before = std::fs::read(&database).unwrap();
    assert!(Storage::open(temp.path().into()).is_err());
    assert_eq!(std::fs::read(database).unwrap(), before);
    assert_eq!(std::fs::read(marker).unwrap(), b"SYNTHETIC EXISTING AUDIO");
}

#[test]
fn windows_parity_backup_restore_and_meeting_deletion_preserve_other_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let storage = Storage::open(root.join("source")).unwrap();
    let meeting = storage.create_meeting("対象会議").unwrap();
    let other = storage.create_meeting("保持する会議").unwrap();
    let archive = root.join("saved.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let relative = PathBuf::from(format!("meetings/{}/audio/sample.flac", meeting.id));
    let audio = storage.managed_path(&relative).unwrap();
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    std::fs::write(&audio, b"owned test audio").unwrap();
    storage
        .register_ready_meeting_asset(&meeting.id, &relative, "audio/flac")
        .unwrap();
    let restored = Storage::open(root.join("destination")).unwrap();
    let existing = restored.create_meeting("復元先にある会議").unwrap();
    let selected = restored
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let preview = selected.preview();
    let receipt = selected
        .publish(&restored, &preview.confirmation_id, &|| false, &|_| {})
        .unwrap();
    assert_eq!(receipt.meeting_ids.len(), 2);
    assert!(restored.get_meeting(&existing.id).unwrap().is_some());
    assert!(
        selected
            .publish(&restored, &preview.confirmation_id, &|| false, &|_| {})
            .unwrap()
            .already_published
    );
    let mut deletion =
        deletion::operations::Prepared::fresh(&storage, &meeting.id, &|| false).unwrap();
    let preview = deletion.preview();
    assert!(preview.execute_allowed, "{:?}", preview.resume_reason);
    let lease = crate::analysis_worker::AnalysisController::default()
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let result = deletion
        .execute(
            &storage,
            &lease,
            &preview.confirmation_id,
            false,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(result.deleted);
    assert!(!audio.exists());
    assert!(storage.get_meeting(&meeting.id).unwrap().is_none());
    assert!(storage.get_meeting(&other.id).unwrap().is_some());
}
