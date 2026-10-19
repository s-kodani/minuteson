use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct DraftUtteranceAudioRequest {
    pub meeting_id: String,
    pub analysis_id: String,
    pub expected_content_hash: String,
    pub expected_draft: DraftStamp,
    pub utterance_id: String,
    pub expected_start_us: i64,
    pub expected_end_us: i64,
}

impl Storage {
    #[cfg(test)]
    pub(crate) fn prepare_draft_utterance_audio(
        &self,
        request: &DraftUtteranceAudioRequest,
    ) -> Result<crate::playback::PreparedAudio, String> {
        self.prepare_draft_utterance_audio_cancellable(
            request,
            &crate::test_support::never_cancelled,
        )
    }
    pub(in crate::storage) fn resolve_draft_utterance_audio_reference(
        &self,
        request: &DraftUtteranceAudioRequest,
    ) -> Result<String, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let visible:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM analysis_revisions a JOIN meetings m ON m.id=a.meeting_id WHERE a.id=?1 AND a.meeting_id=?2 AND (a.publication_state='candidate' OR (a.publication_state='committed' AND a.id IN (m.current_analysis_id,m.previous_analysis_id))))",params![request.analysis_id,request.meeting_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !visible {
            return Err("この解析版は分割音声の参照対象ではありません".into());
        }
        let hash: String = tx
            .query_row(
                "SELECT content_hash FROM analysis_revisions WHERE id=?1",
                params![request.analysis_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if super::revisions::analysis_content_hash(&tx, &request.analysis_id)? != hash {
            return Err("解析版の内容が変更されています".into());
        }
        let mut view = super::analysis::read_analysis_content(
            &tx,
            &request.meeting_id,
            &request.analysis_id,
            &hash,
        )?;
        let mut draft = super::drafts::load_draft_for_base(
            &tx,
            &request.meeting_id,
            Some(&request.analysis_id),
        )?
        .ok_or("保存済み下書きがありません")?;
        draft.payload = super::drafts::validate_payload(&tx, &request.analysis_id, draft.payload)?;
        if view.id != request.analysis_id
            || view.content_hash != request.expected_content_hash
            || draft.id != request.expected_draft.id
            || draft.lock_version != request.expected_draft.lock_version
        {
            return Err(
                "再生前に現在版または分割下書きが変化しています。保存後に確認してください".into(),
            );
        }
        super::minutes_input::apply_edits(&mut view, &draft.payload);
        let u = view
            .utterances
            .iter()
            .find(|u| u.id == request.utterance_id)
            .ok_or("この下書きの発言がありません")?;
        if u.start_us != request.expected_start_us || u.end_us != request.expected_end_us {
            return Err("確認した分割時刻と再生区間が一致しません".into());
        }
        Ok(u.track_id.clone())
    }

    pub(crate) fn prepare_draft_utterance_audio_cancellable(
        &self,
        request: &DraftUtteranceAudioRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<crate::playback::PreparedAudio, String> {
        if cancelled() {
            return Err("音声の再生準備を取り消しました".into());
        }
        let resolve = || self.resolve_draft_utterance_audio_reference(request);
        let track = resolve()?;
        let audio = self.prepare_preview_track_audio_cancellable(
            &request.meeting_id,
            &track,
            request.expected_start_us,
            request.expected_end_us,
            cancelled,
        )?;
        if cancelled() {
            return Err("音声の再生準備を取り消しました".into());
        }
        if resolve()? != track {
            return Err("音声準備中に分割元トラックが変化しました".into());
        }
        Ok(audio)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitPart {
    pub id: String,
    pub start_us: i64,
    pub end_us: i64,
    pub effective_text: String,
    pub speaker_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UtteranceSplit {
    pub source_utterance_id: String,
    pub source_effective_text: String,
    pub parts: Vec<SplitPart>,
}

pub(super) fn validate_splits(
    db: &Connection,
    base: &str,
    payload: &mut TranscriptDraftPayload,
    speakers: &HashSet<String>,
    utterances: &mut HashSet<String>,
) -> Result<(), String> {
    let edits: HashMap<_, _> = payload
        .utterances
        .iter()
        .map(|e| (e.utterance_id.as_str(), e.effective_text.as_str()))
        .collect();
    let mut sources = HashSet::new();
    let mut count = utterances.len();
    for split in &mut payload.utterance_splits {
        if !utterances.contains(&split.source_utterance_id)
            || !sources.insert(split.source_utterance_id.clone())
            || !(2..=1000).contains(&split.parts.len())
        {
            return Err("分割元の発言または分割数が不正です".into());
        }
        let (start,end,text):(i64,i64,String)=db.query_row(
            "SELECT start_us,end_us,effective_text FROM utterances WHERE id=?1 AND analysis_revision_id=?2",
            params![split.source_utterance_id,base],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).map_err(|e|e.to_string())?;
        if split.source_effective_text
            != edits
                .get(split.source_utterance_id.as_str())
                .copied()
                .unwrap_or(&text)
            || split.source_effective_text.chars().count() > 20_000
        {
            return Err(
                "分割元の本文が変化しています。分割を取り消してから編集してください".into(),
            );
        }
        let mut cursor = start;
        let mut joined = String::new();
        for part in &mut split.parts {
            let valid = part
                .id
                .strip_prefix("split:")
                .is_some_and(|id| Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id));
            if !valid
                || !utterances.insert(part.id.clone())
                || part.start_us != cursor
                || part.end_us <= cursor
                || part.end_us > end
                || part.effective_text.is_empty()
                || part.effective_text.chars().count() > 20_000
            {
                return Err(
                    "分割先のID・時刻・本文が不正です。元区間を隙間なく覆ってください".into(),
                );
            }
            let collision:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM utterances WHERE analysis_revision_id=?1 AND lineage_id=?2)",params![base,part.id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if collision {
                return Err("分割先の由来IDが既存発言と重複しています".into());
            }
            part.speaker_ids.sort();
            if part.speaker_ids.windows(2).any(|p| p[0] == p[1])
                || part.speaker_ids.iter().any(|id| !speakers.contains(id))
            {
                return Err("分割先の話者割り当てが不正です".into());
            }
            joined.push_str(&part.effective_text);
            cursor = part.end_us;
        }
        if cursor != end || joined != split.source_effective_text {
            return Err("分割による本文の欠落・重複または時刻の欠落があります".into());
        }
        count = count
            .checked_add(split.parts.len() - 1)
            .ok_or("分割件数が大きすぎます")?;
        if count > 100_000 {
            return Err("分割後の発言は100000件までです".into());
        }
    }
    payload
        .utterance_splits
        .sort_by(|a, b| a.source_utterance_id.cmp(&b.source_utterance_id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::playback::tests::fixture;
    use super::*;
    use crate::playback::{PlaybackPreparations, PlaybackStore};
    use std::cell::Cell;

    #[test]
    fn native_cancel_and_draft_change_during_hash_do_not_publish_split_audio() {
        for cancel in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let storage = Storage::open(root.path().into()).unwrap();
            let (meeting, analysis, _) = fixture(&storage);
            let (draft, p) = payload(&storage, &meeting);
            let part = p.utterance_splits[0].parts[0].clone();
            let saved = storage.save_transcript_draft(&draft.id, 0, p).unwrap();
            let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
            let request = DraftUtteranceAudioRequest {
                meeting_id: meeting.clone(),
                analysis_id: analysis,
                expected_content_hash: view.content_hash,
                expected_draft: DraftStamp {
                    id: saved.id.clone(),
                    lock_version: saved.lock_version,
                },
                utterance_id: part.id,
                expected_start_us: part.start_us,
                expected_end_us: part.end_us,
            };
            let preparations = PlaybackPreparations::default();
            let store = PlaybackStore::default();
            let id = Uuid::new_v4().to_string();
            preparations.reserve(&id).unwrap();
            let guard = preparations.claim(&id).unwrap();
            let checks = Cell::new(0);
            let error = storage
                .prepare_draft_utterance_audio_cancellable(&request, &|| {
                    checks.set(checks.get() + 1);
                    if cancel && checks.get() == 5 {
                        preparations.cancel(&id, &store).unwrap();
                    }
                    if !cancel && checks.get() == 4 {
                        storage
                            .undo_last_transcript_edit(&draft.id, saved.lock_version)
                            .unwrap();
                    }
                    guard.cancelled()
                })
                .err()
                .unwrap();
            if cancel {
                assert!(error.contains("照合を中断"), "{error}");
                assert_eq!(checks.get(), 5);
                assert_eq!(
                    storage.load_transcript_draft(&meeting).unwrap(),
                    Some(saved)
                );
                assert!(guard
                    .publish(|| panic!("cancelled split must not publish"))
                    .is_err());
                assert!(storage.prepare_draft_utterance_audio(&request).is_ok());
            } else {
                assert!(error.contains("分割下書きが変化"), "{error}");
            }
            drop(guard);
            assert!(preparations.claim(&id).is_err());
        }
    }
    fn payload(storage: &Storage, meeting: &str) -> (TranscriptDraft, TranscriptDraftPayload) {
        let view = storage.load_current_analysis(meeting).unwrap().unwrap();
        let draft = storage.create_transcript_draft(meeting, &view.id).unwrap();
        let u = &view.utterances[0];
        let mut p = draft.payload.clone();
        p.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
        p.utterance_splits.push(UtteranceSplit {
            source_utterance_id: u.id.clone(),
            source_effective_text: u.effective_text.clone(),
            parts: vec![
                SplitPart {
                    id: format!("split:{}", Uuid::new_v4()),
                    start_us: u.start_us,
                    end_us: 500_000,
                    effective_text: "発".into(),
                    speaker_ids: vec![],
                },
                SplitPart {
                    id: format!("split:{}", Uuid::new_v4()),
                    start_us: 500_000,
                    end_us: u.end_us,
                    effective_text: "言".into(),
                    speaker_ids: vec![],
                },
            ],
        });
        (draft, p)
    }
    #[test]
    fn split_validation_rejects_lost_text_gaps_overlaps_wrong_scope_and_stale_source() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let (meeting, _, _) = fixture(&storage);
        let (draft, valid) = payload(&storage, &meeting);
        let mut cases = vec![];
        let mut p = valid.clone();
        p.schema_version = 2;
        cases.push(p);
        let mut p = valid.clone();
        p.utterance_splits[0].parts[0].effective_text = "欠落".into();
        cases.push(p);
        let mut p = valid.clone();
        p.utterance_splits[0].parts[1].start_us += 1;
        cases.push(p);
        let mut p = valid.clone();
        p.utterance_splits[0].parts[1].start_us -= 1;
        cases.push(p);
        let mut p = valid.clone();
        p.utterance_splits[0].parts[1].id = p.utterance_splits[0].parts[0].id.clone();
        cases.push(p);
        let mut p = valid.clone();
        p.utterance_splits[0].source_utterance_id = "foreign".into();
        cases.push(p);
        let mut p = valid.clone();
        p.utterance_splits[0].parts[0].speaker_ids = vec!["foreign".into()];
        cases.push(p);
        let mut p = valid.clone();
        p.utterance_splits[0].parts[0].effective_text.clear();
        cases.push(p);
        let mut p = valid.clone();
        p.utterances.push(super::super::drafts::UtteranceEdit {
            utterance_id: p.utterance_splits[0].source_utterance_id.clone(),
            effective_text: "変更した分割元".into(),
            speaker_ids: vec![],
        });
        cases.push(p);
        for p in cases {
            assert!(storage.save_transcript_draft(&draft.id, 0, p).is_err());
        }
        assert_eq!(
            storage.load_transcript_draft(&meeting).unwrap(),
            Some(draft)
        );
    }
    #[test]
    fn split_audio_uses_only_verified_child_range_and_rejects_stale_or_forged_requests() {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().to_owned()).unwrap();
        let (meeting, analysis, _) = fixture(&storage);
        let (draft, p) = payload(&storage, &meeting);
        let first = p.utterance_splits[0].parts[0].clone();
        let second = p.utterance_splits[0].parts[1].clone();
        let saved = storage.save_transcript_draft(&draft.id, 0, p).unwrap();
        let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
        let request = DraftUtteranceAudioRequest {
            meeting_id: meeting.clone(),
            analysis_id: analysis,
            expected_content_hash: view.content_hash,
            expected_draft: DraftStamp {
                id: saved.id.clone(),
                lock_version: saved.lock_version,
            },
            utterance_id: first.id,
            expected_start_us: first.start_us,
            expected_end_us: first.end_us,
        };
        let mut first_audio = storage.prepare_draft_utterance_audio(&request).unwrap();
        assert_eq!(first_audio.total_frames(), 12_000);
        for sample in first_audio.read_pcm(0, 12_000).unwrap().chunks_exact(4) {
            assert_eq!(f32::from_le_bytes(sample.try_into().unwrap()), 0.25);
        }
        let mut second_request = request.clone();
        second_request.utterance_id = second.id;
        second_request.expected_start_us = second.start_us;
        second_request.expected_end_us = second.end_us;
        let mut second_audio = storage
            .prepare_draft_utterance_audio(&second_request)
            .unwrap();
        for sample in second_audio.read_pcm(0, 12_000).unwrap().chunks_exact(4) {
            assert_eq!(f32::from_le_bytes(sample.try_into().unwrap()), -0.5);
        }
        let mut invalid = request.clone();
        invalid.expected_end_us += 1;
        assert!(storage.prepare_draft_utterance_audio(&invalid).is_err());
        let mut invalid = request.clone();
        invalid.utterance_id = "foreign".into();
        assert!(storage.prepare_draft_utterance_audio(&invalid).is_err());
        let mut invalid = request.clone();
        invalid.meeting_id = "foreign".into();
        assert!(storage.prepare_draft_utterance_audio(&invalid).is_err());
        let mut invalid = request.clone();
        invalid.expected_content_hash = "0".repeat(64);
        assert!(storage.prepare_draft_utterance_audio(&invalid).is_err());
        let undone = storage
            .undo_last_transcript_edit(&draft.id, saved.lock_version)
            .unwrap();
        assert_eq!(undone.content_hash, draft.content_hash);
        assert!(storage.prepare_draft_utterance_audio(&request).is_err());
    }
    #[test]
    fn split_playback_start_uses_full_meeting_and_rejects_draft_changes_during_preparation() {
        for mutate in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let storage = Storage::open(root.path().into()).unwrap();
            let (meeting, analysis, _) = fixture(&storage);
            let (draft, p) = payload(&storage, &meeting);
            let part = p.utterance_splits[0].parts[1].clone();
            let saved = storage.save_transcript_draft(&draft.id, 0, p).unwrap();
            let view = storage.load_current_analysis(&meeting).unwrap().unwrap();
            {
                let db = storage.connection.lock().unwrap();
                db.execute(
                    "UPDATE meetings SET duration_us=1000000 WHERE id=?1",
                    params![meeting],
                )
                .unwrap();
                db.execute(
                    "UPDATE recording_sessions SET duration_us=1000000 WHERE meeting_id=?1",
                    params![meeting],
                )
                .unwrap();
            }
            let request = super::super::meeting_playback::MeetingPlaybackRequest {
                meeting_id: meeting.clone(),
                mode: super::super::meeting_playback::MeetingPlaybackMode::Mic,
                start_reference: Some(
                    super::super::meeting_playback::MeetingPlaybackStartReference::DraftUtterance {
                        request: DraftUtteranceAudioRequest {
                            meeting_id: meeting.clone(),
                            analysis_id: analysis,
                            expected_content_hash: view.content_hash,
                            expected_draft: DraftStamp {
                                id: saved.id.clone(),
                                lock_version: saved.lock_version,
                            },
                            utterance_id: part.id,
                            expected_start_us: part.start_us,
                            expected_end_us: part.end_us,
                        },
                    },
                ),
            };
            let count = Cell::new(0);
            let audio = storage.prepare_meeting_audio_cancellable(&request, &|| {
                count.set(count.get() + 1);
                if mutate && count.get() == 3 {
                    storage
                        .undo_last_transcript_edit(&draft.id, saved.lock_version)
                        .unwrap();
                }
                false
            });
            if mutate {
                assert!(audio.is_err());
            } else {
                let info = PlaybackStore::default()
                    .insert_meeting(audio.unwrap())
                    .unwrap();
                assert_eq!(info.start_us, 0);
                assert_eq!(info.end_us, 1000000);
                assert_eq!(info.total_frames, 48000);
            }
        }
    }
}
