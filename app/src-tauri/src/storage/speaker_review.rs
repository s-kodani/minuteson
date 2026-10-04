use super::*;
use crate::diarization::{
    sherpa::Segment,
    timeline::{Attribution, Timeline},
};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
const PAGE_SIZE: usize = 50;
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SpeakerReviewRequest {
    pub meeting_id: String,
    pub analysis_id: String,
    pub expected_content_hash: String,
    pub utterance_ids: Vec<String>,
    pub include_intervals: bool,
    pub page: u32,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawSpeakerInterval {
    pub start_us: i64,
    pub end_us: i64,
    pub speaker_id: Option<String>,
    pub confidence: Option<f64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeakerOverlap {
    pub start_us: i64,
    pub end_us: i64,
    pub speaker_ids: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeakerReview {
    pub utterance_id: String,
    pub track_id: String,
    pub estimated_state: &'static str,
    pub manual_assignment: bool,
    pub segment_count: usize,
    pub overlap_count: usize,
    pub page: u32,
    pub page_count: usize,
    pub segments: Vec<RawSpeakerInterval>,
    pub overlaps: Vec<SpeakerOverlap>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeakerReviewResponse {
    pub analysis_id: String,
    pub content_hash: String,
    pub reviews: Vec<SpeakerReview>,
}
impl Storage {
    pub(crate) fn speaker_reviews(
        &self,
        request: SpeakerReviewRequest,
    ) -> Result<SpeakerReviewResponse, String> {
        let unique: BTreeSet<_> = request.utterance_ids.iter().collect();
        if unique.len() != request.utterance_ids.len()
            || unique.is_empty()
            || unique.len() > 50
            || request.page > 10000
            || (request.include_intervals && unique.len() != 1)
            || (!request.include_intervals && request.page != 0)
        {
            return Err("話者区間の表示対象・ページが不正です".into());
        }
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let hash:Option<String>=tx.query_row("SELECT a.content_hash FROM analysis_revisions a JOIN meetings m ON m.id=a.meeting_id WHERE a.id=?1 AND m.id=?2 AND (a.publication_state='candidate' OR (a.publication_state='committed' AND a.id IN (m.current_analysis_id,m.previous_analysis_id)))",params![request.analysis_id,request.meeting_id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if hash.as_deref() != Some(&request.expected_content_hash)
            || super::revisions::analysis_content_hash(&tx, &request.analysis_id)?
                != request.expected_content_hash
        {
            return Err("確認した解析版・会議・内容ハッシュが一致しません".into());
        }
        let mut reviews = Vec::new();
        for utterance in &request.utterance_ids {
            let (track,start,end,lineage):(String,i64,i64,String)=tx.query_row("SELECT track_id,start_us,end_us,lineage_id FROM utterances WHERE id=?1 AND analysis_revision_id=?2 AND meeting_id=?3",params![utterance,request.analysis_id,request.meeting_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?.ok_or("発言と解析版が一致しません")?;
            let mut query=tx.prepare("SELECT start_us,end_us,speaker_id,confidence FROM speaker_segments WHERE analysis_revision_id=?1 AND track_id=?2 AND start_us<?3 AND end_us>?4 ORDER BY start_us,end_us,id LIMIT 100001").map_err(|e|e.to_string())?;
            let raw = query
                .query_map(params![request.analysis_id, track, end, start], |r| {
                    Ok(RawSpeakerInterval {
                        start_us: r.get(0)?,
                        end_us: r.get(1)?,
                        speaker_id: r.get(2)?,
                        confidence: r.get(3)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if raw.len() > 100000 {
                return Err("表示対象の話者区間が安全上限を超えています".into());
            }
            let ids: Vec<_> = raw
                .iter()
                .filter_map(|s| s.speaker_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if ids.len() > 100 {
                return Err("表示対象の話者が安全上限を超えています".into());
            }
            let indices: HashMap<_, _> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| (id, i as u32))
                .collect();
            let segments: Vec<_> = raw
                .iter()
                .filter_map(|r| {
                    r.speaker_id.as_ref().map(|id| Segment {
                        start_us: r.start_us.max(start) - start,
                        end_us: r.end_us.min(end) - start,
                        speaker: indices[id],
                    })
                })
                .collect();
            let timeline = Timeline::build(&segments, end - start)?;
            let state = match timeline.attribute(0, end - start)? {
                Attribution::Unknown => "unknown",
                Attribution::Assigned { .. } => "assigned",
                Attribution::Overlap { .. } => "overlap",
                Attribution::NeedsSplit { .. } => "needsSplit",
            };
            let overlaps: Vec<_> = timeline
                .intervals
                .iter()
                .filter(|i| i.speakers.len() > 1)
                .collect();
            let manual:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM utterance_speakers WHERE analysis_revision_id=?1 AND utterance_id=?2 AND assignment_source='manual') OR EXISTS(SELECT 1 FROM processing_jobs j JOIN diarization_manual_assignments d ON d.analysis_id=j.input_analysis_id JOIN utterances u ON u.id=d.utterance_id AND u.analysis_revision_id=d.analysis_id WHERE j.output_analysis_id=?1 AND j.kind='diarization' AND u.lineage_id=?3)",params![request.analysis_id,utterance,lineage],|r|r.get(0)).map_err(|e|e.to_string())?;
            let page_count = raw.len().max(overlaps.len()).div_ceil(PAGE_SIZE).max(1);
            if request.include_intervals && request.page as usize >= page_count {
                return Err("話者区間のページが範囲外です".into());
            }
            let offset = request.page as usize * PAGE_SIZE;
            reviews.push(SpeakerReview {
                utterance_id: utterance.clone(),
                track_id: track,
                estimated_state: state,
                manual_assignment: manual,
                segment_count: raw.len(),
                overlap_count: overlaps.len(),
                page: request.page,
                page_count,
                segments: if request.include_intervals {
                    raw.into_iter().skip(offset).take(PAGE_SIZE).collect()
                } else {
                    vec![]
                },
                overlaps: if request.include_intervals {
                    overlaps
                        .into_iter()
                        .skip(offset)
                        .take(PAGE_SIZE)
                        .map(|i| SpeakerOverlap {
                            start_us: start + i.start_us,
                            end_us: start + i.end_us,
                            speaker_ids: i
                                .speakers
                                .iter()
                                .map(|s| ids[*s as usize].clone())
                                .collect(),
                        })
                        .collect()
                } else {
                    vec![]
                },
            });
        }
        Ok(SpeakerReviewResponse {
            analysis_id: request.analysis_id,
            content_hash: request.expected_content_hash,
            reviews,
        })
    }
}
