use super::analysis::{AnalysisSpeaker, AnalysisUtterance};
use super::*;
use crate::minutes_document::Evidence;
use crate::playback::PreparedAudio;
use rusqlite::TransactionBehavior;
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
        let index = input
            .analysis
            .utterances
            .iter()
            .position(|u| u.id == e.utterance_id)
            .ok_or("固定された元発言がありません")?;
        let first = index.saturating_sub(2);
        let last = (index + 3).min(input.analysis.utterances.len());
        Ok(MinutesEvidenceSource {
            evidence: e.clone(),
            analysis_id: input.analysis.id,
            analysis_content_hash: input.analysis.content_hash,
            speakers: input.analysis.speakers,
            total_utterances: input.analysis.utterances.len(),
            utterances: input.analysis.utterances[first..last].to_vec(),
            first_sequence: first + 1,
        })
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
