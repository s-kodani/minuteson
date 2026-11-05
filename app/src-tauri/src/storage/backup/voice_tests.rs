use super::*;
use restore_fixed::workspace::seal::SealedWorkspace;
use restore_fixed::FixedData;
use restore_ids::RestoreIds;
use serde_json::{json, Value};

#[test]
fn current_acoustic_segments_preserve_voice_windows_overlap_and_unknown_speakers() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let source = Storage::open(root.path().into()).unwrap();
    let meeting = archive::tests::meeting(&source, "acoustic-evidence");
    source
        .connection
        .lock()
        .unwrap()
        .execute("DELETE FROM working_drafts WHERE meeting_id=?1", [&meeting])
        .unwrap();
    let plan = source.canonical_audio_plan(&meeting).unwrap();
    let track = plan.tracks[0].id.clone();
    let segments = [
        (Some("a"), 0, 1_000_000, None, Some(0.9)),
        (Some("b"), 1_000_000, 2_000_000, None, Some(0.8)),
        (Some("a"), 2_000_000, 3_000_000, None, None),
        (Some("a"), 3_000_000, 4_000_000, Some("overlap"), Some(0.7)),
        (Some("b"), 3_000_000, 4_000_000, Some("overlap"), Some(0.6)),
        (None, 3_250_000, 3_500_000, None, None),
    ]
    .into_iter()
    .map(
        |(speaker, start, end, overlap, confidence)| SpeakerSegmentInput {
            track_id: track.clone(),
            speaker_key: speaker.map(str::to_owned),
            start_us: start,
            end_us: end,
            overlap_group: overlap.map(str::to_owned),
            confidence,
        },
    )
    .collect();
    let analysis = source
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: plan.manifest_hash,
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![
                    SpeakerInput {
                        key: "a".into(),
                        label: "A".into(),
                    },
                    SpeakerInput {
                        key: "b".into(),
                        label: "B".into(),
                    },
                ],
                segments,
                utterances: vec![UtteranceInput {
                    track_id: track,
                    start_us: 0,
                    end_us: 4_000_000,
                    text: "Aの発言区間にBと重複区間も含まれる".into(),
                    speaker_keys: vec!["a".into()],
                    confidence: None,
                }],
            },
        )
        .unwrap();
    source
        .commit_analysis_candidate(
            &meeting,
            &analysis,
            source.get_meeting(&meeting).unwrap().unwrap().lock_version,
            false,
        )
        .unwrap();
    let windows = |storage: &Storage, meeting: &str| -> Vec<_> {
        storage
            .voice_context(meeting)
            .unwrap()
            .windows
            .iter()
            .map(|w| (w.start_us, w.end_us))
            .collect()
    };
    let before = windows(&source, &meeting);
    assert_eq!(before, vec![(0, 1_000_000), (2_000_000, 3_000_000)]);
    let evidence = |storage: &Storage, meeting: &str| {
        let db = storage.connection.lock().unwrap();
        let mut q = db.prepare("SELECT p.label,s.start_us,s.end_us,s.overlap_group_id IS NOT NULL,s.confidence FROM speaker_segments s LEFT JOIN speakers p ON p.id=s.speaker_id WHERE s.meeting_id=?1 ORDER BY s.start_us,s.end_us,p.label").unwrap();
        q.query_map([meeting], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, Option<f64>>(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let original = evidence(&source, &meeting);
    let path = output.path().join("acoustic-evidence.monbackup");
    source.create_backup_archive(&path, &|| false).unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let selected = target
        .prepare_additive_restore(&path, &|| false, &|_| {})
        .unwrap();
    let preview = selected.preview();
    let receipt = selected
        .publish(&target, &preview.confirmation_id, &|| false, &|_| {})
        .unwrap();
    let restored = &receipt.meeting_ids[0];
    assert_eq!(windows(&target, restored), before);
    assert_eq!(evidence(&target, restored), original);
    let db = target.connection.lock().unwrap();
    let (segments, groups, tracks): (i64, i64, i64) = db.query_row(
        "SELECT COUNT(*),COUNT(DISTINCT overlap_group_id),COUNT(DISTINCT track_id) FROM speaker_segments WHERE meeting_id=?1",
        [restored], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!((segments, groups, tracks), (6, 1, 1));
    for domain in ["speaker_segments", "overlap_group"] {
        let remapped: i64 = db.query_row("SELECT COUNT(*) FROM restore_identity_maps WHERE batch_id=?1 AND domain=?2 AND source_key!=target_key", params![preview.batch_id,domain], |r| r.get(0)).unwrap();
        assert_eq!(remapped, if domain == "speaker_segments" { 6 } else { 1 });
    }
    drop(db);
    target
        .create_backup_archive(&output.path().join("again.monbackup"), &|| false)
        .unwrap();
}

#[test]
fn speaker_assignment_update_round_trip_keeps_current_assignments_without_history() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = archive::tests::meeting(&storage, "assignment-source");
    let (track, manifest): (String, String) = {
        let db = storage.connection.lock().unwrap();
        db.execute("DELETE FROM working_drafts WHERE meeting_id=?1", [&meeting])
            .unwrap();
        db.query_row("SELECT t.id,a.audio_manifest_hash FROM meetings m JOIN audio_tracks t ON t.meeting_id=m.id JOIN analysis_revisions a ON a.id=m.current_analysis_id WHERE m.id=?1",[&meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
    };
    let id = storage
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: manifest,
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![SpeakerInput {
                    key: "a".into(),
                    label: "話者A".into(),
                }],
                segments: vec![SpeakerSegmentInput {
                    track_id: track.clone(),
                    speaker_key: Some("a".into()),
                    start_us: 0,
                    end_us: 1_000_000,
                    overlap_group: None,
                    confidence: None,
                }],
                utterances: vec![
                    UtteranceInput {
                        track_id: track.clone(),
                        start_us: 0,
                        end_us: 500000,
                        text: "Hello".into(),
                        speaker_keys: vec![],
                        confidence: None,
                    },
                    UtteranceInput {
                        track_id: track,
                        start_us: 500000,
                        end_us: 1000000,
                        text: "world".into(),
                        speaker_keys: vec![],
                        confidence: None,
                    },
                ],
            },
        )
        .unwrap();
    storage
        .commit_analysis_candidate(
            &meeting,
            &id,
            storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
            false,
        )
        .unwrap();
    let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.create_transcript_draft(&meeting, &id).unwrap();
    let request = crate::storage::transcript_consolidation::Request {
        meeting_id: meeting.clone(),
        analysis_id: id.clone(),
        expected_content_hash: view.content_hash,
        expected_draft: Some(DraftStamp {
            id: draft.id,
            lock_version: 0,
        }),
        operation_id: Uuid::new_v4().to_string(),
    };
    let next = storage.update_speaker_assignments(&request).unwrap();
    assert_eq!(next.updated_count, 2);
    assert_eq!(next.analysis.utterances.len(), 2);
    let path = output.path().join("consolidated.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let restored = Storage::open(target.path().into()).unwrap();
    restored
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let restored_meeting = ids.get("meetings", "", &meeting).unwrap();
    let current = restored
        .load_current_analysis(&restored_meeting)
        .unwrap()
        .unwrap();
    assert_eq!(current.utterances.len(), 2);
    assert_eq!(current.utterances[0].effective_text, "Hello");
    assert!(current.utterances.iter().all(|u| u.speaker_ids.len() == 1));
    let db = restored.connection.lock().unwrap();
    let config: String = db
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            [&current.id],
            |r| r.get(0),
        )
        .unwrap();
    let config: Value = serde_json::from_str(&config).unwrap();
    assert!(config.get("speakerAssignmentUpdate").is_none());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM working_drafts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    restored
        .create_backup_archive(&output.path().join("second-assignment.monbackup"), &|| {
            false
        })
        .unwrap();
}

#[test]
fn voice_registry_round_trip_remaps_people_and_sources_without_merging_names() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let meeting = archive::tests::meeting(&storage, "voice-source");
    let person = Uuid::new_v4().to_string();
    let sample = Uuid::new_v4().to_string();
    let vector = crate::voice::encode(&[3., 4.]).unwrap();
    {
        let db = storage.connection.lock().unwrap();
        let (analysis,track):(String,String)=db.query_row("SELECT m.current_analysis_id,t.id FROM meetings m JOIN audio_tracks t ON t.meeting_id=m.id WHERE m.id=?1",[&meeting],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        db.execute("INSERT INTO voice_people VALUES(?1,'田中',1,0)", [&person])
            .unwrap();
        db.execute(
            "INSERT INTO voice_samples VALUES(?1,?2,?3,?4,2,?5,?6,?7,?8,0,1000000,0)",
            params![
                sample,
                person,
                "a".repeat(64),
                crate::voice::EXTRACTOR,
                vector,
                meeting,
                analysis,
                track
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO voice_sample_audio(id,state) VALUES(?1,'unavailable')",
            [&sample],
        )
        .unwrap();
    }
    let path = output.path().join("voices.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(&path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let fixed = FixedData::new(&ids, output.path(), &crate::test_support::never_cancelled).unwrap();
    let sealed =
        SealedWorkspace::prepare(&fixed, output.path(), &crate::test_support::never_cancelled)
            .unwrap();
    let destination = Storage::open(target.path().into()).unwrap();
    destination
        .connection
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO voice_people VALUES('destination-person','田中',1,0)",
            [],
        )
        .unwrap();
    destination
        .publish_restore(&sealed, &ids, &crate::test_support::never_cancelled)
        .unwrap();
    let restored = destination.voice_people().unwrap();
    assert_eq!(restored.len(), 2);
    let restored = restored
        .into_iter()
        .find(|p| !p.samples.is_empty())
        .unwrap();
    assert_eq!(restored.name, "田中");
    assert_ne!(restored.id, person);
    assert_ne!(restored.samples[0].id, sample);
    assert_ne!(
        restored.samples[0].source_meeting_id.as_ref().unwrap(),
        &meeting
    );
    assert!(restored.samples[0].source_available);
    let actual: Vec<u8> = destination
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT vector FROM voice_samples", [], |r| r.get(0))
        .unwrap();
    assert_eq!(actual, vector);
    // A subsequent backup must also accept global registry provenance.
    destination
        .create_backup_archive(
            &output.path().join("second.monbackup"),
            &crate::test_support::never_cancelled,
        )
        .unwrap();
}

#[test]
fn voice_registry_exports_samples_even_when_the_source_meeting_was_deleted() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("INSERT INTO voice_people VALUES('person','登録者',1,0)", [])
            .unwrap();
        db.execute("INSERT INTO voice_samples VALUES('sample','person',?1,?2,2,?3,'deleted-meeting','deleted-analysis','deleted-track',0,1000000,0)",params!["a".repeat(64),crate::voice::EXTRACTOR,crate::voice::encode(&[1.,0.]).unwrap()]).unwrap();
        db.execute(
            "INSERT INTO voice_sample_audio(id,state) VALUES('sample','unavailable')",
            [],
        )
        .unwrap();
    }
    let path = output.path().join("voices.monbackup");
    storage
        .create_backup_archive(&path, &crate::test_support::never_cancelled)
        .unwrap();
    let source = archive::read(
        &mut File::open(path).unwrap(),
        output.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let ids = RestoreIds::prepare(&source, output.path(), &Uuid::new_v4().to_string(), &|| {
        false
    })
    .unwrap();
    let row = ids.relational_row("voice_samples", 1).unwrap();
    assert_eq!(row[6], rusqlite::types::Value::Null);
    assert_eq!(row[7], rusqlite::types::Value::Null);
    assert_eq!(row[8], rusqlite::types::Value::Null);
}

#[test]
fn voice_registry_refuses_corrupt_features_before_export() {
    let root = tempfile::tempdir().unwrap();
    let storage = Storage::open(root.path().into()).unwrap();
    let output = tempfile::tempdir().unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("INSERT INTO voice_people VALUES('person','登録者',1,0)", [])
            .unwrap();
        db.execute("INSERT INTO voice_samples VALUES('sample','person',?1,?2,2,?3,NULL,NULL,NULL,0,1000000,0)",params!["a".repeat(64),crate::voice::EXTRACTOR,vec![0u8;8]]).unwrap();
        db.execute(
            "INSERT INTO voice_sample_audio(id,state) VALUES('sample','unavailable')",
            [],
        )
        .unwrap();
    }
    assert!(storage
        .create_backup_archive(
            &output.path().join("bad.monbackup"),
            &crate::test_support::never_cancelled
        )
        .is_err());
}

#[test]
fn current_speaker_person_and_voice_links_keep_shared_identity_and_match_states() {
    use crate::storage::voice::tests::{fixture, register};
    let (_root, s, meeting, speaker) = fixture();
    let person = register(&s, &meeting, &speaker);
    s.publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let view = s.load_current_analysis(&meeting).unwrap().unwrap();
    let speaker = view.speakers[0].id.clone();
    let mut draft = s.create_transcript_draft(&meeting, &view.id).unwrap();
    draft
        .payload
        .speaker_names
        .retain(|n| n.speaker_id != speaker);
    draft
        .payload
        .speaker_names
        .push(crate::storage::drafts::SpeakerNameEdit {
            speaker_id: speaker.clone(),
            display_name: Some("表示専用の名前".into()),
        });
    s.save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    let mut originals = vec![(meeting.clone(), speaker, "confirmed")];
    for state in ["candidate", "automatic", "dismissed"] {
        let m = archive::tests::meeting(&s, state);
        s.connection
            .lock()
            .unwrap()
            .execute("DELETE FROM working_drafts WHERE meeting_id=?1", [&m])
            .unwrap();
        let plan = s.canonical_audio_plan(&m).unwrap();
        let track = plan.tracks[0].id.clone();
        let a = s
            .create_analysis_candidate(
                &m,
                AnalysisInput {
                    audio_manifest_hash: plan.manifest_hash,
                    profile_snapshot: json!({}),
                    config: json!({}),
                    speakers: vec![SpeakerInput {
                        key: "a".into(),
                        label: "話者A".into(),
                    }],
                    segments: vec![SpeakerSegmentInput {
                        track_id: track.clone(),
                        speaker_key: Some("a".into()),
                        start_us: 0,
                        end_us: 1_000_000,
                        overlap_group: None,
                        confidence: None,
                    }],
                    utterances: vec![UtteranceInput {
                        track_id: track,
                        start_us: 0,
                        end_us: 1_000_000,
                        text: state.into(),
                        speaker_keys: vec!["a".into()],
                        confidence: None,
                    }],
                },
            )
            .unwrap();
        s.commit_analysis_candidate(
            &m,
            &a,
            s.get_meeting(&m).unwrap().unwrap().lock_version,
            false,
        )
        .unwrap();
        let sid = s.load_current_analysis(&m).unwrap().unwrap().speakers[0]
            .id
            .clone();
        s.connection.lock().unwrap().execute("INSERT INTO voice_matches(id,meeting_id,analysis_revision_id,speaker_id,person_id,name_snapshot,state,similarity,model_hash,policy_version,created_at) VALUES(?1,?2,?3,?4,?5,'田中',?6,0.9,?7,'test',0)",params![Uuid::new_v4().to_string(),m,a,sid,person,state,"a".repeat(64)]).unwrap();
        originals.push((m, sid, state));
    }
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("links.monbackup");
    s.create_backup_archive(&path, &|| false).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let existing = Uuid::new_v4().to_string();
    target
        .connection
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO voice_people VALUES(?1,'田中',1,0)",
            [&existing],
        )
        .unwrap();
    let selected = target
        .prepare_additive_restore(&path, &|| false, &|_| {})
        .unwrap();
    let preview = selected.preview();
    assert_eq!(preview.registry.people, 1);
    assert_eq!(preview.registry.links, 4);
    assert_eq!(preview.registry.audio, 1);
    let receipt = selected
        .publish(&target, &preview.confirmation_id, &|| false, &|_| {})
        .unwrap();
    assert_eq!(receipt.meeting_ids.len(), 4);
    let db = target.connection.lock().unwrap();
    let restored_person: String = db
        .query_row(
            "SELECT id FROM voice_people WHERE id!=?1",
            [existing],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(restored_person, person);
    for (m, sid, state) in originals {
        let (new_meeting,new_speaker,new_person,new_state):(String,String,String,String)=db.query_row("SELECT v.meeting_id,v.speaker_id,v.person_id,v.state FROM voice_matches v JOIN restore_identity_maps m ON m.target_key=v.meeting_id JOIN restore_identity_maps s ON s.target_key=v.speaker_id WHERE m.batch_id=?1 AND m.domain='meetings' AND m.source_key=?2 AND s.batch_id=?1 AND s.domain='speakers' AND s.source_key=?3",params![preview.batch_id,m,sid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_ne!(new_meeting, m);
        assert_ne!(new_speaker, sid);
        assert_eq!(new_person, restored_person);
        assert_eq!(new_state, state);
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM speaker_segments WHERE meeting_id=?1",
                [&new_meeting],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        if state == "confirmed" {
            assert_eq!(
                db.query_row(
                    "SELECT display_name FROM speakers WHERE id=?1",
                    [new_speaker],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "表示専用の名前"
            );
        }
    }
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM processing_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    assert_eq!(target.voice_people().unwrap().len(), 2);
    let replay = selected
        .publish(&target, &preview.confirmation_id, &|| false, &|_| {})
        .unwrap();
    assert!(replay.already_published);
    assert_eq!(target.voice_people().unwrap().len(), 2);
    target
        .create_backup_archive(&out.path().join("again.monbackup"), &|| false)
        .unwrap();
}

#[test]
fn deleted_person_is_not_recreated_from_registration_jobs_or_name_snapshots() {
    use crate::storage::voice::tests::{fixture, register};
    let (_root, s, meeting, speaker) = fixture();
    let person = register(&s, &meeting, &speaker);
    let version = s.voice_people().unwrap()[0].version;
    s.manage_voice_person(&person, version, None, None).unwrap();
    s.publish_single_wav(&meeting, &|| false, &|_, _| {})
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("deleted-person.monbackup");
    s.create_backup_archive(&path, &|| false).unwrap();
    let target_root = tempfile::tempdir().unwrap();
    let target = Storage::open(target_root.path().into()).unwrap();
    let prepared = target
        .prepare_additive_restore(&path, &|| false, &|_| {})
        .unwrap();
    let receipt = prepared
        .publish(
            &target,
            &prepared.preview().confirmation_id,
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert!(target.voice_people().unwrap().is_empty());
    let context = target.voice_context(&receipt.meeting_ids[0]).unwrap();
    assert_eq!(context.matches.len(), 1);
    assert!(context.matches[0].person_id.is_none());
    assert_eq!(context.matches[0].state, "confirmed");
}
