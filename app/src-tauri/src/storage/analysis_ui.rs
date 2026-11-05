use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DraftStamp {
    pub id: String,
    pub lock_version: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesSource {
    pub meeting_lock_version: i64,
    pub current_id: Option<String>,
    pub content_hash: Option<String>,
    pub draft: Option<DraftStamp>,
}

impl Storage {
    pub(crate) fn minutes_source(&self, meeting_id: &str) -> Result<MinutesSource, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        let (meeting_lock_version, current_id, content_hash) = tx
            .query_row(
                "SELECT m.lock_version, a.id, a.content_hash FROM meetings m
                 LEFT JOIN analysis_revisions a ON a.id=m.current_analysis_id
                   AND a.meeting_id=m.id AND a.publication_state='committed'
                 WHERE m.id=?1",
                [meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("会議が見つかりません")?;
        let draft = super::drafts::load_draft(&tx, meeting_id)?.map(|draft| DraftStamp {
            id: draft.id,
            lock_version: draft.lock_version,
        });
        Ok(MinutesSource {
            meeting_lock_version,
            current_id,
            content_hash,
            draft,
        })
    }
}
