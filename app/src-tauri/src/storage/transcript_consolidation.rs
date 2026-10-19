//! Persist continuous turns as ordinary utterances in a new immutable revision.
use super::analysis::AnalysisUtterance;
use super::*;
use rusqlite::{Transaction, TransactionBehavior};
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Request {
    pub meeting_id: String,
    pub analysis_id: String,
    pub expected_content_hash: String,
    pub expected_draft: Option<DraftStamp>,
    pub operation_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    pub analysis: AnalysisView,
    pub draft: Option<TranscriptDraft>,
    pub changed: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(in crate::storage) struct Provenance {
    pub schema_version: u32,
    pub operation_id: String,
    pub source_draft: Option<DraftStamp>,
    pub groups: Vec<SourceGroup>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(in crate::storage) struct SourceGroup {
    pub lineage_id: String,
    pub source_lineages: Vec<String>,
}

fn word_edge(c: char) -> bool {
    c.is_numeric()
        || c.is_alphabetic()
            && matches!(c as u32,
        0x41..=0x7a | 0xc0..=0x2af | 0x1e00..=0x1eff | 0xa720..=0xa7ff |
        0xab30..=0xab6f | 0xff21..=0xff5a | 0x10780..=0x107bf | 0x1df00..=0x1dfff)
}
fn separator(a: &str, b: &str) -> &'static str {
    if a.chars().next_back().is_some_and(word_edge) && b.chars().next().is_some_and(word_edge) {
        " "
    } else {
        ""
    }
}
fn joined(rows: &[AnalysisUtterance], recognized: bool) -> String {
    let mut result = String::new();
    for row in rows {
        let text = if recognized {
            &row.recognized_text
        } else {
            &row.effective_text
        };
        result.push_str(separator(&result, text));
        result.push_str(text);
    }
    result
}

fn groups(rows: &[AnalysisUtterance], gaps: &[(String, i64, i64)]) -> Vec<Vec<AnalysisUtterance>> {
    let mut by_track: HashMap<&str, Vec<(i64, i64)>> = HashMap::new();
    for (track, start, end) in gaps {
        by_track.entry(track).or_default().push((*start, *end));
    }
    for ranges in by_track.values_mut() {
        ranges.sort();
        let mut union: Vec<(i64, i64)> = Vec::new();
        for &(start, end) in ranges.iter() {
            if let Some(last) = union.last_mut().filter(|last| last.1 >= start) {
                last.1 = last.1.max(end);
            } else {
                union.push((start, end));
            }
        }
        *ranges = union;
    }
    let mut result: Vec<Vec<AnalysisUtterance>> = Vec::new();
    let (mut effective_chars, mut recognized_chars) = (0, 0);
    for row in rows {
        let (chars, recognized) = (
            row.effective_text.chars().count(),
            row.recognized_text.chars().count(),
        );
        let group = result.last();
        let previous = group.and_then(|g| g.last());
        let extra = previous.map_or(0, |p| {
            separator(&p.effective_text, &row.effective_text).len()
        });
        let recognized_extra = previous.map_or(0, |p| {
            separator(&p.recognized_text, &row.recognized_text).len()
        });
        let eligible = group.zip(previous).is_some_and(|(g, p)| {
            let has_gap = by_track.get(row.track_id.as_str()).is_some_and(|ranges| {
                let index = ranges.partition_point(|range| range.1 <= g[0].start_us);
                ranges.get(index).is_some_and(|range| range.0 < row.end_us)
            });
            p.track_id == row.track_id
                && !row.track_id.is_empty()
                && p.attribution_state == "assigned"
                && row.attribution_state == "assigned"
                && p.speaker_ids.len() == 1
                && row.speaker_ids == p.speaker_ids
                && !p.recognition_is_source
                && !row.recognition_is_source
                && p.speaker_identification.is_none()
                && row.speaker_identification.is_none()
                && row.start_us >= p.end_us
                && row.start_us - p.end_us <= 2_000_000
                && row.end_us - g[0].start_us <= 60_000_000
                && !has_gap
                && effective_chars + chars + extra <= 20_000
                && recognized_chars + recognized + recognized_extra <= 20_000
        });
        if eligible {
            result.last_mut().unwrap().push(row.clone());
            effective_chars += chars + extra;
            recognized_chars += recognized + recognized_extra;
        } else {
            result.push(vec![row.clone()]);
            effective_chars = chars;
            recognized_chars = recognized;
        }
    }
    result.into_iter().filter(|g| g.len() > 1).collect()
}

pub(super) fn consolidate_copied_rows(
    tx: &Transaction<'_>,
    source: &AnalysisView,
    payload: &TranscriptDraftPayload,
    id: &str,
    request: &Request,
    groups: &[Vec<AnalysisUtterance>],
) -> Result<(), String> {
    let lineages = super::minutes_input::lineage_map(tx, "utterances", &source.id)?;
    let mut provenance = Provenance {
        schema_version: 1,
        operation_id: request.operation_id.clone(),
        source_draft: request.expected_draft.clone(),
        groups: Vec::new(),
    };
    for group in groups {
        let originals = group
            .iter()
            .map(|row| {
                lineages
                    .get(&row.id)
                    .cloned()
                    .ok_or("統合元の発言の由来がありません")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let targets = originals
            .iter()
            .map(|lineage| {
                tx.query_row(
                    "SELECT id FROM utterances WHERE analysis_revision_id=?1 AND lineage_id=?2",
                    params![id, lineage],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let lineage = Uuid::new_v4().to_string();
        tx.execute("UPDATE utterance_speakers SET assignment_source='manual' WHERE utterance_id=?1 AND analysis_revision_id=?2 AND EXISTS(SELECT 1 FROM utterance_speakers WHERE analysis_revision_id=?2 AND utterance_id IN (SELECT value FROM json_each(?3)) AND assignment_source='manual')",params![targets[0],id,serde_json::to_string(&targets).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE utterances SET lineage_id=?1,start_us=?2,end_us=?3,recognized_text=?4,effective_text=?5,confidence=NULL WHERE id=?6 AND analysis_revision_id=?7",params![lineage,group[0].start_us,group.last().unwrap().end_us,joined(group,true),joined(group,false),targets[0],id]).map_err(|e|e.to_string())?;
        for removed in &targets[1..] {
            tx.execute(
                "DELETE FROM utterance_speakers WHERE utterance_id=?1 AND analysis_revision_id=?2",
                params![removed, id],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM utterances WHERE id=?1 AND analysis_revision_id=?2",
                params![removed, id],
            )
            .map_err(|e| e.to_string())?;
        }
        provenance.groups.push(SourceGroup {
            lineage_id: lineage,
            source_lineages: originals,
        });
    }
    let mut speaker_lineages = super::minutes_input::lineage_map(tx, "speakers", &source.id)?;
    for speaker in &payload.manual_speakers {
        speaker_lineages.insert(speaker.id.clone(), speaker.id.clone());
    }
    for merge in &payload.speaker_merges {
        let lookup = |old: &str| -> Result<String, String> {
            let lineage = speaker_lineages
                .get(old)
                .ok_or("統合話者の由来がありません")?;
            tx.query_row(
                "SELECT id FROM speakers WHERE analysis_revision_id=?1 AND lineage_id=?2",
                params![id, lineage],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
        };
        let (removed, target) = (lookup(&merge.source_id)?, lookup(&merge.target_id)?);
        let referenced:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM utterance_speakers WHERE analysis_revision_id=?1 AND speaker_id=?2)",params![id,removed],|r|r.get(0)).map_err(|e|e.to_string())?;
        if referenced {
            return Err("統合元の話者に未適用の発言があります".into());
        }
        tx.execute("UPDATE speaker_segments SET speaker_id=?1 WHERE analysis_revision_id=?2 AND speaker_id=?3",params![target,id,removed]).map_err(|e|e.to_string())?;
        tx.execute(
            "DELETE FROM speakers WHERE id=?1 AND analysis_revision_id=?2",
            params![removed, id],
        )
        .map_err(|e| e.to_string())?;
    }
    let text: String = tx
        .query_row(
            "SELECT config_json FROM analysis_revisions WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut config: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    // This revision is public, not a fixed job input. The normal parent FK
    // keeps its source; an inputSnapshot would falsely require an input row.
    config
        .as_object_mut()
        .ok_or("解析設定が不正です")?
        .remove("inputSnapshot");
    config["utteranceConsolidation"] =
        serde_json::to_value(provenance).map_err(|e| e.to_string())?;
    if config.to_string().len() > 2 * 1024 * 1024 {
        return Err("発言統合の履歴が大きすぎます".into());
    }
    tx.execute(
        "UPDATE analysis_revisions SET config_json=?1 WHERE id=?2 AND publication_state='building'",
        params![config.to_string(), id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn copy_voice_matches(
    tx: &Transaction<'_>,
    source: &AnalysisView,
    id: &str,
) -> Result<(), String> {
    // Speaker lineages remain 1:1; carry confirmed/candidate person links.
    let matches = {
        let mut q=tx.prepare("SELECT v.person_id,v.name_snapshot,v.state,v.similarity,v.model_hash,v.policy_version,v.created_at,n.id FROM voice_matches v JOIN speakers s ON s.id=v.speaker_id AND s.analysis_revision_id=v.analysis_revision_id JOIN speakers n ON n.lineage_id=s.lineage_id AND n.analysis_revision_id=?1 WHERE v.analysis_revision_id=?2").map_err(|e|e.to_string())?;
        let rows = q
            .query_map(params![id, source.id], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    for (person, name, state, similarity, model, policy, created, speaker) in matches {
        tx.execute("INSERT INTO voice_matches(id,meeting_id,analysis_revision_id,speaker_id,person_id,name_snapshot,state,similarity,model_hash,policy_version,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![Uuid::new_v4().to_string(),source.meeting_id,id,speaker,person,name,state,similarity,model,policy,created]).map_err(|e|e.to_string())?;
    }
    Ok(())
}

impl Storage {
    pub(crate) fn consolidate_transcript(&self, request: &Request) -> Result<Reply, String> {
        if Uuid::parse_str(&request.operation_id).is_err() {
            return Err("発言統合の操作IDが不正です".into());
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
            let receipt = config
                .get("utteranceConsolidation")
                .cloned()
                .map(serde_json::from_value::<Provenance>)
                .transpose()
                .map_err(|e| e.to_string())?;
            if parent.as_deref() == Some(request.analysis_id.as_str())
                && hash.as_deref() == Some(request.expected_content_hash.as_str())
                && receipt.is_some_and(|p| {
                    p.operation_id == request.operation_id
                        && p.source_draft == request.expected_draft
                })
                && draft.is_none()
            {
                return Ok(Reply {
                    analysis: source,
                    draft: None,
                    changed: true,
                });
            }
            return Err("発言統合の対象版が変わりました。再読込してください".into());
        }
        if source.content_hash != request.expected_content_hash || stamp != request.expected_draft {
            return Err("発言統合の前に編集内容が変更されています。再読込してください".into());
        }
        let payload = draft
            .as_ref()
            .map(|d| d.payload.clone())
            .unwrap_or_default();
        let base = source.clone();
        super::minutes_input::apply_edits(&mut source, &payload);
        let gaps = {
            let mut q=tx.prepare("SELECT g.track_id,g.start_us,g.end_us FROM audio_gaps g JOIN audio_tracks t ON t.id=g.track_id WHERE t.meeting_id=?1 AND g.recovered=0 ORDER BY g.track_id,g.start_us").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([&request.meeting_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let groups = groups(&source.utterances, &gaps);
        let voice_busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE meeting_id=?1 AND input_analysis_id=?2 AND kind IN ('voice_registration','voice_matching','speaker_completion') AND state IN ('queued','running','pausing','paused','interrupted'))",params![request.meeting_id,source.id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if groups.is_empty() || voice_busy {
            return Ok(Reply {
                analysis: base,
                draft,
                changed: false,
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
        let id = super::minutes_input::copy_snapshot_with_consolidation(
            &tx,
            &source,
            &payload,
            &title,
            created,
            started,
            &working,
            None,
            Some((request, &groups)),
        )?;
        copy_voice_matches(&tx, &source, &id)?;
        tx.execute("UPDATE working_drafts SET active=0 WHERE meeting_id=?1 AND target_kind='transcript' AND active=1",[&request.meeting_id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE meetings SET current_analysis_id=?1,previous_analysis_id=?2,lock_version=lock_version+1,updated_at=?3 WHERE id=?4 AND current_analysis_id=?2",params![id,source.id,now_ms()?,request.meeting_id]).map_err(|e|e.to_string())?;
        let hash = super::revisions::analysis_content_hash(&tx, &id)?;
        let analysis =
            super::analysis::read_analysis_content(&tx, &request.meeting_id, &id, &hash)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Reply {
            analysis,
            draft: None,
            changed: true,
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Storage, Request) {
        let root = tempfile::tempdir().unwrap();
        let s = Storage::open(root.path().into()).unwrap();
        let (meeting, _, _) = super::super::playback::tests::fixture(&s);
        let id = s
            .create_analysis_candidate(
                &meeting,
                AnalysisInput {
                    audio_manifest_hash: "a".repeat(64),
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
                            end_us: 250000,
                            text: "本日は".into(),
                            speaker_keys: vec!["a".into()],
                            confidence: None,
                        },
                        UtteranceInput {
                            track_id: "track".into(),
                            start_us: 250000,
                            end_us: 500000,
                            text: "点検日です。".into(),
                            speaker_keys: vec!["a".into()],
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
    #[test]
    fn consolidation_persists_one_record_with_complete_text_and_bounds_and_keeps_old_revision() {
        let (_root, s, r) = fixture();
        let original = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let reply = s.consolidate_transcript(&r).unwrap();
        assert!(reply.changed);
        assert_ne!(reply.analysis.id, r.analysis_id);
        assert!(reply.draft.is_none());
        assert_eq!(reply.analysis.utterances.len(), 2);
        let first = &reply.analysis.utterances[0];
        assert_eq!((first.start_us, first.end_us), (0, 500000));
        assert_eq!(first.effective_text, "本日は点検日です。");
        assert_eq!(first.recognized_text, first.effective_text);
        assert_eq!(first.speaker_ids.len(), 1);
        let db = s.connection.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM utterances WHERE analysis_revision_id=?1",
                [&reply.analysis.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(
            super::super::analysis::read_analysis_content(
                &db,
                &r.meeting_id,
                &r.analysis_id,
                &r.expected_content_hash
            )
            .unwrap(),
            original
        );
        let old_lineage: String = db
            .query_row(
                "SELECT lineage_id FROM utterances WHERE id=?1",
                [&original.utterances[0].id],
                |r| r.get(0),
            )
            .unwrap();
        let new_lineage: String = db
            .query_row(
                "SELECT lineage_id FROM utterances WHERE id=?1",
                [&first.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(old_lineage, new_lineage);
        drop(db);
        assert!(s.verify_analysis_content_hash(&reply.analysis.id).unwrap());
        assert!(s.verify_analysis_content_hash(&r.analysis_id).unwrap());
        drop(s);
        let reopened = Storage::open(_root.path().into()).unwrap();
        assert_eq!(
            reopened
                .load_current_analysis(&r.meeting_id)
                .unwrap()
                .unwrap(),
            reply.analysis
        );
    }
    #[test]
    fn consolidation_reuses_a_lost_receipt_and_rejects_stale_or_foreign_inputs() {
        let (_root, s, r) = fixture();
        let mut wrong = r.clone();
        wrong.expected_content_hash = "f".repeat(64);
        assert!(s.consolidate_transcript(&wrong).is_err());
        wrong = r.clone();
        wrong.expected_draft = Some(DraftStamp {
            id: "missing".into(),
            lock_version: 0,
        });
        assert!(s.consolidate_transcript(&wrong).is_err());
        let first = s.consolidate_transcript(&r).unwrap();
        assert_eq!(
            s.consolidate_transcript(&r).unwrap().analysis,
            first.analysis
        );
        let latest = Request {
            analysis_id: first.analysis.id,
            expected_content_hash: first.analysis.content_hash,
            operation_id: Uuid::new_v4().to_string(),
            ..r.clone()
        };
        assert!(!s.consolidate_transcript(&latest).unwrap().changed);
        wrong = r.clone();
        wrong.operation_id = Uuid::new_v4().to_string();
        assert!(s.consolidate_transcript(&wrong).is_err());
        wrong = r;
        wrong.meeting_id = s.create_meeting("別の会議").unwrap().id;
        assert!(s.consolidate_transcript(&wrong).is_err());
    }
    #[test]
    fn consolidation_keeps_saved_edits_and_manual_splits_remain_separate() {
        let (_root, s, mut r) = fixture();
        let before = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let draft = s
            .create_transcript_draft(&r.meeting_id, &r.analysis_id)
            .unwrap();
        let mut payload = draft.payload;
        payload
            .speaker_names
            .push(super::super::drafts::SpeakerNameEdit {
                speaker_id: before.speakers[0].id.clone(),
                display_name: Some("手動の名前".into()),
            });
        payload
            .utterances
            .push(super::super::drafts::UtteranceEdit {
                utterance_id: before.utterances[0].id.clone(),
                effective_text: "修正した".into(),
                speaker_ids: before.utterances[0].speaker_ids.clone(),
            });
        let saved = s.save_transcript_draft(&draft.id, 0, payload).unwrap();
        r.expected_draft = Some(DraftStamp {
            id: saved.id.clone(),
            lock_version: saved.lock_version,
        });
        let next = s.consolidate_transcript(&r).unwrap().analysis;
        assert_eq!(next.utterances[0].effective_text, "修正した点検日です。");
        assert!(next
            .speakers
            .iter()
            .any(|s| s.display_name.as_deref() == Some("手動の名前")));
        assert!(s.load_transcript_draft(&r.meeting_id).unwrap().is_none());
        assert_eq!(
            s.connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT active FROM working_drafts WHERE id=?1",
                    [&saved.id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let draft = s.create_transcript_draft(&r.meeting_id, &next.id).unwrap();
        let first = &next.utterances[0];
        let text = first.effective_text.clone();
        let mut p = draft.payload;
        p.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
        p.utterance_splits
            .push(super::super::draft_splits::UtteranceSplit {
                source_utterance_id: first.id.clone(),
                source_effective_text: text.clone(),
                parts: vec![
                    super::super::draft_splits::SplitPart {
                        id: format!("split:{}", Uuid::new_v4()),
                        start_us: 0,
                        end_us: 250000,
                        effective_text: "修正した".into(),
                        speaker_ids: first.speaker_ids.clone(),
                    },
                    super::super::draft_splits::SplitPart {
                        id: format!("split:{}", Uuid::new_v4()),
                        start_us: 250000,
                        end_us: 500000,
                        effective_text: "点検日です。".into(),
                        speaker_ids: first.speaker_ids.clone(),
                    },
                ],
            });
        let saved = s.save_transcript_draft(&draft.id, 0, p).unwrap();
        let request = Request {
            analysis_id: next.id,
            expected_content_hash: next.content_hash,
            expected_draft: Some(DraftStamp {
                id: saved.id,
                lock_version: saved.lock_version,
            }),
            operation_id: Uuid::new_v4().to_string(),
            ..r
        };
        assert!(!s.consolidate_transcript(&request).unwrap().changed);
    }
    #[test]
    fn consolidation_keeps_registered_features_source_audio_and_person_matches() {
        let (_root, s, r) = fixture();
        let before = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let speaker = &before.utterances[0].speaker_ids[0];
        {
            let db = s.connection.lock().unwrap();
            db.execute("INSERT INTO voice_people VALUES('person','登録者',1,0)", [])
                .unwrap();
            db.execute("INSERT INTO voice_samples VALUES('sample','person',?1,?2,2,?3,?4,?5,'track',0,250000,0)",params!["a".repeat(64),crate::voice::EXTRACTOR,crate::voice::encode(&[1.,0.]).unwrap(),r.meeting_id,r.analysis_id]).unwrap();
            db.execute("INSERT INTO voice_matches VALUES('match',?1,?2,?3,'person','登録者','confirmed',NULL,?4,'manual',0)",params![r.meeting_id,r.analysis_id,speaker,"a".repeat(64)]).unwrap();
        }
        let next = s.consolidate_transcript(&r).unwrap().analysis;
        let people = s.voice_people().unwrap();
        assert_eq!(
            people[0].samples[0].source_meeting_id.as_deref(),
            Some(r.meeting_id.as_str())
        );
        let audio = s
            .prepare_voice_sample_audio_cancellable("sample", &crate::test_support::never_cancelled)
            .unwrap();
        assert_eq!(audio.total_frames(), 12000);
        assert_eq!(s.connection.lock().unwrap().query_row("SELECT COUNT(*) FROM voice_matches WHERE analysis_revision_id=?1 AND person_id='person' AND state='confirmed'",[&next.id],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    #[test]
    fn consolidation_keeps_a_saved_speaker_merge_in_the_new_current_revision() {
        let (_root, s, mut r) = fixture();
        let view = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let (source, target) = (
            view.utterances[0].speaker_ids[0].clone(),
            view.utterances[2].speaker_ids[0].clone(),
        );
        let draft = s
            .create_transcript_draft(&r.meeting_id, &r.analysis_id)
            .unwrap();
        let mut payload = draft.payload;
        payload.schema_version = crate::current_formats::TRANSCRIPT_DRAFT;
        payload
            .speaker_merges
            .push(super::super::drafts::SpeakerMerge {
                source_id: source,
                target_id: target,
            });
        let saved = s.save_transcript_draft(&draft.id, 0, payload).unwrap();
        r.expected_draft = Some(DraftStamp {
            id: saved.id,
            lock_version: saved.lock_version,
        });
        let reply = s.consolidate_transcript(&r).unwrap();
        assert_eq!(reply.analysis.utterances.len(), 1);
        assert_eq!(reply.analysis.speakers.len(), 1);
        assert_eq!(
            reply.analysis.utterances[0].speaker_ids,
            vec![reply.analysis.speakers[0].id.clone()]
        );
        assert!(s.load_transcript_draft(&r.meeting_id).unwrap().is_none());
        assert_eq!(s.connection.lock().unwrap().query_row("SELECT COUNT(DISTINCT speaker_id) FROM speaker_segments WHERE analysis_revision_id=?1",[&reply.analysis.id],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    #[test]
    fn consolidation_rolls_back_the_entire_copy_when_head_update_fails() {
        let (_root, s, r) = fixture();
        let before = s.load_current_analysis(&r.meeting_id).unwrap().unwrap();
        let count = s
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM analysis_revisions", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        s.connection.lock().unwrap().execute_batch("CREATE TRIGGER reject_consolidation BEFORE UPDATE OF current_analysis_id ON meetings BEGIN SELECT RAISE(ABORT,'synthetic head failure'); END;").unwrap();
        assert!(s.consolidate_transcript(&r).is_err());
        assert_eq!(
            s.load_current_analysis(&r.meeting_id).unwrap().unwrap(),
            before
        );
        assert_eq!(
            s.connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM analysis_revisions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            count
        );
    }
    #[test]
    fn consolidation_boundaries_keep_turns_tracks_overlaps_pauses_gaps_and_manual_parts() {
        let row = |id: &str, start: i64| AnalysisUtterance {
            speaker_identification: None,
            id: id.into(),
            track_id: "track".into(),
            start_us: start,
            end_us: start + 1000000,
            recognized_text: "Hello".into(),
            effective_text: "Hello".into(),
            attribution_state: "assigned".into(),
            speaker_ids: vec!["a".into()],
            recognition_is_source: false,
        };
        let a = row("a", 0);
        let b = row("b", 1000000);
        assert_eq!(
            joined(&groups(&[a.clone(), b.clone()], &[])[0], false),
            "Hello Hello"
        );
        for modified in [
            AnalysisUtterance {
                track_id: "other".into(),
                ..b.clone()
            },
            AnalysisUtterance {
                speaker_ids: vec!["b".into()],
                ..b.clone()
            },
            AnalysisUtterance {
                speaker_ids: vec![],
                attribution_state: "unknown".into(),
                ..b.clone()
            },
            AnalysisUtterance {
                start_us: 999999,
                ..b.clone()
            },
            AnalysisUtterance {
                start_us: 3000001,
                end_us: 4000001,
                ..b.clone()
            },
            AnalysisUtterance {
                recognition_is_source: true,
                ..b.clone()
            },
            AnalysisUtterance {
                speaker_identification: Some(super::super::analysis::SpeakerIdentification {
                    similarity_milli: 530,
                    runner_up_milli: Some(200),
                    needs_review: true,
                    reason: "nearest_below_threshold".into(),
                }),
                ..b.clone()
            },
        ] {
            assert!(groups(&[a.clone(), modified], &[]).is_empty());
        }
        let b = row("b", 2000000);
        assert!(groups(&[a.clone(), b], &[("track".into(), 1000000, 1500000)]).is_empty());
        let rows = (0..61)
            .map(|i| row(&i.to_string(), i * 1000000))
            .collect::<Vec<_>>();
        assert_eq!(
            groups(&rows, &[]).iter().map(Vec::len).collect::<Vec<_>>(),
            vec![60]
        );
        assert_eq!(separator("本日は", "点検日。"), "");
        assert_eq!(separator("café", "été"), " ");
    }
}
