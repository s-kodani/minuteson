use super::*;
use rusqlite::TransactionBehavior;

#[cfg(test)]
#[path = "analysis_ui_tests.rs"]
mod tests;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DraftStamp {
    pub id: String,
    pub lock_version: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSummary {
    pub id: String,
    pub revision_no: i64,
    pub content_hash: String,
    pub publication_state: String,
    pub utterance_count: i64,
    pub created_at: i64,
    pub profile: serde_json::Value,
    pub config: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionState {
    pub meeting_lock_version: i64,
    pub current_id: Option<String>,
    pub previous_id: Option<String>,
    pub draft: Option<DraftStamp>,
    pub versions: Vec<VersionSummary>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AdoptRequest {
    pub meeting_id: String,
    pub revision_id: String,
    pub expected_content_hash: String,
    pub expected_meeting_lock_version: i64,
    pub expected_draft: Option<DraftStamp>,
    pub confirm_replacing_previous: bool,
}

impl Storage {
    pub(crate) fn analysis_versions(&self, meeting_id: &str) -> Result<VersionState, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (meeting_lock_version, current_id, previous_id) = transaction.query_row("SELECT lock_version, current_analysis_id, previous_analysis_id FROM meetings WHERE id = ?1", params![meeting_id], |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?))).optional().map_err(|e| e.to_string())?.ok_or("会議が見つかりません")?;
        let draft = super::drafts::load_draft(&transaction, meeting_id)?.map(|draft| DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        });
        let mut query = transaction.prepare("SELECT a.id, a.revision_no, a.content_hash, a.publication_state, (SELECT COUNT(*) FROM utterances WHERE analysis_revision_id = a.id), a.created_at, a.profile_snapshot_json, a.config_json FROM analysis_revisions a WHERE a.meeting_id = ?1 AND (a.id IN (?2, ?3) OR a.id IN (SELECT id FROM analysis_revisions WHERE meeting_id = ?1 AND publication_state = 'candidate' ORDER BY revision_no DESC LIMIT 20) OR (a.publication_state='candidate' AND a.id IN (SELECT j.output_analysis_id FROM analysis_flows f JOIN processing_jobs j ON j.id IN (f.transcription_job_id,f.diarization_job_id) WHERE f.meeting_id=?1 AND j.state='succeeded'))) ORDER BY a.revision_no DESC").map_err(|e| e.to_string())?;
        let rows = query
            .query_map(params![meeting_id, current_id, previous_id], |row| {
                let profile: String = row.get(6)?;
                let config: String = row.get(7)?;
                let decode = |index, error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        index,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                };
                Ok(VersionSummary {
                    id: row.get(0)?,
                    revision_no: row.get(1)?,
                    content_hash: row.get(2)?,
                    publication_state: row.get(3)?,
                    utterance_count: row.get(4)?,
                    created_at: row.get(5)?,
                    profile: serde_json::from_str(&profile).map_err(|e| decode(6, e))?,
                    config: serde_json::from_str(&config).map_err(|e| decode(7, e))?,
                })
            })
            .map_err(|e| e.to_string())?;
        let versions = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(VersionState {
            meeting_lock_version,
            current_id,
            previous_id,
            draft,
            versions,
        })
    }

    pub(crate) fn adopt_analysis(
        &self,
        request: AdoptRequest,
    ) -> Result<super::revisions::CommittedAnalysis, String> {
        self.commit_analysis_inner(
            &request.meeting_id,
            &request.revision_id,
            request.expected_meeting_lock_version,
            request.confirm_replacing_previous,
            Some((
                &request.expected_content_hash,
                request.expected_draft.as_ref(),
            )),
        )
    }

    pub(crate) fn restore_analysis_checked(
        &self,
        meeting_id: &str,
        expected_lock: i64,
        expected_previous_id: &str,
        expected_content_hash: &str,
        expected_draft: Option<DraftStamp>,
    ) -> Result<super::revisions::CommittedAnalysis, String> {
        self.restore_analysis_inner(
            meeting_id,
            expected_lock,
            Some((
                expected_previous_id,
                expected_content_hash,
                expected_draft.as_ref(),
            )),
        )
    }

    pub(crate) fn visible_analysis_draft(
        &self,
        meeting_id: &str,
        analysis_id: &str,
    ) -> Result<Option<TranscriptDraft>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let visible: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1 AND ?2 IN (current_analysis_id, previous_analysis_id))", params![meeting_id, analysis_id], |row| row.get(0)).map_err(|e| e.to_string())?;
        if !visible {
            return Err("現在版・直前版以外の下書きは表示できません".into());
        }
        super::drafts::load_draft_for_base(&connection, meeting_id, Some(analysis_id))
    }
}

pub(super) fn switch_draft(
    connection: &Connection,
    meeting_id: &str,
    next_base: &str,
    expected: Option<&DraftStamp>,
) -> Result<(), String> {
    let current = super::drafts::load_draft(connection, meeting_id)?;
    let actual = current.as_ref().map(|draft| DraftStamp {
        id: draft.id.clone(),
        lock_version: draft.lock_version,
    });
    if actual.as_ref() != expected {
        return Err("確認後に編集下書きが変更されました。保存後にもう一度確認してください".into());
    }
    // Read/hash-check the archived draft before reactivating it.
    super::drafts::load_draft_for_base(connection, meeting_id, Some(next_base))?;
    connection.execute("UPDATE working_drafts SET active = 0 WHERE meeting_id = ?1 AND target_kind = 'transcript' AND active = 1", params![meeting_id]).map_err(|e| e.to_string())?;
    connection.execute("UPDATE working_drafts SET active = 1 WHERE meeting_id = ?1 AND target_kind = 'transcript' AND base_analysis_id = ?2", params![meeting_id, next_base]).map_err(|e| e.to_string())?;
    Ok(())
}
