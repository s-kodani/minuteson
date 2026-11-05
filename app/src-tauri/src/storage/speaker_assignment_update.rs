//! Reuse saved evidence without changing text, timing or detector output.
#[cfg(test)]
pub(crate) use super::transcript_consolidation::Request;
use super::*;
use rusqlite::Transaction;
#[cfg(test)]
use rusqlite::TransactionBehavior;
use std::collections::BTreeSet;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    pub analysis: AnalysisView,
    pub draft: Option<TranscriptDraft>,
    pub updated_count: usize,
    pub matching_error: Option<String>,
    pub completion_job_id: Option<String>,
}

// A cleared assignment has no speaker row, so preserve its lineage explicitly.
pub(super) fn manual_lineages(
    tx: &Transaction<'_>,
    analysis: &str,
) -> Result<BTreeSet<String>, String> {
    let config: String = tx
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            [analysis],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let config: serde_json::Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
    let mut manual: BTreeSet<String> = config
        .get("manualAssignmentLineages")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let mut query = tx.prepare(
        "SELECT DISTINCT u.lineage_id FROM utterances u
         JOIN utterance_speakers s ON s.utterance_id=u.id AND s.analysis_revision_id=u.analysis_revision_id
         WHERE u.analysis_revision_id=?1 AND s.assignment_source='manual'
         UNION SELECT u.lineage_id FROM diarization_manual_assignments d
         JOIN utterances u ON u.id=d.utterance_id AND u.analysis_revision_id=d.analysis_id
         WHERE d.analysis_id=?1"
    ).map_err(|e| e.to_string())?;
    for lineage in query
        .query_map([analysis], |row| row.get(0))
        .map_err(|e| e.to_string())?
    {
        manual.insert(lineage.map_err(|e| e.to_string())?);
    }
    let draft: Option<String> = tx.query_row(
        "SELECT payload_json FROM working_drafts WHERE base_analysis_id=?1 AND target_kind='transcript'",
        [analysis], |row| row.get(0),
    ).optional().map_err(|e| e.to_string())?;
    if let Some(draft) = draft {
        let payload: TranscriptDraftPayload =
            serde_json::from_str(&draft).map_err(|e| e.to_string())?;
        let mut assignments = tx.prepare(
            "SELECT u.id,u.lineage_id,s.speaker_id FROM utterances u LEFT JOIN utterance_speakers s
             ON s.utterance_id=u.id AND s.analysis_revision_id=u.analysis_revision_id
             WHERE u.analysis_revision_id=?1 ORDER BY u.id,s.speaker_id"
        ).map_err(|e| e.to_string())?;
        let mut rows = assignments.query([analysis]).map_err(|e| e.to_string())?;
        let mut by_utterance = std::collections::BTreeMap::<String, (String, Vec<String>)>::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let id: String = row.get(0).map_err(|e| e.to_string())?;
            let lineage: String = row.get(1).map_err(|e| e.to_string())?;
            let speaker: Option<String> = row.get(2).map_err(|e| e.to_string())?;
            let (_, speakers) = by_utterance
                .entry(id)
                .or_insert_with(|| (lineage, Vec::new()));
            speakers.extend(speaker);
        }
        let edits: std::collections::BTreeMap<_, _> = payload
            .utterances
            .iter()
            .map(|edit| (edit.utterance_id.as_str(), &edit.speaker_ids))
            .collect();
        for (id, (lineage, speakers)) in by_utterance {
            let mut effective = edits
                .get(id.as_str())
                .map_or_else(|| speakers.clone(), |ids| (*ids).clone());
            effective.sort();
            if super::drafts::merged_assignments(&payload, &effective) != speakers {
                manual.insert(lineage);
            }
        }
        manual.extend(
            payload
                .utterance_splits
                .into_iter()
                .flat_map(|split| split.parts.into_iter().map(|part| part.id)),
        );
    }
    Ok(manual)
}
// A known-run result is evidence only inside its frozen continuous runs.
// Explicit gaps alone cannot describe every clock discontinuity.
#[cfg(test)]
fn saved_runs(
    tx: &Transaction<'_>,
    analysis: &str,
) -> Result<Option<Vec<(String, i64, i64)>>, String> {
    let config: String = tx
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            [analysis],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let config: serde_json::Value = serde_json::from_str(&config).map_err(|e| e.to_string())?;
    if config["audioCoverage"]["mode"] != "known_runs" {
        return Ok(None);
    }
    let job = config["jobId"]
        .as_str()
        .ok_or("保存区間の話者分離の由来がありません")?;
    let (spec, _) = super::jobs::read_fixed_input(tx, job)?;
    let source: RecordedAudioPlan =
        serde_json::from_value(spec.parameters["diarization"]["source"].clone())
            .map_err(|e| e.to_string())?;
    let known: KnownInput =
        serde_json::from_value(spec.parameters["diarization"]["knownInput"].clone())
            .map_err(|e| e.to_string())?;
    known.validate(&source)?;
    Ok(Some(
        known
            .runs
            .iter()
            .map(|r| {
                (
                    source.tracks[r.track_index].id.clone(),
                    r.start_us,
                    r.end_us,
                )
            })
            .collect(),
    ))
}
#[cfg(test)]
fn within_run(runs: &[(String, i64, i64)], track: &str, start: i64, end: i64) -> bool {
    runs.iter()
        .any(|(id, from, to)| id == track && start >= *from && end <= *to)
}
impl Storage {
    #[cfg(test)]
    pub(crate) fn update_speaker_assignments(&self, request: &Request) -> Result<Reply, String> {
        self.require_data_writable()?;
        if Uuid::parse_str(&request.operation_id).is_err() {
            return Err("操作IDが不正です".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (mut source, draft, title, created, started) =
            super::minutes_input::current_working(&tx, &request.meeting_id)?;
        let stamp = draft.as_ref().map(|d| DraftStamp {
            id: d.id.clone(),
            lock_version: d.lock_version,
        });
        if source.id != request.analysis_id {
            let (parent,hash,config):(Option<String>,Option<String>,String)=tx.query_row("SELECT parent_revision_id,parent_content_hash,config_json FROM analysis_revisions WHERE id=?1",[&source.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
            let config: serde_json::Value =
                serde_json::from_str(&config).map_err(|e| e.to_string())?;
            let receipt = &config["speakerAssignmentUpdate"];
            if parent.as_deref() == Some(&request.analysis_id)
                && hash.as_deref() == Some(&request.expected_content_hash)
                && receipt["operationId"] == request.operation_id
                && receipt["sourceDraft"]
                    == serde_json::to_value(&request.expected_draft).map_err(|e| e.to_string())?
            {
                return Ok(Reply {
                    analysis: source,
                    draft,
                    updated_count: receipt["updatedCount"].as_u64().unwrap_or(0) as usize,
                    matching_error: None,
                    completion_job_id: None,
                });
            }
            return Err("対象の文字起こし版が変わりました。再読込してください".into());
        }
        if source.content_hash != request.expected_content_hash || stamp != request.expected_draft {
            return Err("編集内容が変更されました。再読込してください".into());
        }
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping')) OR EXISTS(SELECT 1 FROM processing_jobs WHERE meeting_id=?1 AND state IN ('queued','running','pausing','paused','interrupted'))",[&request.meeting_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if busy {
            return Err("録音・解析の完了後に話者割り当てを更新してください".into());
        }
        let base = source.clone();
        let payload = draft
            .as_ref()
            .map(|d| d.payload.clone())
            .unwrap_or_default();
        let lineages = super::minutes_input::lineage_map(&tx, "utterances", &source.id)?;
        let mut manual = manual_lineages(&tx, &source.id)?;
        for e in &payload.utterances {
            if source
                .utterances
                .iter()
                .any(|u| u.id == e.utterance_id && u.speaker_ids != e.speaker_ids)
            {
                if let Some(l) = lineages.get(&e.utterance_id) {
                    manual.insert(l.clone());
                }
            }
        }
        for split in &payload.utterance_splits {
            for part in &split.parts {
                manual.insert(part.id.clone());
            }
        }
        super::minutes_input::apply_edits(&mut source, &payload);
        let runs = saved_runs(&tx, &source.id)?;
        let mut assignments: HashMap<String, Vec<String>> = HashMap::new();
        for u in &source.utterances {
            let lineage = lineages.get(&u.id).unwrap_or(&u.id);
            if !u.speaker_ids.is_empty()
                || manual.contains(lineage)
                || runs
                    .as_ref()
                    .is_some_and(|runs| !within_run(runs, &u.track_id, u.start_us, u.end_us))
            {
                continue;
            }
            let gap:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM audio_gaps WHERE track_id=?1 AND recovered=0 AND start_us<?3 AND end_us>?2)",params![u.track_id,u.start_us,u.end_us],|r|r.get(0)).map_err(|e|e.to_string())?;
            if gap {
                continue;
            }
            let mut q=tx.prepare("SELECT DISTINCT speaker_id FROM speaker_segments WHERE analysis_revision_id=?1 AND track_id=?2 AND start_us<?4 AND end_us>?3 AND speaker_id IS NOT NULL ORDER BY speaker_id").map_err(|e|e.to_string())?;
            let rows = q
                .query_map(params![source.id, u.track_id, u.start_us, u.end_us], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            let ids = rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let ids = super::drafts::merged_assignments(&payload, &ids);
            if !ids.is_empty() {
                assignments.insert(lineage.clone(), ids);
            }
        }
        if assignments.is_empty() {
            return Ok(Reply {
                analysis: base,
                draft,
                updated_count: 0,
                matching_error: None,
                completion_job_id: None,
            });
        }
        let working = super::minutes_input::working_hash(
            &tx,
            &source,
            &title,
            created,
            started,
            Some(&payload),
        )?;
        let speaker_lineages = super::minutes_input::lineage_map(&tx, "speakers", &source.id)?;
        let update = |tx: &Transaction<'_>, id: &str| -> Result<(), String> {
            // Reuse the same merge application as ordinary public snapshots.
            super::transcript_consolidation::consolidate_copied_rows(
                tx,
                &source,
                &payload,
                id,
                request,
                &[],
            )?;
            for (lineage, ids) in &assignments {
                let utterance: String = tx
                    .query_row(
                        "SELECT id FROM utterances WHERE analysis_revision_id=?1 AND lineage_id=?2",
                        params![id, lineage],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                for (index, speaker) in ids.iter().enumerate() {
                    let lineage = speaker_lineages.get(speaker).unwrap_or(speaker);
                    let mapped:String=tx.query_row("SELECT id FROM speakers WHERE analysis_revision_id=?1 AND lineage_id=?2",params![id,lineage],|r|r.get(0)).map_err(|e|e.to_string())?;
                    tx.execute("INSERT INTO utterance_speakers(utterance_id,speaker_id,analysis_revision_id,role,assignment_source) VALUES(?1,?2,?3,?4,'inferred')",params![utterance,mapped,id,if index==0{"primary"}else{"overlap"}]).map_err(|e|e.to_string())?;
                }
                tx.execute("UPDATE utterances SET attribution_state=?1 WHERE id=?2 AND analysis_revision_id=?3",params![if ids.len()==1{"assigned"}else{"overlap"},utterance,id]).map_err(|e|e.to_string())?;
            }
            let text: String = tx
                .query_row(
                    "SELECT config_json FROM analysis_revisions WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut config: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| e.to_string())?;
            config
                .as_object_mut()
                .ok_or("解析設定が不正です")?
                .remove("utteranceConsolidation");
            config["speakerAssignmentUpdate"] = serde_json::json!({"schemaVersion":1,"operationId":request.operation_id,"sourceDraft":request.expected_draft,"updatedCount":assignments.len(),"policy":"all_detected_v1"});
            if config.to_string().len() > 2 * 1024 * 1024 {
                return Err("話者割り当ての更新履歴が大きすぎます".into());
            }
            tx.execute(
                "UPDATE analysis_revisions SET config_json=?1 WHERE id=?2",
                params![config.to_string(), id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        };
        let id = super::minutes_input::copy_snapshot_with_update(
            &tx,
            &source,
            &payload,
            &title,
            created,
            started,
            &working,
            None,
            None,
            Some(&update),
        )?;
        super::transcript_consolidation::copy_voice_matches(&tx, &source, &id)?;
        tx.execute(
            "DELETE FROM working_drafts WHERE meeting_id=?1 AND target_kind='transcript'",
            [&request.meeting_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("UPDATE meetings SET current_analysis_id=?1,lock_version=lock_version+1,updated_at=?3 WHERE id=?4 AND current_analysis_id=?2",params![id,source.id,now_ms()?,request.meeting_id]).map_err(|e|e.to_string())?;
        let hash = super::revisions::analysis_content_hash(&tx, &id)?;
        let analysis =
            super::analysis::read_analysis_content(&tx, &request.meeting_id, &id, &hash)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Reply {
            analysis,
            draft: None,
            updated_count: assignments.len(),
            matching_error: None,
            completion_job_id: None,
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn fixture_with_span(sequential: bool) -> (tempfile::TempDir, Storage, Request) {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (meeting, _, _) = super::super::playback::tests::fixture(&s);
        let id = s
            .create_analysis_candidate(
                &meeting,
                AnalysisInput {
                    audio_manifest_hash: s.recorded_audio_plan(&meeting).unwrap().manifest_hash,
                    profile_snapshot: serde_json::json!({}),
                    config: serde_json::json!({}),
                    speakers: vec![
                        SpeakerInput {
                            key: "a".into(),
                            label: "話者A".into(),
                        },
                        SpeakerInput {
                            key: "b".into(),
                            label: "話者B".into(),
                        },
                    ],
                    segments: vec![
                        SpeakerSegmentInput {
                            track_id: "track".into(),
                            speaker_key: Some("a".into()),
                            start_us: 0,
                            end_us: 500000,
                            overlap_group: None,
                            confidence: None,
                        },
                        SpeakerSegmentInput {
                            track_id: "track".into(),
                            speaker_key: Some("b".into()),
                            start_us: 500000,
                            end_us: 1000000,
                            overlap_group: None,
                            confidence: None,
                        },
                    ],
                    utterances: vec![
                        UtteranceInput {
                            track_id: "track".into(),
                            start_us: 0,
                            end_us: if sequential { 1000000 } else { 250000 },
                            text: "本日は".into(),
                            speaker_keys: vec![],
                            confidence: None,
                        },
                        UtteranceInput {
                            track_id: "track".into(),
                            start_us: 250000,
                            end_us: 500000,
                            text: "点検日です。".into(),
                            speaker_keys: vec![],
                            confidence: None,
                        },
                        UtteranceInput {
                            track_id: "track".into(),
                            start_us: 500000,
                            end_us: 1000000,
                            text: "承知しました。".into(),
                            speaker_keys: vec!["b".into()],
                            confidence: None,
                        },
                    ],
                },
            )
            .unwrap();
        s.commit_analysis_candidate(
            &meeting,
            &id,
            s.get_meeting(&meeting).unwrap().unwrap().lock_version,
            false,
        )
        .unwrap();
        let view = s.load_current_analysis(&meeting).unwrap().unwrap();
        let request = Request {
            meeting_id: meeting,
            analysis_id: id,
            expected_content_hash: view.content_hash,
            expected_draft: None,
            operation_id: Uuid::new_v4().to_string(),
        };
        (root, s, request)
    }

    fn fixture() -> (tempfile::TempDir, Storage, Request) {
        fixture_with_span(false)
    }
    fn sequential_fixture() -> (tempfile::TempDir, Storage, Request) {
        fixture_with_span(true)
    }
    #[test]
    fn saved_run_boundaries_do_not_allow_inference_across_clock_holes_or_tracks() {
        let runs = vec![
            ("track".to_string(), 0, 100),
            ("track".to_string(), 200, 300),
        ];
        assert!(within_run(&runs, "track", 0, 100));
        assert!(within_run(&runs, "track", 200, 300));
        assert!(!within_run(&runs, "track", 50, 250));
        assert!(!within_run(&runs, "other", 0, 100));
    }
    #[test]
    fn sequential_speakers_and_manual_merges_use_saved_segments() {
        let (_root, s, r) = sequential_fixture();
        let first = s.update_speaker_assignments(&r).unwrap();
        assert_eq!(first.analysis.utterances[0].speaker_ids.len(), 2);
        assert_eq!(first.analysis.utterances[0].attribution_state, "overlap");
        // A saved speaker merge must also repair unknown rows using the merged evidence.
        let (_root, s, mut r) = fixture();
        let v = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let d = s
            .create_transcript_draft(&r.meeting_id, &r.analysis_id)
            .unwrap();
        let mut payload = d.payload;
        payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
        payload
            .speaker_merges
            .push(super::super::drafts::SpeakerMerge {
                source_id: v.speakers[0].id.clone(),
                target_id: v.speakers[1].id.clone(),
            });
        let d = s.save_transcript_draft(&d.id, 0, payload).unwrap();
        r.expected_draft = Some(DraftStamp {
            id: d.id,
            lock_version: d.lock_version,
        });
        let reply = s.update_speaker_assignments(&r).unwrap();
        assert_eq!(reply.analysis.speakers.len(), 1);
        assert!(reply
            .analysis
            .utterances
            .iter()
            .all(|u| u.speaker_ids == vec![reply.analysis.speakers[0].id.clone()]));
    }
    #[test]
    fn gaps_busy_state_and_transaction_failure_do_not_change_current_revision() {
        let (_root, s, r) = fixture();
        let original_count: i64 = s
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM analysis_revisions", [], |r| r.get(0))
            .unwrap();
        s.connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO audio_gaps VALUES('gap','track',0,500000,'fixture',0)",
                [],
            )
            .unwrap();
        assert_eq!(s.update_speaker_assignments(&r).unwrap().updated_count, 0);
        s.connection
            .lock()
            .unwrap()
            .execute("DELETE FROM audio_gaps", [])
            .unwrap();
        s.connection
            .lock()
            .unwrap()
            .execute("UPDATE recording_sessions SET state='recording'", [])
            .unwrap();
        assert!(s.update_speaker_assignments(&r).is_err());
        s.connection
            .lock()
            .unwrap()
            .execute("UPDATE recording_sessions SET state='saved'", [])
            .unwrap();
        s.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER reject_update BEFORE UPDATE OF current_analysis_id ON meetings BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        assert!(s.update_speaker_assignments(&r).is_err());
        assert_eq!(
            s.load_current_analysis(&r.meeting_id).unwrap().unwrap().id,
            r.analysis_id
        );
        let count: i64 = s
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM analysis_revisions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, original_count);
    }
    #[test]
    fn updates_unknown_only_preserves_evidence_and_is_idempotent() {
        let (_root, s, r) = fixture();
        let before = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let reply = s.update_speaker_assignments(&r).unwrap();
        assert_eq!(reply.updated_count, 2);
        assert_ne!(reply.analysis.id, before.id);
        for (a, b) in before.utterances.iter().zip(&reply.analysis.utterances) {
            assert_eq!(
                (&a.effective_text, a.start_us, a.end_us),
                (&b.effective_text, b.start_us, b.end_us)
            );
            assert_eq!(b.speaker_ids.len(), 1);
        }
        assert_eq!(
            s.update_speaker_assignments(&r).unwrap().analysis,
            reply.analysis
        );
        let next = Request {
            analysis_id: reply.analysis.id.clone(),
            expected_content_hash: reply.analysis.content_hash.clone(),
            operation_id: Uuid::new_v4().to_string(),
            ..r.clone()
        };
        assert_eq!(
            s.update_speaker_assignments(&next).unwrap().updated_count,
            0
        );
        let mut db = s.connection.lock().unwrap();
        let db = db.transaction().unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM speaker_segments WHERE analysis_revision_id=?1",
                [&reply.analysis.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        let state: String = db
            .query_row(
                "SELECT publication_state FROM analysis_revisions WHERE id=?1",
                [&before.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "committed");
        assert_eq!(
            super::super::revisions::analysis_content_hash(&db, &before.id).unwrap(),
            before.content_hash
        );
    }
    #[test]
    fn manual_unknown_text_only_and_stale_requests() {
        let (_root, s, mut r) = fixture();
        let before = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let draft = s
            .create_transcript_draft(&r.meeting_id, &r.analysis_id)
            .unwrap();
        let mut payload = draft.payload;
        payload
            .utterances
            .push(super::super::drafts::UtteranceEdit {
                utterance_id: before.utterances[0].id.clone(),
                effective_text: "本文だけ修正".into(),
                speaker_ids: vec![],
            });
        payload
            .utterances
            .push(super::super::drafts::UtteranceEdit {
                utterance_id: before.utterances[2].id.clone(),
                effective_text: before.utterances[2].effective_text.clone(),
                speaker_ids: vec![],
            });
        let saved = s.save_transcript_draft(&draft.id, 0, payload).unwrap();
        assert!(s.update_speaker_assignments(&r).is_err());
        r.expected_draft = Some(DraftStamp {
            id: saved.id,
            lock_version: saved.lock_version,
        });
        let reply = s.update_speaker_assignments(&r).unwrap();
        assert_eq!(reply.updated_count, 2);
        assert_eq!(reply.analysis.utterances[0].effective_text, "本文だけ修正");
        assert!(reply.analysis.utterances[2].speaker_ids.is_empty());
        let next = Request {
            analysis_id: reply.analysis.id.clone(),
            expected_content_hash: reply.analysis.content_hash.clone(),
            expected_draft: None,
            operation_id: Uuid::new_v4().to_string(),
            ..r
        };
        assert_eq!(
            s.update_speaker_assignments(&next).unwrap().updated_count,
            0
        );
    }
}
