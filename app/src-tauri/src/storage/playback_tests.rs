use super::*;
use crate::playback::{PlaybackPreparations, PlaybackStore};
use crate::storage::{AnalysisInput, UtteranceInput};

pub(in crate::storage) fn fixture(storage: &Storage) -> (String, String, String) {
    let meeting = storage.create_meeting("再生テスト").unwrap();
    let relative = PathBuf::from("meetings").join(&meeting.id).join("audio");
    let directory = storage.managed_path(&relative).unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("manifest.json"), b"{\"state\":\"saved\"}").unwrap();
    let manifest = storage
        .register_ready_meeting_asset(
            &meeting.id,
            &relative.join("manifest.json"),
            "application/json",
        )
        .unwrap();
    let mut bytes = crate::audio::wave::header(16000).unwrap();
    for i in 0..16000 {
        bytes.extend_from_slice(&(if i < 8000 { 8192i16 } else { -16384i16 }).to_le_bytes());
    }
    std::fs::write(directory.join("meeting.wav"), bytes).unwrap();
    let asset = storage
        .register_ready_meeting_asset(&meeting.id, &relative.join("meeting.wav"), "audio/wav")
        .unwrap();
    {
        let db = storage.connection.lock().unwrap();
        db.execute("UPDATE meetings SET source_type='recording',lifecycle='recorded',duration_us=1000000 WHERE id=?1",[&meeting.id]).unwrap();
        db.execute("INSERT INTO recording_sessions(id,meeting_id,state,capture_mode,capture_config_json,start_operation_id,audio_manifest_hash,duration_us,created_at,updated_at) VALUES('session',?1,'saved','system','{}','start',?2,1000000,0,0)",params![meeting.id,manifest.content_hash]).unwrap();
        db.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,created_at,updated_at) VALUES('track',?1,'session','processed','wav',16000,1,'ready',0,0)",[&meeting.id]).unwrap();
        db.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES('chunk',?1,'track',?2,0,0,1000000,16000,16000,1,'ready',0)",params![meeting.id,asset.id]).unwrap();
        db.execute("INSERT INTO meeting_audio_commits VALUES(?1,'track',?2,?3,16000,'{}','[]','cleaned',0)",params![meeting.id,manifest.id,manifest.content_hash]).unwrap();
    }
    let candidate = storage
        .create_analysis_candidate(
            &meeting.id,
            AnalysisInput {
                audio_manifest_hash: manifest.content_hash,
                profile_snapshot: serde_json::json!({}),
                config: serde_json::json!({}),
                speakers: vec![],
                segments: vec![],
                utterances: vec![UtteranceInput {
                    track_id: "track".into(),
                    start_us: 250000,
                    end_us: 750000,
                    text: "発言".into(),
                    speaker_keys: vec![],
                    confidence: None,
                }],
            },
        )
        .unwrap();
    storage
        .commit_analysis_candidate(&meeting.id, &candidate, meeting.lock_version, false)
        .unwrap();
    let utterance = storage
        .load_current_analysis(&meeting.id)
        .unwrap()
        .unwrap()
        .utterances[0]
        .id
        .clone();
    (meeting.id, candidate, utterance)
}

#[test]
fn canonical_playback_is_version_scoped_and_reads_the_requested_timeline() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, analysis, utterance) = fixture(&storage);
    assert!(storage
        .prepare_utterance_audio("other", &analysis, &utterance)
        .is_err());
    assert!(storage
        .prepare_utterance_audio(&meeting, "other", &utterance)
        .is_err());
    let store = PlaybackStore::default();
    let info = store
        .insert(
            storage
                .prepare_utterance_audio(&meeting, &analysis, &utterance)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(info.total_frames, 8000);
    assert_eq!(info.sample_rate, 16000);
    let pcm = store.read(&info.token, 0).unwrap();
    for (i, value) in pcm[12..].chunks_exact(4).enumerate() {
        assert_eq!(
            f32::from_le_bytes(value.try_into().unwrap()),
            if i < 4000 { 0.25 } else { -0.5 }
        );
    }
    assert!(store.read(&info.token, 8000).is_err());
    store.close(&info.token).unwrap();
    assert!(store.read(&info.token, 0).is_err());
}

#[test]
fn cancellation_and_revoked_utterance_never_publish_audio() {
    for revoke in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().into()).unwrap();
        let (meeting, analysis, utterance) = fixture(&storage);
        let checks = std::cell::Cell::new(0);
        let result=storage.prepare_utterance_audio_cancellable(&meeting,&analysis,&utterance,&|| {
            checks.set(checks.get()+1);
            if checks.get()==4 && revoke {storage.connection.lock().unwrap().execute("UPDATE meetings SET current_analysis_id=NULL,previous_analysis_id=NULL WHERE id=?1",[&meeting]).unwrap();}
            !revoke && checks.get()>=4
        });
        assert!(result.is_err());
    }
}

#[test]
fn lost_prepare_reply_can_close_native_playback_by_preparation_id() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, analysis, utterance) = fixture(&storage);
    let store = PlaybackStore::default();
    let preparations = PlaybackPreparations::default();
    let id = Uuid::new_v4().to_string();
    preparations.reserve(&id).unwrap();
    let guard = preparations.claim(&id).unwrap();
    let info = guard
        .publish(|| store.insert(storage.prepare_utterance_audio(&meeting, &analysis, &utterance)?))
        .unwrap();
    drop(guard);
    preparations.cancel(&id, &store).unwrap();
    assert!(store.read(&info.token, 0).is_err());
    assert!(preparations.acknowledge(&id, &info.token).is_err());
}

#[test]
fn changed_canonical_pcm_is_rejected_without_changing_transcript() {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open(temp.path().into()).unwrap();
    let (meeting, analysis, utterance) = fixture(&storage);
    let before = storage.load_current_analysis(&meeting).unwrap();
    let plan = storage.canonical_audio_plan(&meeting).unwrap();
    let path = storage.meeting_mix_path(&meeting, &plan.tracks[0]).unwrap();
    crate::audio::reuse::release_all();
    crate::local_engine::verification::release(&path).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(storage
        .prepare_utterance_audio(&meeting, &analysis, &utterance)
        .is_err());
    assert_eq!(storage.load_current_analysis(&meeting).unwrap(), before);
}
