use super::*;
use restore_fixed::workspace::seal::SealedWorkspace;
use restore_fixed::FixedData;
use restore_ids::RestoreIds;

#[test]
fn owned_voice_audio_roundtrips_without_its_deleted_source_meeting() {
    use crate::storage::deletion::operations;
    use crate::storage::voice::tests::{fixture, register};
    let (_root, storage, meeting, speaker) = fixture();
    let person = register(&storage, &meeting, &speaker);
    let id = storage.voice_people().unwrap()[0].samples[0].id.clone();
    let before = std::fs::read(
        storage
            .root()
            .join(crate::storage::voice::audio::relative(&person, &id).unwrap()),
    )
    .unwrap();
    let controller = crate::analysis_worker::AnalysisController::default();
    let lease = controller
        .begin_data_maintenance(false, false, &|| false)
        .unwrap();
    let mut deletion = operations::Prepared::fresh(&storage, &meeting, &|| false).unwrap();
    let preview = deletion.preview();
    assert!(
        deletion
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
    drop(lease);
    assert!(storage.list_meetings().unwrap().is_empty());
    assert!(storage.voice_people().unwrap()[0].samples[0].audio_available);
    assert!(!storage.voice_people().unwrap()[0].samples[0].source_available);
    storage
        .prepare_voice_sample_audio_cancellable(&id, &|| false)
        .unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("independent-voice.monbackup");
    storage.create_backup_archive(&path, &|| false).unwrap();
    let source = archive::read(&mut File::open(&path).unwrap(), output.path(), &|| false).unwrap();
    assert_eq!(source.manifest.format_version, 7);
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &|| false).unwrap();
    let sealed = SealedWorkspace::prepare(&fixed, output.path(), &|| false).unwrap();
    let target = tempfile::tempdir().unwrap();
    let restored = Storage::open(target.path().into()).unwrap();
    restored.publish_restore(&sealed, &ids, &|| false).unwrap();
    let p = restored.voice_people().unwrap().remove(0);
    assert_ne!(p.id, person);
    assert_ne!(p.samples[0].id, id);
    assert!(p.samples[0].source_meeting_id.is_none());
    assert!(p.samples[0].audio_available);
    let file = restored
        .root()
        .join(crate::storage::voice::audio::relative(&p.id, &p.samples[0].id).unwrap());
    assert_eq!(std::fs::read(file).unwrap(), before);
    assert_eq!(
        restored
            .prepare_voice_sample_audio_cancellable(&p.samples[0].id, &|| false)
            .unwrap()
            .total_frames(),
        16000
    );
    restored.publish_restore(&sealed, &ids, &|| false).unwrap();
    assert_eq!(restored.voice_people().unwrap().len(), 1);
    restored
        .create_backup_archive(&output.path().join("second.monbackup"), &|| false)
        .unwrap();
}
