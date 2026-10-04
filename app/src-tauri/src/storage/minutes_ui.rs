use super::*;
use rusqlite::TransactionBehavior;
use serde::Deserialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesVersionSummary {
    pub id: String,
    pub revision_no: i64,
    pub content_hash: String,
    pub publication_state: String,
    pub input_analysis_id: String,
    pub input_updated: bool,
    pub item_count: usize,
    pub created_at: i64,
    pub profile: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinutesVersionState {
    pub meeting_lock_version: i64,
    pub current_id: Option<String>,
    pub previous_id: Option<String>,
    pub draft: Option<DraftStamp>,
    pub versions: Vec<MinutesVersionSummary>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct MinutesAdoptRequest {
    pub meeting_id: String,
    pub revision_id: String,
    pub expected_content_hash: String,
    pub expected_meeting_lock_version: i64,
    pub expected_draft: Option<DraftStamp>,
    pub confirm_replacing_previous: bool,
}

impl Storage {
    pub(crate) fn minutes_versions(&self, meeting: &str) -> Result<MinutesVersionState, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let (meeting_lock_version,current_id,previous_id):(i64,Option<String>,Option<String>)=tx.query_row("SELECT lock_version,current_minutes_id,previous_minutes_id FROM meetings WHERE id=?1",params![meeting],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?.ok_or("会議がありません")?;
        let draft = super::minutes_drafts::load(&tx, meeting, None)?.map(|d| DraftStamp {
            id: d.id,
            lock_version: d.lock_version,
        });
        let mut query=tx.prepare("SELECT id,revision_no,publication_state,created_at,model_snapshot_json FROM minutes_revisions WHERE meeting_id=?1 AND (id IN (?2,?3) OR id IN (SELECT id FROM minutes_revisions WHERE meeting_id=?1 AND publication_state='candidate' ORDER BY revision_no DESC LIMIT 20)) ORDER BY revision_no DESC").map_err(|e|e.to_string())?;
        let rows = query
            .query_map(params![meeting, current_id, previous_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut versions = vec![];
        for row in rows {
            let (id, revision_no, publication_state, created_at, profile) =
                row.map_err(|e| e.to_string())?;
            let view = super::minutes::read_content(&tx, meeting, &id)?
                .ok_or("表示できる議事録版がありません")?;
            let input = super::minutes_input::read_snapshot(
                &tx,
                meeting,
                &view.input_analysis_id,
                &view.input_content_hash,
            )?;
            versions.push(MinutesVersionSummary {
                id,
                revision_no,
                publication_state,
                created_at,
                profile: serde_json::from_str(&profile).map_err(|e| e.to_string())?,
                content_hash: view.content_hash,
                input_analysis_id: view.input_analysis_id,
                input_updated: super::minutes_input::input_updated(&tx, meeting, &input)?,
                item_count: view.content.items.len(),
            });
        }
        Ok(MinutesVersionState {
            meeting_lock_version,
            current_id,
            previous_id,
            draft,
            versions,
        })
    }

    pub(crate) fn adopt_minutes(
        &self,
        request: MinutesAdoptRequest,
    ) -> Result<super::revisions::CommittedMinutes, String> {
        self.commit_minutes_inner(
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

    pub(crate) fn restore_minutes_checked(
        &self,
        meeting: &str,
        expected_lock: i64,
        previous: &str,
        expected_hash: &str,
        draft: Option<DraftStamp>,
    ) -> Result<super::revisions::CommittedMinutes, String> {
        self.restore_minutes_inner(
            meeting,
            expected_lock,
            Some((previous, expected_hash, draft.as_ref())),
        )
    }
}

#[cfg(test)]
#[path = "minutes_ui_tests.rs"]
pub(super) mod tests;
