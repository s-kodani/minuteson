//! Read existing source records through the owned connection, never by opening
//! another live SQLite connection. No checkpoint/input/recovery data is pruned.
use super::*;
use crate::job_history::Entry;
use std::collections::BTreeMap;

impl Storage {
    pub(crate) fn history_restored_meetings(&self, batch: &str) -> Result<Vec<String>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut q=db.prepare("SELECT m.target_key FROM restore_identity_maps m JOIN meetings a ON a.id=m.target_key WHERE m.batch_id=?1 AND m.domain='meetings' ORDER BY m.target_key").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([batch], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string());
        rows
    }
    pub(crate) fn history_model_name(&self, package: &str) -> Result<String, String> {
        self.connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?
            .query_row(
                "SELECT model_name FROM model_packages WHERE id=?1",
                [package],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub(crate) fn history_meetings(&self) -> Result<BTreeMap<String, String>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut q = db
            .prepare("SELECT id,title FROM meetings")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string());
        rows
    }
    pub(crate) fn history_source_records(&self, cutoff: i64) -> Result<Vec<Entry>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let mut records = Vec::new();
        {
            let mut q=db.prepare("SELECT j.id,j.kind,j.state,j.meeting_id,m.title,j.created_at,j.updated_at,j.error_code,j.finished_at FROM processing_jobs j JOIN meetings m ON m.id=j.meeting_id WHERE j.kind IN ('transcription','diarization','minutes_generation','voice_registration','voice_matching','speaker_completion') AND (j.updated_at>=?1 OR j.state IN ('queued','running','pausing','paused')) ORDER BY j.updated_at,j.id").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([cutoff], |r| {
                    let kind: String = r.get(1)?;
                    let state: String = r.get(2)?;
                    let updated: i64 = r.get(6)?;
                    let mut e = Entry::new(
                        &kind,
                        &r.get::<_, String>(0)?,
                        &r.get::<_, String>(4)?,
                        Some(r.get(3)?),
                        &state,
                    );
                    e.accepted_at = Some(r.get(5)?);
                    e.observed_at = updated;
                    e.detail = r.get(7)?;
                    e.finished_at =
                        if ["succeeded", "failed", "cancelled"].contains(&state.as_str()) {
                            r.get(8)?
                        } else {
                            None
                        };
                    Ok(e)
                })
                .map_err(|e| e.to_string())?;
            records.extend(
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
            );
        }
        {
            let mut q=db.prepare("SELECT r.id,r.meeting_id,m.title,r.state,r.updated_at FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.updated_at>=?1 AND r.state IN ('saved','failed','interrupted','stopping') AND NOT EXISTS(SELECT 1 FROM restore_provenance p WHERE p.meeting_id=r.meeting_id AND p.domain='meetings')").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([cutoff], |r| {
                    let state: String = r.get(3)?;
                    let updated: i64 = r.get(4)?;
                    let outcome = match state.as_str() {
                        "failed" => "failed",
                        "stopping" => "running",
                        _ => "interrupted",
                    };
                    let mut e = Entry::new(
                        "recording_save",
                        &r.get::<_, String>(0)?,
                        &r.get::<_, String>(2)?,
                        Some(r.get(1)?),
                        outcome,
                    );
                    // Old sessions do not retain the beginning of audio saving.
                    e.accepted_at = None;
                    e.finished_at = if state == "failed" {
                        Some(updated)
                    } else {
                        None
                    };
                    e.observed_at = updated;
                    Ok(e)
                })
                .map_err(|e| e.to_string())?;
            records.extend(
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
            );
        }
        {
            let mut q=db.prepare("SELECT p.operation_id,i.meeting_id,m.title,i.state,i.created_at,i.updated_at,coalesce(p.error_message,i.error_code) FROM audio_imports i JOIN audio_import_commits p ON p.import_id=i.id JOIN meetings m ON m.id=i.meeting_id WHERE i.updated_at>=?1 AND NOT EXISTS(SELECT 1 FROM restore_provenance v WHERE v.meeting_id=i.meeting_id AND v.domain='meetings')").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([cutoff], |r| {
                    let state: String = r.get(3)?;
                    let operation: String = r.get(0)?;
                    let updated: i64 = r.get(5)?;
                    let outcome = match state.as_str() {
                        "ready" => "succeeded",
                        "failed" => "failed",
                        _ => "running",
                    };
                    let mut e = Entry::new(
                        "audio_import",
                        operation.strip_prefix("import:").unwrap_or(&operation),
                        &r.get::<_, String>(2)?,
                        Some(r.get(1)?),
                        outcome,
                    );
                    e.accepted_at = Some(r.get(4)?);
                    e.observed_at = updated;
                    e.detail = r.get(6)?;
                    e.finished_at = if ["ready", "failed"].contains(&state.as_str()) {
                        Some(updated)
                    } else {
                        None
                    };
                    Ok(e)
                })
                .map_err(|e| e.to_string())?;
            records.extend(
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
            );
        }
        {
            let mut q=db.prepare("SELECT id,state,created_at,updated_at,error_code FROM maintenance_operations WHERE kind='delete_meeting' AND updated_at>=?1 AND consent_at IS NOT NULL").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([cutoff], |r| {
                    let state: String = r.get(1)?;
                    let time: i64 = r.get(3)?;
                    let outcome = match state.as_str() {
                        "completed" | "recovered" => "succeeded",
                        "attention" => "attention",
                        _ => "interrupted",
                    };
                    let mut e = Entry::new(
                        "deletion",
                        &r.get::<_, String>(0)?,
                        "会議 1件",
                        None,
                        outcome,
                    );
                    e.accepted_at = Some(r.get(2)?);
                    e.observed_at = time;
                    e.finished_at = if outcome == "succeeded" {
                        Some(time)
                    } else {
                        None
                    };
                    e.detail =
                        (outcome == "attention").then(|| "削除結果の確認が必要です。".into());
                    Ok(e)
                })
                .map_err(|e| e.to_string())?;
            records.extend(
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
            );
        }
        {
            let mut q=db.prepare("SELECT batch_id,created_at,published_at,(SELECT count(*) FROM restore_identity_maps m WHERE m.batch_id=b.batch_id AND m.domain='meetings') FROM restore_batches b WHERE state='published' AND published_at>=?1").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([cutoff], |r| {
                    let time: i64 = r.get(2)?;
                    let count: i64 = r.get(3)?;
                    let mut e = Entry::new(
                        "restore",
                        &r.get::<_, String>(0)?,
                        &format!("会議 {count}件"),
                        None,
                        "succeeded",
                    );
                    e.accepted_at = Some(r.get(1)?);
                    e.finished_at = Some(time);
                    e.observed_at = time;
                    Ok(e)
                })
                .map_err(|e| e.to_string())?;
            records.extend(
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(records)
    }

    // This receipt only reuses an observation made for the history. Playback,
    // recovery and publication still verify their own source bytes.
    fn recording_history_source(
        &self,
        session: &str,
        meeting: &str,
    ) -> Result<Option<String>, String> {
        if !self.recording_save_committed(session, meeting)? {
            return Ok(None);
        }
        let plan = self.canonical_audio_plan(meeting)?;
        let track = &plan.tracks[0];
        let path = self.meeting_mix_path(meeting, track)?;
        if path.canonicalize().map_err(|e| e.to_string())? != path {
            return Err("共通WAVの保存先が変更されています".into());
        }
        let metadata = crate::native_fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || u64::try_from(track.chunks[0].size).ok() != Some(metadata.len()) {
            return Err("共通WAVのサイズが保存時と一致しません".into());
        }
        let source = serde_json::to_vec(&(
            "recording-history-verification-v1",
            session,
            meeting,
            plan,
            crate::audio::reuse::stamp(&metadata),
        ))
        .map_err(|e| e.to_string())?;
        Ok(Some(format!("{:x}", Sha256::digest(source))))
    }

    pub(crate) fn verify_recording_history_source(
        &self,
        entry: &mut Entry,
        previous: Option<&str>,
        cancelled: &impl Fn() -> bool,
    ) -> Option<String> {
        let result = (|| -> Result<Option<String>, String> {
            let session = entry
                .id
                .strip_prefix("recording_save:")
                .ok_or("録音履歴IDが不正です")?;
            let meeting = entry
                .meeting_id
                .as_deref()
                .ok_or("録音履歴の会議が不明です")?;
            let Some(source) = self.recording_history_source(session, meeting)? else {
                return Ok(None);
            };
            if previous != Some(source.as_str()) {
                if !self.recording_save_complete(session, meeting, cancelled)? {
                    return Ok(None);
                }
                if self.recording_history_source(session, meeting)?.as_ref() != Some(&source) {
                    return Err("照合中に共通WAVの保存記録またはファイルが変更されました".into());
                }
            }
            Ok(Some(source))
        })();
        match result {
            Ok(Some(source)) => {
                entry.state = "succeeded".into();
                entry.finished_at = Some(entry.observed_at);
                Some(source)
            }
            Ok(None) => None,
            Err(error) => {
                entry.state = "interrupted".into();
                entry.finished_at = None;
                entry.detail = Some(format!(
                    "録音の保存完了を確認できません。元の記録を保持しています: {error}"
                ));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backfill_reads_confirmed_source_times_without_rewriting_existing_jobs() {
        use crate::storage::Storage;
        let dir = tempfile::tempdir().unwrap();
        let store = Storage::open(dir.path().into()).unwrap();
        let meeting = store.create_meeting("履歴の元会議").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        {
            let db = store.connection.lock().unwrap();
            db.execute("INSERT INTO processing_jobs(id,meeting_id,kind,state,operation_id,input_hash,profile_snapshot_json,parameters_json,spec_hash,total_units,completed_units,created_at,updated_at,finished_at) VALUES(?1,?2,'transcription','succeeded',?3,?4,'{}',?5,?4,1,1,100,200,190)",params![id,meeting.id,uuid::Uuid::new_v4().to_string(),"a".repeat(64),serde_json::json!({"schemaVersion":crate::current_formats::TRANSCRIPTION}).to_string()]).unwrap();
        }
        let rows = store.history_source_records(0).unwrap();
        let row = rows
            .iter()
            .find(|e| e.id == format!("transcription:{id}"))
            .unwrap();
        assert_eq!(row.target, "履歴の元会議");
        assert_eq!(row.accepted_at, Some(100));
        assert_eq!(row.finished_at, Some(190));
        assert_eq!(row.observed_at, 200);
        assert!(store.history_source_records(201).unwrap().is_empty());
        assert_eq!(store.list_jobs(&meeting.id).unwrap()[0].state, "succeeded");
    }

    #[test]
    fn recording_backfill_requires_verified_wav_and_completed_cleanup_not_just_saved_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = Storage::open(dir.path().into()).unwrap();
        let (meeting, session, _, _) = super::super::recordings::tests::saved_probe_fixture(&store);
        store.finalize_recording(&session).unwrap();
        assert!(store
            .recording_save_complete(&session, &meeting, &|| false)
            .unwrap());
        let candidate = || {
            store
                .history_source_records(0)
                .unwrap()
                .into_iter()
                .find(|e| e.id == format!("recording_save:{session}"))
                .unwrap()
        };
        let mut verified = candidate();
        let proof = store
            .verify_recording_history_source(&mut verified, None, &|| false)
            .unwrap();
        assert_eq!(verified.state, "succeeded");
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE meeting_audio_commits SET phase='committed' WHERE meeting_id=?1",
                [&meeting],
            )
            .unwrap();
        let mut pending = candidate();
        assert!(store
            .verify_recording_history_source(&mut pending, Some(&proof), &|| false)
            .is_none());
        assert_eq!(pending.state, "interrupted");
        assert!(pending.finished_at.is_none());
        assert!(!store
            .recording_save_complete(&session, &meeting, &|| false)
            .unwrap());
        store
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE meeting_audio_commits SET phase='cleaned' WHERE meeting_id=?1",
                [&meeting],
            )
            .unwrap();
        let plan = store.canonical_audio_plan(&meeting).unwrap();
        let path = store.meeting_mix_path(&meeting, &plan.tracks[0]).unwrap();
        std::fs::write(path, b"invalid canonical wav").unwrap();
        let mut unverified = candidate();
        assert!(store
            .verify_recording_history_source(&mut unverified, Some(&proof), &|| false)
            .is_none());
        assert_eq!(unverified.state, "interrupted");
        assert!(unverified
            .detail
            .as_ref()
            .is_some_and(|detail| detail.contains("保存完了を確認できません")));
    }

    #[test]
    fn recording_history_reuses_unchanged_wav_after_reopen_and_detects_modified_bytes() {
        use std::cell::Cell;
        let dir = tempfile::tempdir().unwrap();
        let store = Storage::open(dir.path().into()).unwrap();
        let (meeting, session, _, _) = super::super::recordings::tests::saved_probe_fixture(&store);
        store.finalize_recording(&session).unwrap();
        drop(store);
        let candidate = |store: &Storage| {
            store
                .history_source_records(0)
                .unwrap()
                .into_iter()
                .find(|e| e.id == format!("recording_save:{session}"))
                .unwrap()
        };
        let store = Storage::open(dir.path().into()).unwrap();
        let plan = store.canonical_audio_plan(&meeting).unwrap();
        let size = plan.tracks[0].chunks[0].size;
        let legacy_checks = Cell::new(0);
        assert!(store
            .recording_save_complete(&session, &meeting, &|| {
                legacy_checks.set(legacy_checks.get() + 1);
                false
            })
            .unwrap());
        assert!(legacy_checks.get() > 0);
        let proof = store
            .verify_recording_history_source(&mut candidate(&store), None, &|| false)
            .unwrap();
        drop(store);
        let store = Storage::open(dir.path().into()).unwrap();
        let mut reused = candidate(&store);
        assert_eq!(
            store.verify_recording_history_source(&mut reused, Some(&proof), &|| {
                panic!("unchanged audio must not enter canonical byte verification")
            }),
            Some(proof.clone())
        );
        assert_eq!(reused.state, "succeeded");
        println!("Unchanged restart: previous WAV SHA-256 input {size} bytes, durable history receipt 0 WAV bytes");

        let path = store.meeting_mix_path(&meeting, &plan.tracks[0]).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.last_mut().unwrap();
        *last ^= 1;
        std::fs::write(path, bytes).unwrap();
        let mut changed = candidate(&store);
        let checks = Cell::new(0);
        assert!(store
            .verify_recording_history_source(&mut changed, Some(&proof), &|| {
                checks.set(checks.get() + 1);
                false
            })
            .is_none());
        assert!(
            checks.get() > 0,
            "same-size modified WAV must require a fresh digest"
        );
        assert_eq!(changed.state, "interrupted");
    }
}
