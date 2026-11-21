use super::*;

fn private_directory(parent: &Path, leaf: &str) -> PathBuf {
    let parent = Directory::open(&parent.canonicalize().unwrap()).unwrap();
    parent.create_child(leaf).unwrap().path
}

#[test]
fn verified_deleted_audio_retains_its_status_through_salvage_and_backup() {
    let source = tempfile::tempdir().unwrap();
    let storage = Storage::open(source.path().into()).unwrap();
    let meeting =
        backup::deleted_current_meeting_for_test(&storage, "deleted-audio-salvage-source");
    drop(storage);

    let stage = tempfile::tempdir().unwrap();
    let snapshot = stage.path().join("source.sqlite3");
    std::fs::copy(source.path().join("meetings.sqlite3"), &snapshot).unwrap();
    let operation = Uuid::new_v4().to_string();
    let plan = inspect(
        &operation,
        source.path(),
        None,
        &snapshot,
        stage.path(),
        &|| false,
    )
    .unwrap();
    assert!(plan.preview.items.iter().any(|item| item.kind == "audio"
        && item.source_id.as_deref() == Some(meeting.as_str())
        && matches!(item.status, Status::Deleted)));
    assert!(plan.assets.is_empty());

    let destination = tempfile::tempdir().unwrap();
    let root = private_directory(destination.path(), "data");
    let models = private_directory(destination.path(), "models");
    build(&plan, stage.path(), &root, &models, &|| false).unwrap();
    let storage = Storage::open_with_models(root, models).unwrap();
    let salvaged = remap(&operation, "meetings", "", &meeting);
    let assert_deleted = |storage: &Storage, meeting: &str| {
        assert!(storage
            .canonical_audio_plan(meeting)
            .err()
            .is_some_and(|error| error.contains("削除済み")));
        assert!(storage.load_current_analysis(meeting).unwrap().is_some());
        with_test_database(storage, |db| {
            assert!(deletion::audio::source_deleted(db, meeting).unwrap());
            let record = salvage_schema::meeting(db, meeting).unwrap().unwrap();
            assert!(record.audio_deleted);
            assert!(record.audio_unavailable.is_none());
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM maintenance_operations WHERE kind='delete_assets' AND meeting_id=?1",
                    [meeting],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
                0
            );
        });
    };
    assert_deleted(&storage, &salvaged);

    let archive = destination.path().join("deleted-audio.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let restored = Storage::open(destination.path().join("backup-restore")).unwrap();
    let prepared = restored
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let restored_ids = prepared
        .publish(
            &restored,
            &prepared.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap()
        .meeting_ids;
    assert_eq!(restored_ids.len(), 1);
    assert_deleted(&restored, &restored_ids[0]);
}

#[test]
fn damaged_audio_and_draft_keep_committed_documents_through_rescue_and_backup() {
    let source = tempfile::tempdir().unwrap();
    let storage = Storage::open(source.path().into()).unwrap();
    let good = backup::archive::tests::meeting(&storage, "good-salvage-source");
    let damaged = backup::archive::tests::meeting(&storage, "damaged-salvage-source");
    let draft_only = backup::archive::tests::meeting(&storage, "draft-only-salvage-source");
    let edit = |meeting: &str, text: &str| {
        let view = storage.load_current_analysis(meeting).unwrap().unwrap();
        let utterance = &view.utterances[0];
        let mut draft = storage.load_transcript_draft(meeting).unwrap().unwrap();
        draft.payload.utterances = vec![drafts::UtteranceEdit {
            utterance_id: utterance.id.clone(),
            effective_text: text.into(),
            speaker_ids: utterance.speaker_ids.clone(),
        }];
        storage
            .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
            .unwrap();
        utterance.effective_text.clone()
    };
    let valid_edit = "確認済みの下書きの編集";
    edit(&good, valid_edit);
    let confirmed = edit(&damaged, "破損した下書きの編集");
    let confirmed_draft_only = edit(&draft_only, "本文に混ぜてはいけない編集");
    let markdown = current_minutes_fixture::save(&storage, &damaged, "# 救出する本文\n");
    let canonical_file: String = with_test_database(&storage, |db| {
        db.execute(
            "UPDATE working_drafts SET content_hash=?1 WHERE meeting_id IN (?2, ?3)",
            params!["0".repeat(64), damaged, draft_only],
        )
        .unwrap();
        db.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES('ui_preferences','{}',?1,1,1)",
            ["0".repeat(64)]).unwrap();
        db.query_row(
            "SELECT f.relative_path FROM meeting_audio_commits a JOIN audio_chunks c ON c.track_id=a.track_id JOIN file_assets f ON f.id=c.file_asset_id WHERE a.meeting_id=?1",
            [&damaged], |r| r.get(0),
        ).unwrap()
    });
    std::fs::write(source.path().join(&canonical_file), b"damaged WAV").unwrap();
    drop(storage);
    let origin = source.path().join("meetings.sqlite3");
    let original = std::fs::read(&origin).unwrap();
    let stage = tempfile::tempdir().unwrap();
    let snapshot = stage.path().join("source.sqlite3");
    std::fs::copy(&origin, &snapshot).unwrap();
    let operation = Uuid::new_v4().to_string();
    let plan = inspect(
        &operation,
        source.path(),
        None,
        &snapshot,
        stage.path(),
        &|| false,
    )
    .unwrap();
    assert_eq!(plan.preview.meeting_total, Some(3));
    assert!(plan.preview.items.iter().all(|item| item.kind != "draft"));
    for meeting in [&damaged, &draft_only] {
        assert!(plan
            .preview
            .items
            .iter()
            .any(|item| item.kind == "transcript"
                && item.source_id.as_deref() == Some(meeting.as_str())
                && matches!(item.status, Status::Recoverable)
                && item
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("保存済みの文字起こし")
                        && reason.contains("編集途中の下書きは復元できません"))));
    }
    assert!(plan.preview.items.iter().any(|item| item.kind == "audio"
        && item.source_id.as_deref() == Some(&damaged)
        && matches!(item.status, Status::Unavailable)));
    assert!(plan
        .preview
        .items
        .iter()
        .filter(|item| item.source_id.as_deref() == Some(&draft_only))
        .all(|item| !matches!(item.status, Status::Unavailable | Status::Uninspected)));
    assert!(plan
        .preview
        .items
        .iter()
        .any(|item| item.kind == "setting" && matches!(item.status, Status::Defaulted)));
    let plan: Plan = serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    let destination = tempfile::tempdir().unwrap();
    let root = private_directory(destination.path(), "data");
    let models = private_directory(destination.path(), "models");
    let receipt = build(&plan, stage.path(), &root, &models, &|| false).unwrap();
    assert_eq!(receipt.meeting_count, 3);
    assert_eq!(std::fs::read(&origin).unwrap(), original);
    assert_eq!(
        std::fs::read(source.path().join(&canonical_file)).unwrap(),
        b"damaged WAV"
    );
    let delivery = Connection::open_with_flags(
        root.join("meetings.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        delivery
            .query_row("SELECT COUNT(*) FROM working_drafts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(delivery);

    let storage = Storage::open_with_models(root, models).unwrap();
    let saved = storage.salvage_result().unwrap().unwrap();
    assert_eq!(saved.operation_id, operation);
    let good_id = remap(&operation, "meetings", "", &good);
    let damaged_id = remap(&operation, "meetings", "", &damaged);
    let draft_only_id = remap(&operation, "meetings", "", &draft_only);
    assert!(storage.canonical_audio_plan(&good_id).is_ok());
    assert!(storage.canonical_audio_plan(&draft_only_id).is_ok());
    assert!(storage.canonical_audio_plan(&damaged_id).is_err());
    for (id, expected) in [
        (&damaged_id, confirmed.as_str()),
        (&draft_only_id, confirmed_draft_only.as_str()),
        (&good_id, valid_edit),
    ] {
        assert!(storage.load_transcript_draft(id).unwrap().is_none());
        assert_eq!(
            storage
                .load_current_analysis(id)
                .unwrap()
                .unwrap()
                .utterances[0]
                .effective_text,
            expected
        );
    }
    let diagnostics = serde_json::to_value(storage.meeting_list_diagnostics().unwrap()).unwrap();
    assert!(diagnostics["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["meetingId"].as_str() != Some(draft_only_id.as_str())));
    let reason = with_test_database(&storage, |db| {
        assert_eq!(
            minutes_markdown::read(db, &damaged_id)
                .unwrap()
                .unwrap()
                .markdown,
            markdown.markdown
        );
        assert!(!deletion::audio::source_deleted(db, &damaged_id).unwrap());
        salvage_schema::meeting(db, &damaged_id).unwrap().unwrap()
    });
    assert!(reason.audio_unavailable.is_some());
    assert!(!reason.notes.contains_key("draft"));
    let _ = storage.finish_pending_import_audio().unwrap();

    let outgoing = tempfile::tempdir().unwrap();
    let archive = outgoing.path().join("rescued.monbackup");
    storage.create_backup_archive(&archive, &|| false).unwrap();
    let restored_root = tempfile::tempdir().unwrap();
    let restored = Storage::open(restored_root.path().into()).unwrap();
    let prepared = restored
        .prepare_additive_restore(&archive, &|| false, &|_| {})
        .unwrap();
    let result = prepared
        .publish(
            &restored,
            &prepared.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(result.meeting_ids.len(), 3);
    let mut restored_ids = BTreeMap::new();
    with_test_database(&restored, |db| {
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM working_drafts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        for id in &result.meeting_ids {
            let record = salvage_schema::meeting(db, id).unwrap().unwrap();
            assert!(!record.notes.contains_key("draft"));
            assert_eq!(
                record.audio_unavailable.is_some(),
                record.source_meeting_id == damaged
            );
            restored_ids.insert(record.source_meeting_id, id.as_str());
        }
        let id = restored_ids[&damaged];
        assert_eq!(
            minutes_markdown::read(db, id).unwrap().unwrap().markdown,
            markdown.markdown
        );
        assert!(!deletion::audio::source_deleted(db, id).unwrap());
    });
    for (source, expected) in [
        (&damaged, confirmed.as_str()),
        (&draft_only, confirmed_draft_only.as_str()),
        (&good, valid_edit),
    ] {
        let id = restored_ids[source];
        let view = restored.load_current_analysis(id).unwrap().unwrap();
        assert_eq!(view.utterances[0].effective_text, expected);
        restored.create_transcript_draft(id, &view.id).unwrap();
    }
}

#[test]
fn verified_models_and_voice_features_are_registered_with_new_ids() {
    use crate::model_inspection::{self, Engine, PackageFile, PackageManifest, Role, Target};
    let source = tempfile::tempdir().unwrap();
    let data = private_directory(source.path(), "data");
    let models = private_directory(source.path(), "models");
    let storage = Storage::open_with_models(data.clone(), models.clone()).unwrap();
    let material = tempfile::tempdir().unwrap();
    let package = |name: &str, id: &str| {
        let folder = private_directory(material.path(), name);
        let manifest = PackageManifest {
            backend: None,
            schema_version: 1,
            engine: Engine::Whisper,
            engine_version: "fixture".into(),
            model_name: name.into(),
            model_version: "1".into(),
            quantization: None,
            license_ref: "fixture".into(),
            target: Target::current().unwrap(),
            files: vec![
                PackageFile {
                    relative_path: "engine-cli".into(),
                    role: Role::Executable,
                    expected_size: None,
                    expected_hash: None,
                },
                PackageFile {
                    relative_path: "model.bin".into(),
                    role: Role::Asr,
                    expected_size: None,
                    expected_hash: None,
                },
            ],
        };
        std::fs::write(folder.join("engine-cli"), b"verification only").unwrap();
        std::fs::write(folder.join("model.bin"), name.as_bytes()).unwrap();
        std::fs::write(
            folder.join(model_inspection::MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let report = model_inspection::inspect(&folder, &|| false).unwrap();
        crate::model_placement::place(&storage, &models, &folder, id, &report.review_hash, &|| {
            false
        })
        .unwrap();
    };
    let good_model = Uuid::new_v4().to_string();
    let bad_model = Uuid::new_v4().to_string();
    package("good", &good_model);
    package("bad", &bad_model);
    std::fs::write(models.join(&bad_model).join("model.bin"), b"damaged").unwrap();
    let person = Uuid::new_v4().to_string();
    let sample = Uuid::new_v4().to_string();
    let missing_meeting = Uuid::new_v4().to_string();
    with_test_database(&storage, |db| {
        db.execute(
            "INSERT INTO voice_people(id,name,created_at) VALUES(?1,'声の検証用人物',1)",
            [&person],
        )
        .unwrap();
        db.execute("INSERT INTO voice_samples(id,person_id,model_hash,extractor_version,dimension,vector,source_meeting_id,start_us,end_us,created_at) VALUES(?1,?2,?3,?4,2,?5,?6,0,1,1)",
            params![sample,person,"b".repeat(64),crate::voice::EXTRACTOR,crate::voice::encode(&[1.,0.]).unwrap(),missing_meeting]).unwrap();
        db.execute(
            "INSERT INTO voice_sample_audio(id,state) VALUES(?1,'unavailable')",
            [&sample],
        )
        .unwrap();
    });
    drop(storage);
    let workspace = tempfile::tempdir().unwrap();
    let copy = workspace.path().join("sqlite");
    std::fs::copy(data.join("meetings.sqlite3"), &copy).unwrap();
    let operation = Uuid::new_v4().to_string();
    let plan = inspect(
        &operation,
        &data,
        Some(&models),
        &copy,
        workspace.path(),
        &|| false,
    )
    .unwrap();
    assert!(plan.preview.items.iter().any(|item| item.kind == "model"
        && item.source_id.as_deref() == Some(&bad_model)
        && matches!(item.status, Status::Unavailable)));
    assert!(plan
        .preview
        .items
        .iter()
        .any(|item| item.kind == "voice_sample"
            && item.source_id.as_deref() == Some(&sample)
            && matches!(item.status, Status::Partial)
            && item.reason.as_deref().is_some_and(|reason| {
                reason.contains("元の会議との関連は引き継げません")
                    && !reason.contains(&missing_meeting)
            })));
    let destination = tempfile::tempdir().unwrap();
    let target = private_directory(destination.path(), "data");
    let target_models = private_directory(destination.path(), "models");
    assert!(build(&plan, workspace.path(), &target, &target_models, &|| true).is_err());
    assert!(!target.join("meetings.sqlite3").exists());
    let receipt = build(&plan, workspace.path(), &target, &target_models, &|| false).unwrap();
    assert_eq!((receipt.voice_person_count, receipt.model_count), (1, 1));
    let new_model = remap(&operation, "model_packages", "", &good_model);
    assert_eq!(
        std::fs::read(target_models.join(&new_model).join("model.bin")).unwrap(),
        b"good"
    );
    assert_eq!(
        std::fs::read(models.join(&good_model).join("model.bin")).unwrap(),
        b"good"
    );
    assert_eq!(
        std::fs::read(models.join(&bad_model).join("model.bin")).unwrap(),
        b"damaged"
    );
    let rebuilt = Storage::open_with_models(target, target_models).unwrap();
    with_test_database(&rebuilt, |db| {
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM model_compatibility_checks", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let restored: (Option<String>,String) = db.query_row("SELECT s.source_meeting_id,a.state FROM voice_samples s JOIN voice_sample_audio a ON a.id=s.id", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(restored, (None, "unavailable".into()));
    });
    let catalog = rebuilt.list_model_placements().unwrap();
    assert_eq!(catalog[0].package_id, new_model);
    assert!(catalog[0].compatibility.is_none());
}

#[test]
fn empty_source_cannot_authorize_a_switch() {
    let source = tempfile::tempdir().unwrap();
    drop(Storage::open(source.path().into()).unwrap());
    let workspace = tempfile::tempdir().unwrap();
    let copy = workspace.path().join("sqlite");
    std::fs::copy(source.path().join("meetings.sqlite3"), &copy).unwrap();
    let operation = Uuid::new_v4().to_string();
    let plan = inspect(
        &operation,
        source.path(),
        None,
        &copy,
        workspace.path(),
        &|| false,
    )
    .unwrap();
    assert_eq!(plan.preview.recoverable_count, 0);
    let target = tempfile::tempdir().unwrap();
    assert!(build(
        &plan,
        workspace.path(),
        target.path(),
        target.path(),
        &|| false
    )
    .is_err());
    assert!(!target.path().join("meetings.sqlite3").exists());
}
