use super::*;
use crate::diarization::{self, sherpa::SherpaConfig, TrackSettings};
use crate::local_engine::Fingerprint;
use std::os::unix::fs::PermissionsExt;

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[test]
fn manual_split_ranges_text_original_scope_and_explicit_unknown_survive_reinference_candidate() {
    use super::super::draft_splits::{SplitPart, UtteranceSplit};
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, mut r) = fixture(root.path(), false);
    let original = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.load_transcript_draft(&m).unwrap().unwrap();
    let u = &original.utterances[0];
    let text = draft
        .payload
        .utterances
        .iter()
        .find(|e| e.utterance_id == u.id)
        .unwrap()
        .effective_text
        .clone();
    let chars: Vec<_> = text.chars().collect();
    let mut p = draft.payload.clone();
    p.schema_version = 3;
    p.utterance_splits.push(UtteranceSplit {
        source_utterance_id: u.id.clone(),
        source_effective_text: text.clone(),
        parts: vec![
            SplitPart {
                id: format!("split:{}", Uuid::new_v4()),
                start_us: u.start_us,
                end_us: 500_000,
                effective_text: chars[..3].iter().collect(),
                speaker_ids: vec![original.speakers[0].id.clone()],
            },
            SplitPart {
                id: format!("split:{}", Uuid::new_v4()),
                start_us: 500_000,
                end_us: u.end_us,
                effective_text: chars[3..].iter().collect(),
                speaker_ids: vec![],
            },
        ],
    });
    let saved = s
        .save_transcript_draft(&draft.id, draft.lock_version, p)
        .unwrap();
    r.expected_draft = Some(DraftStamp {
        id: saved.id.clone(),
        lock_version: saved.lock_version,
    });
    let job = enqueue(&s, r, c);
    let fixed = s.load_diarization_input(&job.id).unwrap();
    let parts: Vec<_> = fixed
        .analysis
        .utterances
        .iter()
        .filter(|u| u.recognition_is_source)
        .collect();
    assert_eq!(parts.len(), 2);
    assert!(parts
        .iter()
        .all(|u| fixed.manual_utterances.contains(&u.id)));
    s.acquire_local_inference_job().unwrap().unwrap();
    let candidate = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let result = s.load_visible_analysis(&m, &candidate).unwrap().unwrap();
    let parts: Vec<_> = result
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
    assert!(parts[1].speaker_ids.is_empty());
    assert_eq!(parts[1].attribution_state, "unknown");
    assert_eq!(parts[0].end_us, parts[1].start_us);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), original);
    assert_eq!(s.load_transcript_draft(&m).unwrap(), Some(saved));
}

#[test]
fn merging_into_added_speaker_survives_fixed_diarization_and_manual_preservation() {
    use super::super::drafts::{ManualSpeaker, SpeakerMerge};
    let root = tempfile::tempdir().unwrap();
    let (storage, meeting, config, mut request) = fixture(root.path(), false);
    let original = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.load_transcript_draft(&meeting).unwrap().unwrap();
    let mut payload = draft.payload.clone();
    payload.schema_version = 2;
    let id = format!("manual:{}", Uuid::new_v4());
    payload.manual_speakers.push(ManualSpeaker {
        id: id.clone(),
        label: "手動話者".into(),
    });
    payload.speaker_merges.push(SpeakerMerge {
        source_id: original.speakers[0].id.clone(),
        target_id: id.clone(),
    });
    let saved = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    request.expected_draft = Some(DraftStamp {
        id: saved.id.clone(),
        lock_version: saved.lock_version,
    });
    let job = enqueue(&storage, request, config);
    let input = storage.load_diarization_input(&job.id).unwrap();
    let added = input
        .analysis
        .speakers
        .iter()
        .find(|s| input.speaker_lineages[&s.id] == id)
        .unwrap();
    let utterance = input
        .analysis
        .utterances
        .iter()
        .find(|u| u.start_us == 0)
        .unwrap();
    assert_eq!(utterance.speaker_ids, vec![added.id.clone()]);
    assert!(input.manual_utterances.contains(&utterance.id));
    storage.acquire_local_inference_job().unwrap().unwrap();
    let candidate = diarization::process(&storage, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let result = storage
        .load_visible_analysis(&meeting, &candidate)
        .unwrap()
        .unwrap();
    let speaker = result
        .speakers
        .iter()
        .find(|s| s.label == "手動話者")
        .unwrap();
    let utterance = result.utterances.iter().find(|u| u.start_us == 0).unwrap();
    assert_eq!(utterance.speaker_ids, vec![speaker.id.clone()]);
    assert_eq!(utterance.effective_text, "保存した修正0");
    assert_eq!(
        storage.load_transcript_draft(&meeting).unwrap(),
        Some(saved)
    );
    assert_eq!(
        storage.load_current_analysis(&meeting).unwrap().unwrap(),
        original
    );
}
fn fixture(root: &Path, two: bool) -> (Storage, String, SherpaConfig, MinutesJobRequest) {
    let (storage, meeting, _) = super::super::transcription::tests::fixture(root, 10);
    if two {
        let relative = PathBuf::from("meetings").join(&meeting).join("audio");
        std::fs::copy(
            storage.managed_path(&relative.join("mic.caf")).unwrap(),
            storage.managed_path(&relative.join("system.caf")).unwrap(),
        )
        .unwrap();
        let asset = storage
            .register_ready_meeting_asset(&meeting, &relative.join("system.caf"), "audio/x-caf")
            .unwrap();
        let db = storage.connection.lock().unwrap();
        db.execute("INSERT INTO audio_tracks (id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) SELECT 'system-track',meeting_id,session_id,'system',format,sample_rate,channels,state,created_at,updated_at FROM audio_tracks WHERE id='track'",[]).unwrap();
        db.execute("INSERT INTO audio_chunks (id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) SELECT 'system-chunk',meeting_id,'system-track',?1,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at FROM audio_chunks WHERE id='chunk'",params![asset.id]).unwrap();
    }
    let source = storage.recorded_audio_plan(&meeting).unwrap();
    let mut utterances: Vec<_> = [
        (0, 1_000_000),
        (2_400_000, 3_200_000),
        (3_500_000, 5_500_000),
        (7_000_000, 8_000_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (start_us, end_us))| UtteranceInput {
        track_id: "track".into(),
        start_us,
        end_us,
        text: format!("元の認識本文{i}"),
        speaker_keys: if i == 3 {
            vec!["original".into()]
        } else {
            vec![]
        },
        confidence: Some(0.8),
    })
    .collect();
    if two {
        utterances.push(UtteranceInput {
            track_id: "system-track".into(),
            start_us: 0,
            end_us: 1_000_000,
            text: "PC音声の元本文".into(),
            speaker_keys: vec![],
            confidence: None,
        });
    }
    utterances.sort_by(|a, b| (a.start_us, &a.track_id).cmp(&(b.start_us, &b.track_id)));
    let current = storage
        .create_analysis_candidate(
            &meeting,
            AnalysisInput {
                audio_manifest_hash: source.manifest_hash,
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![SpeakerInput {
                    key: "original".into(),
                    label: "元の話者".into(),
                }],
                segments: vec![],
                utterances,
            },
        )
        .unwrap();
    storage
        .commit_analysis_candidate(&meeting, &current, 0, false)
        .unwrap();
    let original = storage.load_current_analysis(&meeting).unwrap().unwrap();
    let draft = storage.create_transcript_draft(&meeting, &current).unwrap();
    let prior = original.speakers[0].id.clone();
    let mut payload = draft.payload.clone();
    payload
        .speaker_names
        .push(super::super::drafts::SpeakerNameEdit {
            speaker_id: prior.clone(),
            display_name: Some("確定した氏名".into()),
        });
    for utterance in &original.utterances {
        if utterance.track_id != "track" {
            continue;
        }
        if [0, 2_400_000, 7_000_000].contains(&utterance.start_us) {
            payload
                .utterances
                .push(super::super::drafts::UtteranceEdit {
                    utterance_id: utterance.id.clone(),
                    effective_text: format!("保存した修正{}", utterance.start_us),
                    speaker_ids: if utterance.start_us == 0 {
                        vec![prior.clone()]
                    } else {
                        vec![]
                    },
                });
        }
    }
    let edited = storage
        .save_transcript_draft(&draft.id, draft.lock_version, payload)
        .unwrap();
    let request = MinutesJobRequest {
        meeting_id: meeting.clone(),
        operation_id: "diarization-start".into(),
        expected_analysis_id: original.id,
        expected_content_hash: original.content_hash,
        expected_meeting_lock_version: storage.get_meeting(&meeting).unwrap().unwrap().lock_version,
        expected_draft: Some(DraftStamp {
            id: edited.id,
            lock_version: edited.lock_version,
        }),
    };
    let segmentation = root.join("segmentation.onnx");
    let embedding = root.join("embedding.onnx");
    let cli = root.join("diarization-cli");
    std::fs::write(&segmentation, b"segmentation").unwrap();
    std::fs::write(&embedding, b"embedding").unwrap();
    std::fs::write(&cli, b"fixture").unwrap();
    let mut config = SherpaConfig {
        executable: Fingerprint::capture(&cli, &|| false).unwrap(),
        segmentation_model: Fingerprint::capture(&segmentation, &|| false).unwrap(),
        embedding_model: Fingerprint::capture(&embedding, &|| false).unwrap(),
        runtime_libraries: vec![],
        threads: 1,
        cluster_threshold_milli: 500,
        window_shift_milli: 100,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
    };
    let rows="0.000 -- 4.000 speaker_00\n2.000 -- 3.000 speaker_01\n5.000 -- 6.000 speaker_01\n7.000 -- 8.000 speaker_00\n";
    let auto = format!("{}\nStarted\n{rows}", config.expected_header(None));
    let fixed = format!("{}\nStarted\n{rows}", config.expected_header(Some(3)));
    std::fs::write(&cli,format!("#!/bin/sh\nprintf x >> {}\ncase \"$PWD\" in */unit-1-*) if test -f {}; then exit 42; fi;; esac\nfor arg in \"$@\"; do if test \"$arg\" = '--clustering.num-clusters=3'; then printf '%s' {};exit 0;fi;done\nprintf '%s' {}\n",quote(root.join("cli-calls").to_str().unwrap()),quote(root.join("fail-second").to_str().unwrap()),quote(&fixed),quote(&auto))).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    config.executable = Fingerprint::capture(&cli, &|| false).unwrap();
    (storage, meeting, config, request)
}
fn tracks(storage: &Storage, meeting: &str) -> Vec<TrackSettings> {
    storage
        .recorded_audio_plan(meeting)
        .unwrap()
        .tracks
        .iter()
        .map(|t| TrackSettings {
            track_id: t.id.clone(),
            num_speakers: None,
        })
        .collect()
}
fn enqueue(storage: &Storage, request: MinutesJobRequest, config: SherpaConfig) -> JobSnapshot {
    let settings = tracks(storage, &request.meeting_id);
    diarization::enqueue(storage, request, config, "standard", settings).unwrap()
}
fn calls(root: &Path) -> usize {
    std::fs::read(root.join("cli-calls")).map_or(0, |b| b.len())
}
fn pause_after_first(storage: &Storage, job: &str) {
    assert_eq!(
        storage.acquire_local_inference_job().unwrap().unwrap().id,
        job
    );
    assert_eq!(
        diarization::process(storage, job, &|| false, &|j| {
            if j.completed_units == 1 && j.state == "running" {
                storage.request_job_pause(&j.id, "user").unwrap();
            }
        })
        .unwrap(),
        None
    );
    assert_eq!(storage.get_job(job).unwrap().state, "paused");
}

#[test]
fn v16_migration_preserves_heads_drafts_vocabulary_and_fixed_jobs() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, _, _) = fixture(root.path(), false);
    let original = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.load_transcript_draft(&m).unwrap().unwrap();
    let entry = |term: &str| super::super::vocabulary::VocabularyEntry {
        id: Uuid::new_v4().to_string(),
        term: term.into(),
        reading: None,
        note: Some("移行前の登録".into()),
        enabled: true,
    };
    let common = s
        .save_vocabulary_book(None, 0, vec![entry("共通用語")])
        .unwrap();
    let meeting_vocabulary = s
        .save_vocabulary_book(Some(&m), 0, vec![entry("点検日")])
        .unwrap();
    let spec = JobSpec {
        meeting_id: m.clone(),
        kind: JobKind::Transcription,
        operation_id: "pre-migration".into(),
        input_analysis_id: None,
        input_hash: s.recorded_audio_plan(&m).unwrap().manifest_hash,
        profile_snapshot: json!({"profile":"standard"}),
        parameters: json!({"preMigration":true}),
        total_units: Some(1),
    };
    let job = s.enqueue_job(spec.clone()).unwrap();
    {
        let db = s.connection.lock().unwrap();
        super::imports::remove_import_schema_for_test(&db);
        db.execute_batch("DROP TABLE analysis_flows; DROP TABLE diarization_manual_assignments; DROP TRIGGER job_analysis_result_insert; DROP TRIGGER job_result_scope; DELETE FROM schema_migrations WHERE version>=17;").unwrap();
        let old_trigger = MIGRATION_9
            .split_once("CREATE TRIGGER job_result_scope")
            .unwrap()
            .1
            .split_once("CREATE TRIGGER job_result_fixed")
            .unwrap()
            .0;
        db.execute_batch(&format!("CREATE TRIGGER job_result_scope{old_trigger}"))
            .unwrap();
    }
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), original);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), draft);
    assert_eq!(
        serde_json::to_value(s.vocabulary_book(None).unwrap()).unwrap(),
        serde_json::to_value(common).unwrap()
    );
    assert_eq!(
        serde_json::to_value(s.vocabulary_book(Some(&m)).unwrap()).unwrap(),
        serde_json::to_value(meeting_vocabulary).unwrap()
    );
    assert_eq!(s.get_job(&job.id).unwrap().spec_hash, job.spec_hash);
    assert_eq!(s.fixed_job_input(&job.id).unwrap().0, spec);
    let db = s.connection.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        19
    );
    assert!(db
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap()
        .is_none());
}

#[test]
fn fixed_speaker_count_is_applied_to_each_explicit_track_without_a_quality_claim() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), true);
    let mut settings = tracks(&s, &m);
    for t in &mut settings {
        t.num_speakers = Some(3);
    }
    let job = diarization::enqueue(&s, r, c, "high_accuracy", settings.clone()).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let id = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(calls(root.path()), 2);
    let config = s
        .analysis_versions(&m)
        .unwrap()
        .versions
        .into_iter()
        .find(|v| v.id == id)
        .unwrap()
        .config;
    assert_eq!(config["tracks"], serde_json::to_value(settings).unwrap());
    assert_eq!(config["estimatedSpeakers"], json!([2, 2]));
    assert_eq!(config["timelineAlignment"], "unverified");
}

#[test]
fn fixed_edited_input_publishes_candidate_without_changing_current_or_draft() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), true);
    let original = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.load_transcript_draft(&m).unwrap().unwrap();
    let job = enqueue(&s, r.clone(), c.clone());
    assert_eq!(enqueue(&s, r, c).id, job.id);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), original);
    let fixed = s.load_diarization_input(&job.id).unwrap();
    assert_ne!(fixed.analysis.id, original.id);
    assert_eq!(fixed.manual_utterances.len(), 2);
    s.acquire_local_inference_job().unwrap().unwrap();
    let candidate = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let view = s.load_visible_analysis(&m, &candidate).unwrap().unwrap();
    assert_eq!(view.utterances.len(), original.utterances.len());
    for (new, old) in view.utterances.iter().zip(&original.utterances) {
        assert_eq!(new.recognized_text, old.recognized_text);
        assert_eq!((new.start_us, new.end_us), (old.start_us, old.end_us));
    }
    let find = |start| {
        view.utterances
            .iter()
            .find(|u| u.track_id == "track" && u.start_us == start)
            .unwrap()
    };
    assert_eq!(find(0).effective_text, "保存した修正0");
    let manual = &find(0).speaker_ids[0];
    assert_eq!(
        view.speakers
            .iter()
            .find(|s| &s.id == manual)
            .unwrap()
            .display_name
            .as_deref(),
        Some("確定した氏名")
    );
    assert_eq!(find(2_400_000).effective_text, "保存した修正2400000");
    assert_eq!(find(2_400_000).attribution_state, "overlap");
    assert_eq!(find(2_400_000).speaker_ids.len(), 2);
    assert_eq!(find(3_500_000).attribution_state, "unknown");
    assert_eq!(find(7_000_000).attribution_state, "unknown");
    let system = &view
        .utterances
        .iter()
        .find(|u| u.track_id == "system-track")
        .unwrap()
        .speaker_ids[0];
    assert_ne!(system, &find(2_400_000).speaker_ids[0]);
    assert_eq!(s.get_job(&job.id).unwrap().state, "succeeded");
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), original);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), draft);
    let db = s.connection.lock().unwrap();
    let preserved: i64 = db.query_row(
        "SELECT COUNT(*) FROM utterances u JOIN utterances old ON old.lineage_id=u.lineage_id WHERE u.analysis_revision_id=?1 AND old.analysis_revision_id=?2 AND u.confidence IS old.confidence",
        params![candidate,original.id],|r|r.get(0),
    ).unwrap();
    assert_eq!(preserved as usize, original.utterances.len());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM speaker_segments WHERE analysis_revision_id=?1",
            params![candidate],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        8
    );
    assert_eq!(
        db.query_row(
            "SELECT parent_revision_id FROM analysis_revisions WHERE id=?1",
            params![candidate],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        original.id
    );
    assert!(db
        .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
        .optional()
        .unwrap()
        .is_none());
}

#[test]
fn whole_track_checkpoint_reopens_and_runs_only_the_remaining_track() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), true);
    let job = enqueue(&s, r, c);
    let fixed = s.load_diarization_input(&job.id).unwrap().analysis;
    pause_after_first(&s, &job.id);
    assert_eq!(calls(root.path()), 1);
    let saved = s.fixed_job_input(&job.id).unwrap();
    drop(s);
    let s = Storage::open(root.path().into()).unwrap();
    assert_eq!(s.load_diarization_input(&job.id).unwrap().analysis, fixed);
    s.resume_job(&job.id).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    assert!(diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .is_some());
    assert_eq!(calls(root.path()), 2);
    assert_eq!(s.get_job(&job.id).unwrap().completed_units, 2);
    assert_eq!(
        s.fixed_job_input(&job.id).unwrap().1.unwrap()["outputs"][0],
        saved.1.unwrap()["outputs"][0]
    );
    assert!(s.load_transcript_draft(&m).unwrap().is_some());
}

#[test]
fn retry_inherits_verified_tracks_and_the_original_input_despite_later_edits() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), true);
    let job = enqueue(&s, r, c);
    std::fs::write(root.path().join("fail-second"), b"fail").unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let failure = diarization::process(&s, &job.id, &|| false, &|_| {}).unwrap_err();
    assert!(failure.contains("sherpa-onnxが失敗"), "{failure}");
    let failed = s
        .finish_job(&job.id, 1, "failed", Some("engine_failed"))
        .unwrap();
    let mut draft = s.load_transcript_draft(&m).unwrap().unwrap();
    draft.payload.utterances[0].effective_text = "開始後の編集".into();
    let edited = s
        .save_transcript_draft(&draft.id, draft.lock_version, draft.payload)
        .unwrap();
    let fixed = s.load_diarization_input(&job.id).unwrap().analysis;
    std::fs::remove_file(root.path().join("fail-second")).unwrap();
    let request = || diarization::RetryRequest {
        meeting_id: m.clone(),
        job_id: job.id.clone(),
        operation_id: "retry".into(),
        expected_spec_hash: failed.spec_hash.clone(),
    };
    let retry = request().enqueue(&s).unwrap();
    assert_eq!(request().enqueue(&s).unwrap().id, retry.id);
    assert_eq!(retry.completed_units, 1);
    assert_eq!(s.load_diarization_input(&retry.id).unwrap().analysis, fixed);
    assert_eq!(calls(root.path()), 2);
    s.acquire_local_inference_job().unwrap().unwrap();
    let id = diarization::process(&s, &retry.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(calls(root.path()), 3);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), edited);
    let view = s.load_visible_analysis(&m, &id).unwrap().unwrap();
    assert_eq!(
        view.utterances
            .iter()
            .find(|u| u.track_id == "track" && u.start_us == 2_400_000)
            .unwrap()
            .effective_text,
        "保存した修正2400000"
    );
}

#[test]
fn source_models_and_saved_artifacts_are_checked_before_more_inference_or_retry() {
    for target in ["model", "source", "wav", "stdout", "result"] {
        let root = tempfile::tempdir().unwrap();
        let (s, m, c, r) = fixture(root.path(), true);
        let job = enqueue(&s, r, c.clone());
        pause_after_first(&s, &job.id);
        let checkpoint = s.fixed_job_input(&job.id).unwrap().1.unwrap();
        let path = if target == "model" {
            c.embedding_model.path.clone()
        } else {
            let id = if target == "source" {
                s.recorded_audio_plan(&m).unwrap().tracks[0].chunks[0]
                    .asset_id
                    .clone()
            } else {
                checkpoint["outputs"][0][format!("{target}AssetId")]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            let relative: String = s
                .connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT relative_path FROM file_assets WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .unwrap();
            s.managed_path(Path::new(&relative)).unwrap()
        };
        std::fs::write(path, b"changed").unwrap();
        s.resume_job(&job.id).unwrap();
        s.acquire_local_inference_job().unwrap().unwrap();
        assert!(
            diarization::process(&s, &job.id, &|| false, &|_| {}).is_err(),
            "accepted {target}"
        );
        assert_eq!(calls(root.path()), 1);
        let failed = s.finish_job(&job.id, 1, "failed", Some("changed")).unwrap();
        assert!(diarization::RetryRequest {
            meeting_id: m,
            job_id: job.id,
            operation_id: "retry".into(),
            expected_spec_hash: failed.spec_hash
        }
        .enqueue(&s)
        .is_err());
        assert_eq!(calls(root.path()), 1);
    }
}

#[test]
fn fixed_snapshot_enqueue_and_candidate_success_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), false);
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_diarization_enqueue BEFORE INSERT ON processing_jobs WHEN NEW.kind='diarization' BEGIN SELECT RAISE(ABORT,'enqueue failed');END;").unwrap();
    assert!(diarization::enqueue(&s, r.clone(), c.clone(), "standard", tracks(&s, &m)).is_err());
    assert_eq!(
        s.connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM analysis_input_snapshots", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    s.connection.lock().unwrap().execute_batch("DROP TRIGGER fail_diarization_enqueue; CREATE TRIGGER fail_diarization_finish BEFORE UPDATE ON processing_jobs WHEN NEW.kind='diarization' AND NEW.state='succeeded' BEGIN SELECT RAISE(ABORT,'finish failed');END;").unwrap();
    let job = enqueue(&s, r, c);
    s.acquire_local_inference_job().unwrap().unwrap();
    let failure = diarization::process(&s, &job.id, &|| false, &|_| {}).unwrap_err();
    assert!(failure.contains("finish failed"), "{failure}");
    assert_eq!(s.get_job(&job.id).unwrap().state, "running");
    assert_eq!(s.get_job(&job.id).unwrap().output_analysis_id, None);
    assert_eq!(s.get_job(&job.id).unwrap().completed_units, 1);
    assert_eq!(
        s.connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM analysis_revisions WHERE publication_state='candidate'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let count = calls(root.path());
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_diarization_finish;")
        .unwrap();
    assert!(diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .is_some());
    assert_eq!(calls(root.path()), count);
}

#[test]
fn stale_start_wrong_tracks_and_sql_result_redirection_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), false);
    for change in [
        "meeting",
        "analysis",
        "hash",
        "draft-id",
        "draft-version",
        "no-draft",
    ] {
        let mut stale = r.clone();
        match change {
            "meeting" => stale.expected_meeting_lock_version += 1,
            "analysis" => stale.expected_analysis_id = "another-analysis".into(),
            "hash" => stale.expected_content_hash = "f".repeat(64),
            "draft-id" => stale.expected_draft.as_mut().unwrap().id = "another-draft".into(),
            "draft-version" => stale.expected_draft.as_mut().unwrap().lock_version += 1,
            "no-draft" => stale.expected_draft = None,
            _ => unreachable!(),
        }
        assert!(
            diarization::enqueue(&s, stale, c.clone(), "standard", tracks(&s, &m)).is_err(),
            "{change}"
        );
    }
    for count in [0, 101] {
        let mut settings = tracks(&s, &m);
        settings[0].num_speakers = Some(count);
        assert!(diarization::enqueue(&s, r.clone(), c.clone(), "standard", settings).is_err());
    }
    assert!(diarization::enqueue(
        &s,
        r.clone(),
        c.clone(),
        "standard",
        vec![TrackSettings {
            track_id: "other".into(),
            num_speakers: Some(3)
        }]
    )
    .is_err());
    assert_eq!(
        s.connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM processing_jobs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM analysis_input_snapshots", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    let job = enqueue(&s, r.clone(), c);
    let input = s.load_diarization_input(&job.id).unwrap();
    let db = s.connection.lock().unwrap();
    assert!(db
        .execute(
            "UPDATE processing_jobs SET state='succeeded',output_analysis_id=?1 WHERE id=?2",
            params![r.expected_analysis_id, job.id]
        )
        .is_err());
    let nonmanual = input
        .analysis
        .utterances
        .iter()
        .find(|u| !input.manual_utterances.contains(&u.id))
        .unwrap();
    assert!(db
        .execute(
            "INSERT INTO diarization_manual_assignments (analysis_id,utterance_id) VALUES (?1,?2)",
            params![input.analysis.id, nonmanual.id]
        )
        .is_err());
    assert!(db
        .execute(
            "DELETE FROM diarization_manual_assignments WHERE analysis_id=?1",
            params![input.analysis.id]
        )
        .is_err());
}

fn current_request(s: &Storage, m: &str, operation: &str) -> MinutesJobRequest {
    let view = s.load_current_analysis(m).unwrap().unwrap();
    let state = s.analysis_versions(m).unwrap();
    MinutesJobRequest {
        meeting_id: m.into(),
        operation_id: operation.into(),
        expected_analysis_id: view.id,
        expected_content_hash: view.content_hash,
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: state.draft,
    }
}

#[test]
fn speaker_review_distinguishes_raw_overlap_turns_and_manual_assignment_and_plays_candidate() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), true);
    let job = enqueue(&s, r, c);
    s.acquire_local_inference_job().unwrap().unwrap();
    let id = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let view = s.load_visible_analysis(&m, &id).unwrap().unwrap();
    let mut request = SpeakerReviewRequest {
        meeting_id: m.clone(),
        analysis_id: id.clone(),
        expected_content_hash: view.content_hash.clone(),
        utterance_ids: view.utterances.iter().map(|u| u.id.clone()).collect(),
        include_intervals: false,
        page: 0,
    };
    let batch = s.speaker_reviews(request.clone()).unwrap();
    assert_eq!(batch.reviews.len(), view.utterances.len());
    for (u, review) in view.utterances.iter().zip(&batch.reviews) {
        assert!(review.segments.is_empty());
        assert!(review.overlaps.is_empty());
        if u.track_id == "track" {
            match u.start_us {
                0 => assert!(review.manual_assignment),
                2_400_000 => {
                    assert_eq!(review.estimated_state, "overlap");
                    assert_eq!(review.overlap_count, 1);
                }
                3_500_000 => {
                    assert_eq!(review.estimated_state, "needsSplit");
                    assert_eq!(review.overlap_count, 0);
                }
                7_000_000 => {
                    assert!(review.manual_assignment);
                    assert_eq!(review.estimated_state, "assigned");
                    assert!(u.speaker_ids.is_empty());
                }
                _ => unreachable!(),
            }
        }
    }
    let u = view
        .utterances
        .iter()
        .find(|u| u.track_id == "track" && u.start_us == 2_400_000)
        .unwrap();
    request.utterance_ids = vec![u.id.clone()];
    request.include_intervals = true;
    let detail = s
        .speaker_reviews(request.clone())
        .unwrap()
        .reviews
        .remove(0);
    assert_eq!(
        (detail.overlaps[0].start_us, detail.overlaps[0].end_us),
        (2_400_000, 3_000_000)
    );
    assert!(detail.segments.iter().all(|s| s.confidence.is_none()));
    let audio = s.prepare_utterance_audio(&m, &id, &u.id).unwrap();
    assert_eq!(audio.total_frames(), 12_800);
    for change in [
        "hash",
        "meeting",
        "analysis",
        "utterance",
        "duplicate",
        "batch-details",
        "page",
    ] {
        let mut bad = request.clone();
        match change {
            "hash" => bad.expected_content_hash = "f".repeat(64),
            "meeting" => bad.meeting_id = "other".into(),
            "analysis" => bad.analysis_id = "other".into(),
            "utterance" => bad.utterance_ids = vec!["other".into()],
            "duplicate" => bad.utterance_ids.push(u.id.clone()),
            "batch-details" => {
                bad.utterance_ids = view.utterances.iter().map(|u| u.id.clone()).collect()
            }
            "page" => bad.page = 1,
            _ => unreachable!(),
        }
        assert!(s.speaker_reviews(bad).is_err(), "{change}");
    }
    let current = s.load_current_analysis(&m).unwrap().unwrap();
    request.analysis_id = current.id;
    request.expected_content_hash = current.content_hash;
    assert!(s.speaker_reviews(request).is_err());
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER segments_frozen_update;")
        .unwrap();
    s.connection
        .lock()
        .unwrap()
        .execute(
            "UPDATE speaker_segments SET end_us=end_us+1 WHERE analysis_revision_id=?1",
            params![id],
        )
        .unwrap();
    let bad = SpeakerReviewRequest {
        meeting_id: m.clone(),
        analysis_id: id.clone(),
        expected_content_hash: view.content_hash,
        utterance_ids: vec![u.id.clone()],
        include_intervals: true,
        page: 0,
    };
    assert!(s.speaker_reviews(bad).err().unwrap().contains("ハッシュ"));
    assert!(s
        .prepare_utterance_audio(&m, &id, &u.id)
        .err()
        .unwrap()
        .contains("変更"));
}

#[test]
fn speaker_review_pages_do_not_confuse_same_speaker_duplicates_with_more_people() {
    let root = tempfile::tempdir().unwrap();
    let s = Storage::open(root.path().into()).unwrap();
    let (m, _, _) = super::super::playback::tests::fixture(&s);
    let id = s
        .create_analysis_candidate(
            &m,
            AnalysisInput {
                audio_manifest_hash: "a".repeat(64),
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![SpeakerInput {
                    key: "one".into(),
                    label: "一人".into(),
                }],
                segments: (0..125)
                    .map(|i| SpeakerSegmentInput {
                        track_id: "track".into(),
                        speaker_key: Some("one".into()),
                        start_us: 250_000 + i * 1000,
                        end_us: 255_000 + i * 1000,
                        confidence: None,
                        overlap_group: None,
                    })
                    .collect(),
                utterances: vec![UtteranceInput {
                    track_id: "track".into(),
                    start_us: 250_000,
                    end_us: 750_000,
                    text: "一人の発言".into(),
                    speaker_keys: vec![],
                    confidence: None,
                }],
            },
        )
        .unwrap();
    let view = s.load_visible_analysis(&m, &id).unwrap().unwrap();
    let mut r = SpeakerReviewRequest {
        meeting_id: m,
        analysis_id: id,
        expected_content_hash: view.content_hash,
        utterance_ids: vec![view.utterances[0].id.clone()],
        include_intervals: true,
        page: 0,
    };
    for (page, count) in [(0, 50), (1, 50), (2, 25)] {
        r.page = page;
        let review = s.speaker_reviews(r.clone()).unwrap().reviews.remove(0);
        assert_eq!(review.segment_count, 125);
        assert_eq!(review.page_count, 3);
        assert_eq!(review.segments.len(), count);
        assert_eq!(review.overlap_count, 0);
        assert_eq!(review.estimated_state, "assigned");
    }
    r.page = 3;
    assert!(s.speaker_reviews(r).is_err());
}

#[test]
fn speaker_review_and_playback_reject_internal_or_no_longer_visible_revisions() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), false);
    let old = s.load_current_analysis(&m).unwrap().unwrap();
    let job = enqueue(&s, r, c.clone());
    let fixed = s.load_diarization_input(&job.id).unwrap().analysis;
    let request = |a: &AnalysisView| SpeakerReviewRequest {
        meeting_id: m.clone(),
        analysis_id: a.id.clone(),
        expected_content_hash: a.content_hash.clone(),
        utterance_ids: vec![a.utterances[0].id.clone()],
        include_intervals: false,
        page: 0,
    };
    assert!(s.speaker_reviews(request(&fixed)).is_err());
    assert!(s
        .prepare_utterance_audio(&m, &fixed.id, &fixed.utterances[0].id)
        .is_err());
    s.acquire_local_inference_job().unwrap().unwrap();
    let first = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    adopt(&s, &m, &first).unwrap();
    assert!(s.speaker_reviews(request(&old)).is_ok());
    let next = enqueue(&s, current_request(&s, &m, "next"), c);
    s.acquire_local_inference_job().unwrap().unwrap();
    let second = diarization::process(&s, &next.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    adopt(&s, &m, &second).unwrap();
    assert!(s.speaker_reviews(request(&old)).is_err());
    assert!(s
        .prepare_utterance_audio(&m, &old.id, &old.utterances[0].id)
        .is_err());
}
fn adopt(s: &Storage, m: &str, id: &str) -> Result<CommittedAnalysis, String> {
    let view = s.load_visible_analysis(m, id)?.unwrap();
    let state = s.analysis_versions(m)?;
    s.adopt_analysis(AdoptRequest {
        meeting_id: m.into(),
        revision_id: id.into(),
        expected_content_hash: view.content_hash,
        expected_meeting_lock_version: state.meeting_lock_version,
        expected_draft: state.draft,
        confirm_replacing_previous: true,
    })
}

#[test]
fn explicit_unknown_assignment_and_names_survive_adoption_and_next_diarization() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), false);
    let original = r.expected_analysis_id.clone();
    let job = enqueue(&s, r, c.clone());
    s.acquire_local_inference_job().unwrap().unwrap();
    let id = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    adopt(&s, &m, &id).unwrap();
    assert_eq!(
        s.analysis_versions(&m).unwrap().previous_id,
        Some(original.clone())
    );
    let archived = s.visible_analysis_draft(&m, &original).unwrap().unwrap();
    assert_eq!(archived.payload.utterances.len(), 3);
    let next = enqueue(&s, current_request(&s, &m, "next-analysis"), c);
    assert_eq!(
        s.load_diarization_input(&next.id)
            .unwrap()
            .manual_utterances
            .len(),
        2
    );
    s.acquire_local_inference_job().unwrap().unwrap();
    let candidate = diarization::process(&s, &next.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    let view = s.load_visible_analysis(&m, &candidate).unwrap().unwrap();
    let unknown = view
        .utterances
        .iter()
        .find(|u| u.start_us == 7_000_000)
        .unwrap();
    assert_eq!(unknown.attribution_state, "unknown");
    let manual = view.utterances.iter().find(|u| u.start_us == 0).unwrap();
    assert_eq!(manual.effective_text, "保存した修正0");
    assert_eq!(
        view.speakers
            .iter()
            .find(|s| s.id == manual.speaker_ids[0])
            .unwrap()
            .display_name
            .as_deref(),
        Some("確定した氏名")
    );
    assert_eq!(
        s.visible_analysis_draft(&m, &original).unwrap().unwrap(),
        archived
    );
}

#[test]
fn a_changed_current_head_does_not_reparent_or_silently_adopt_the_old_input_result() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), false);
    let old = r.expected_analysis_id.clone();
    let job = enqueue(&s, r, c);
    let source = s.load_current_analysis(&m).unwrap().unwrap();
    let manifest = s.recorded_audio_plan(&m).unwrap().manifest_hash;
    let replacement = s
        .create_analysis_candidate(
            &m,
            AnalysisInput {
                audio_manifest_hash: manifest,
                profile_snapshot: json!({}),
                config: json!({}),
                speakers: vec![],
                segments: vec![],
                utterances: source
                    .utterances
                    .iter()
                    .map(|u| UtteranceInput {
                        track_id: u.track_id.clone(),
                        start_us: u.start_us,
                        end_us: u.end_us,
                        text: u.recognized_text.clone(),
                        speaker_keys: vec![],
                        confidence: None,
                    })
                    .collect(),
            },
        )
        .unwrap();
    adopt(&s, &m, &replacement).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    let id = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(
        s.connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT parent_revision_id FROM analysis_revisions WHERE id=?1",
                params![id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        old
    );
    assert!(adopt(&s, &m, &id).is_err());
    assert_eq!(
        s.analysis_versions(&m).unwrap().current_id,
        Some(replacement)
    );
}

#[test]
fn a_validly_hashed_but_wrong_result_origin_is_rejected_before_more_inference() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), true);
    let job = enqueue(&s, r, c);
    pause_after_first(&s, &job.id);
    let mut checkpoint = s.fixed_job_input(&job.id).unwrap().1.unwrap();
    let output = &checkpoint["outputs"][0];
    let bytes = s
        .read_analysis_result(
            &m,
            output["resultAssetId"].as_str().unwrap(),
            output["resultHash"].as_str().unwrap(),
        )
        .unwrap();
    let mut result: Value = serde_json::from_slice(&bytes).unwrap();
    result["originOperationId"] = json!("another-operation");
    let directory = s.analysis_unit_directory(&m, &job.id, 0).unwrap();
    let path = directory.join("wrong-result.json");
    std::fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
    let asset = s
        .register_analysis_artifact(&m, &path, "application/json")
        .unwrap();
    checkpoint["outputs"][0]["resultAssetId"] = json!(asset.id);
    checkpoint["outputs"][0]["resultHash"] = json!(asset.content_hash);
    s.resume_job(&job.id).unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    s.save_job_checkpoint(&job.id, 1, 1, checkpoint).unwrap();
    assert!(diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap_err()
        .contains("由来"));
    assert_eq!(calls(root.path()), 1);
}

#[test]
fn recording_and_exit_pause_kill_and_reap_unfinished_whole_track_without_a_checkpoint() {
    for recording in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (s, _, mut c, r) = fixture(root.path(), false);
        std::fs::write(
            &c.executable.path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' $$ > {}\nexec /bin/sleep 30\n",
                quote(root.path().join("child.pid").to_str().unwrap())
            ),
        )
        .unwrap();
        c.executable = Fingerprint::capture(&c.executable.path, &|| false).unwrap();
        let job = enqueue(&s, r, c);
        s.acquire_local_inference_job().unwrap().unwrap();
        let started = std::time::Instant::now();
        let requested_at = std::cell::Cell::new(None::<std::time::Instant>);
        let requested = std::cell::Cell::new(false);
        assert_eq!(
            diarization::process(
                &s,
                &job.id,
                &|| {
                    // Wait for a complete PID: under the parallel full suite,
                    // hashing/conversion can legitimately exceed 250 ms.
                    let child_started = std::fs::read(root.path().join("child.pid"))
                        .is_ok_and(|pid| pid.ends_with(b"\n") && pid.len() > 1);
                    if child_started || started.elapsed() > std::time::Duration::from_secs(10) {
                        if requested_at.get().is_none() {
                            requested_at.set(Some(std::time::Instant::now()));
                        }
                        if recording {
                            if !requested.replace(true) {
                                s.request_pause_for_recording().unwrap();
                            }
                            return false;
                        }
                        return true;
                    }
                    false
                },
                &|_| {}
            )
            .unwrap(),
            None
        );
        let stopped = s.get_job(&job.id).unwrap();
        assert_eq!(stopped.state, "paused");
        assert_eq!(stopped.completed_units, 0);
        assert_eq!(
            stopped.pause_reason.as_deref(),
            Some(if recording { "recording" } else { "exit" })
        );
        assert!(requested_at.get().unwrap().elapsed() < std::time::Duration::from_secs(3));
        let pid = std::fs::read_to_string(root.path().join("child.pid"))
            .expect("native test CLI must start before testing its cancellation");
        assert!(!std::process::Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success());
    }
}

#[test]
fn failed_checkpoint_commit_does_not_reuse_an_uncommitted_native_result() {
    let root = tempfile::tempdir().unwrap();
    let (s, m, c, r) = fixture(root.path(), false);
    let job = enqueue(&s, r, c);
    s.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_diarization_checkpoint BEFORE INSERT ON job_checkpoints WHEN NEW.completed_units>0 BEGIN SELECT RAISE(ABORT,'checkpoint failed');END;").unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    assert!(diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap_err()
        .contains("checkpoint failed"));
    assert_eq!(s.get_job(&job.id).unwrap().completed_units, 0);
    assert_eq!(calls(root.path()), 1);
    let failed = s
        .finish_job(&job.id, 0, "failed", Some("checkpoint_failed"))
        .unwrap();
    s.connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_diarization_checkpoint;")
        .unwrap();
    let retry = diarization::RetryRequest {
        meeting_id: m,
        job_id: job.id,
        operation_id: "retry".into(),
        expected_spec_hash: failed.spec_hash,
    }
    .enqueue(&s)
    .unwrap();
    s.acquire_local_inference_job().unwrap().unwrap();
    assert!(diarization::process(&s, &retry.id, &|| false, &|_| {})
        .unwrap()
        .is_some());
    assert_eq!(calls(root.path()), 2);
}

#[test]
#[ignore = "requires MON_SHERPA_CLI, MON_SHERPA_SEGMENTATION, MON_SHERPA_EMBEDDING, MON_SHERPA_INPUT_CAF"]
fn real_sherpa_worker_commits_checked_tracks_and_a_candidate_without_replacing_edits() {
    use crate::audio::VerifiedAudio;
    let root = tempfile::tempdir().unwrap();
    let (s, m, _, r) = fixture(root.path(), false);
    let old = s.load_current_analysis(&m).unwrap().unwrap();
    let draft = s.load_transcript_draft(&m).unwrap().unwrap();
    let input = PathBuf::from(std::env::var("MON_SHERPA_INPUT_CAF").unwrap());
    let relative = PathBuf::from("meetings").join(&m).join("audio/real.caf");
    let path = s.managed_path(&relative).unwrap();
    std::fs::copy(&input, &path).unwrap();
    let asset = s
        .register_ready_meeting_asset(&m, &relative, "audio/x-caf")
        .unwrap();
    let verified = VerifiedAudio::open(
        std::fs::File::open(&path).unwrap(),
        asset.size_bytes as u64,
        &asset.content_hash,
    )
    .unwrap();
    let format = verified.format;
    let duration = (format.frames * 1_000_000 / u64::from(format.sample_rate)) as i64;
    assert!(duration >= 10_000_000);
    {
        let db = s.connection.lock().unwrap();
        db.execute(
            "UPDATE audio_tracks SET sample_rate=?1,channels=?2 WHERE id='track'",
            params![format.sample_rate, format.channels],
        )
        .unwrap();
        db.execute("UPDATE audio_chunks SET file_asset_id=?1,end_us=?2,frame_count=?3,sample_rate=?4,channels=?5 WHERE id='chunk'",params![asset.id,duration,i64::try_from(format.frames).unwrap(),format.sample_rate,format.channels]).unwrap();
    }
    let runtime = std::env::var("MON_SHERPA_RUNTIME_LIBRARY")
        .ok()
        .map(PathBuf::from)
        .into_iter()
        .collect();
    let job = diarization::StartRequest {
        input: r,
        executable_path: PathBuf::from(std::env::var("MON_SHERPA_CLI").unwrap()),
        segmentation_model_path: PathBuf::from(std::env::var("MON_SHERPA_SEGMENTATION").unwrap()),
        embedding_model_path: PathBuf::from(std::env::var("MON_SHERPA_EMBEDDING").unwrap()),
        runtime_library_paths: runtime,
        threads: 2,
        profile: "standard".into(),
        tracks: tracks(&s, &m),
        cluster_threshold_milli: 500,
        window_shift_milli: 100,
        min_duration_on_ms: 0,
        min_duration_off_ms: 0,
    }
    .enqueue(&s)
    .unwrap();
    let fixed = s.load_diarization_input(&job.id).unwrap().analysis;
    s.acquire_local_inference_job().unwrap().unwrap();
    let started = std::time::Instant::now();
    let id = diarization::process(&s, &job.id, &|| false, &|_| {})
        .unwrap()
        .unwrap();
    assert_eq!(s.get_job(&job.id).unwrap().state, "succeeded");
    assert_eq!(s.get_job(&job.id).unwrap().completed_units, 1);
    assert_eq!(s.load_current_analysis(&m).unwrap().unwrap(), old);
    assert_eq!(s.load_transcript_draft(&m).unwrap().unwrap(), draft);
    let view = s.load_visible_analysis(&m, &id).unwrap().unwrap();
    assert_eq!(view.utterances.len(), old.utterances.len());
    for (candidate, input) in view.utterances.iter().zip(&fixed.utterances) {
        assert_eq!(candidate.track_id, input.track_id);
        assert_eq!(candidate.start_us, input.start_us);
        assert_eq!(candidate.end_us, input.end_us);
        assert_eq!(candidate.recognized_text, input.recognized_text);
        assert_eq!(candidate.effective_text, input.effective_text);
    }
    let count: i64 = s
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM speaker_segments WHERE analysis_revision_id=?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(count > 0);
    eprintln!("real sherpa worker: {} ms, {} raw intervals; synthetic transcript is for storage binding, not accuracy",started.elapsed().as_millis(),count);
}
