use super::analysis::{AnalysisSpeaker, AnalysisUtterance};
use super::*;
use crate::minutes_document::Evidence;
use crate::playback::PreparedAudio;
use rusqlite::{Transaction, TransactionBehavior};
use serde::Deserialize;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesEvidenceRequest {
    pub meeting_id: String,
    pub minutes_id: String,
    pub expected_content_hash: String,
    pub expected_input_content_hash: String,
    pub expected_evidence: Evidence,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesEvidenceSource {
    pub evidence: Evidence,
    pub analysis_id: String,
    pub analysis_content_hash: String,
    pub speakers: Vec<AnalysisSpeaker>,
    pub utterances: Vec<AnalysisUtterance>,
    pub first_sequence: usize,
    pub total_utterances: usize,
}

const TRANSCRIPT_PAGE_SIZE: usize = 50;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesTranscriptRequest {
    pub reference: MinutesEvidenceRequest,
    // None locates the page containing the referenced utterance.
    pub page: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesTranscriptPage {
    #[serde(flatten)]
    pub source: MinutesEvidenceSource,
    pub page: usize,
    pub page_size: usize,
    pub target_sequence: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesTranscriptAudioRequest {
    pub reference: MinutesEvidenceRequest,
    pub utterance_id: String,
    pub expected_start_us: i64,
    pub expected_end_us: i64,
}

fn fixed_reference(
    tx: &Transaction<'_>,
    request: &MinutesEvidenceRequest,
) -> Result<(Evidence, MinutesInput), String> {
    let view = super::minutes::read_content(&tx, &request.meeting_id, &request.minutes_id)?
        .ok_or("参照する議事録版がありません")?;
    if view.content_hash != request.expected_content_hash
        || view.input_content_hash != request.expected_input_content_hash
    {
        return Err("参照する議事録内容が変更されています".into());
    }
    // A candidate or a previous version is resolved by its own base, never
    // by the meeting's current analysis head or the currently active draft.
    let draft = super::minutes_drafts::load(&tx, &request.meeting_id, Some(&view.id))?;
    let content = draft.as_ref().map(|d| &d.payload).unwrap_or(&view.content);
    let e = content
        .evidence
        .iter()
        .find(|e| e.id == request.expected_evidence.id)
        .ok_or("この議事録の保存済み根拠がありません。編集を保存して確認し直してください")?;
    let expected = &request.expected_evidence;
    if e.minutes_item_id != expected.minutes_item_id
        || e.analysis_revision_id != expected.analysis_revision_id
        || e.utterance_id != expected.utterance_id
        || e.quote_text != expected.quote_text
        || e.start_us != expected.start_us
        || e.end_us != expected.end_us
    {
        return Err("確認した根拠の参照先・引用・時刻が変更されています".into());
    }
    let input = super::minutes_input::read_snapshot(
        &tx,
        &request.meeting_id,
        &view.input_analysis_id,
        &view.input_content_hash,
    )?;
    content.validate(&input)?;
    Ok((e.clone(), input))
}

impl Storage {
    pub(crate) fn minutes_evidence_source(
        &self,
        request: &MinutesEvidenceRequest,
    ) -> Result<MinutesEvidenceSource, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (e, input) = fixed_reference(&tx, request)?;
        let index = input
            .analysis
            .utterances
            .iter()
            .position(|u| u.id == e.utterance_id)
            .ok_or("固定された元発言がありません")?;
        let first = index.saturating_sub(2);
        let last = (index + 3).min(input.analysis.utterances.len());
        Ok(MinutesEvidenceSource {
            evidence: e,
            analysis_id: input.analysis.id,
            analysis_content_hash: input.analysis.content_hash,
            speakers: input.analysis.speakers,
            total_utterances: input.analysis.utterances.len(),
            utterances: input.analysis.utterances[first..last].to_vec(),
            first_sequence: first + 1,
        })
    }

    pub(crate) fn minutes_transcript_page(
        &self,
        request: &MinutesTranscriptRequest,
    ) -> Result<MinutesTranscriptPage, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (evidence, input) = fixed_reference(&tx, &request.reference)?;
        let utterances = &input.analysis.utterances;
        let target = utterances
            .iter()
            .position(|u| u.id == evidence.utterance_id)
            .ok_or("固定された元発言がありません")?;
        let page = request
            .page
            .map(|p| p as usize)
            .unwrap_or(target / TRANSCRIPT_PAGE_SIZE);
        let first = page
            .checked_mul(TRANSCRIPT_PAGE_SIZE)
            .filter(|&i| i < utterances.len())
            .ok_or("固定文字起こしのページが範囲外です")?;
        let last = (first + TRANSCRIPT_PAGE_SIZE).min(utterances.len());
        Ok(MinutesTranscriptPage {
            source: MinutesEvidenceSource {
                evidence,
                analysis_id: input.analysis.id,
                analysis_content_hash: input.analysis.content_hash,
                speakers: input.analysis.speakers,
                utterances: utterances[first..last].to_vec(),
                first_sequence: first + 1,
                total_utterances: utterances.len(),
            },
            page,
            page_size: TRANSCRIPT_PAGE_SIZE,
            target_sequence: target + 1,
        })
    }

    pub(crate) fn prepare_minutes_transcript_audio(
        &self,
        request: &MinutesTranscriptAudioRequest,
    ) -> Result<PreparedAudio, String> {
        let (track, start, end) = {
            let mut db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(|e| e.to_string())?;
            let (_, input) = fixed_reference(&tx, &request.reference)?;
            let u = input
                .analysis
                .utterances
                .iter()
                .find(|u| u.id == request.utterance_id)
                .ok_or("この固定文字起こしに発言がありません")?;
            if (u.start_us, u.end_us) != (request.expected_start_us, request.expected_end_us) {
                return Err("固定発言と確認した再生区間が一致しません".into());
            }
            (u.track_id.clone(), u.start_us, u.end_us)
        };
        // Full-workspace playback still requires a verified minutes reference;
        // arbitrary internal-snapshot playback remains inaccessible.
        self.prepare_track_audio(&request.reference.meeting_id, &track, start, end)
    }

    pub(crate) fn prepare_minutes_evidence_audio(
        &self,
        request: &MinutesEvidenceRequest,
    ) -> Result<PreparedAudio, String> {
        let source = self.minutes_evidence_source(request)?;
        let utterance = source
            .utterances
            .iter()
            .find(|u| u.id == source.evidence.utterance_id)
            .ok_or("根拠の元発言がありません")?;
        // Internal snapshots are not enabled for arbitrary-utterance playback.
        // Only this verified minutes reference authorizes its bounded interval.
        self.prepare_track_audio(
            &request.meeting_id,
            &utterance.track_id,
            source.evidence.start_us,
            source.evidence.end_us,
        )
    }
}

#[cfg(test)]
#[path = "minutes_evidence_tests.rs"]
mod tests;
