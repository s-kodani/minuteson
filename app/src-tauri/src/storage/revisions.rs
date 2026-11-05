use super::*;
#[cfg(test)]
use rusqlite::TransactionBehavior;
use rusqlite::{types::ValueRef, Params, Transaction};

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedAnalysis {
    pub id: String,
    pub content_hash: String,
    pub meeting_lock_version: i64,
}

impl Storage {
    #[cfg(test)]
    pub fn commit_analysis_candidate(
        &self,
        meeting_id: &str,
        revision_id: &str,
        expected_meeting_lock_version: i64,
        _confirm_replacing_previous: bool,
    ) -> Result<CommittedAnalysis, String> {
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let content_hash = analysis_content_hash(&tx, revision_id)?;
        if tx
            .execute(
                "UPDATE analysis_revisions SET content_hash=?1, publication_state='committed'
             WHERE id=?2 AND meeting_id=?3 AND publication_state='candidate'",
                params![content_hash, revision_id, meeting_id],
            )
            .map_err(|e| e.to_string())?
            != 1
        {
            return Err("解析候補が見つかりません".into());
        }
        tx.execute(
            "DELETE FROM working_drafts WHERE meeting_id=?1",
            [meeting_id],
        )
        .map_err(|e| e.to_string())?;
        if tx.execute(
            "UPDATE meetings SET current_analysis_id=?1,lock_version=lock_version+1,updated_at=?2
             WHERE id=?3 AND lock_version=?4",
            params![revision_id, now_ms()?, meeting_id, expected_meeting_lock_version],
        ).map_err(|e| e.to_string())? != 1 {
            return Err("会議が変更されています".into());
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(CommittedAnalysis {
            id: revision_id.to_owned(),
            content_hash,
            meeting_lock_version: expected_meeting_lock_version + 1,
        })
    }

    #[allow(dead_code)]
    #[cfg(test)]
    pub fn verify_analysis_content_hash(&self, revision_id: &str) -> Result<bool, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| error.to_string())?;
        let expected: Option<String> = transaction
            .query_row(
                "SELECT content_hash FROM analysis_revisions WHERE id = ?1 AND publication_state IN ('committed', 'internal_snapshot')",
                params![revision_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let expected = expected.ok_or("確定済み解析版が見つかりません")?;
        let actual = analysis_content_hash(&transaction, revision_id)?;
        Ok(expected == actual)
    }
}

pub(super) fn analysis_content_hash(
    transaction: &Transaction<'_>,
    revision_id: &str,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(b"mon-analysis-content-v1");
    for (marker, sql) in [
        ("revision", "SELECT meeting_id, parent_revision_id, revision_no, parent_content_hash, origin, audio_manifest_hash, profile_snapshot_json, config_json, schema_version FROM analysis_revisions WHERE id = ?1"),
        ("speakers", "SELECT id, lineage_id, label, display_name, input_scope, identification_state FROM speakers WHERE analysis_revision_id = ?1 ORDER BY id"),
        ("segments", "SELECT id, track_id, speaker_id, start_us, end_us, overlap_group_id, confidence FROM speaker_segments WHERE analysis_revision_id = ?1 ORDER BY id"),
        ("utterances", "SELECT id, track_id, lineage_id, sequence, start_us, end_us, recognized_text, effective_text, attribution_state, confidence FROM utterances WHERE analysis_revision_id = ?1 ORDER BY sequence, id"),
        ("assignments", "SELECT utterance_id, speaker_id, role, assignment_source FROM utterance_speakers WHERE analysis_revision_id = ?1 ORDER BY utterance_id, speaker_id"),
    ] {
        hash_rows(transaction, &mut hasher, marker, sql, params![revision_id])?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_rows<P: Params>(
    transaction: &Transaction<'_>,
    hasher: &mut Sha256,
    marker: &str,
    sql: &str,
    parameters: P,
) -> Result<(), String> {
    hasher.update(marker.as_bytes());
    hasher.update([0xff]);
    let mut statement = transaction
        .prepare(sql)
        .map_err(|error| error.to_string())?;
    let columns = statement.column_count();
    let mut rows = statement
        .query(parameters)
        .map_err(|error| error.to_string())?;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        hasher.update([0xfe]);
        for index in 0..columns {
            match row.get_ref(index).map_err(|error| error.to_string())? {
                ValueRef::Null => hasher.update([0]),
                ValueRef::Integer(value) => {
                    hasher.update([1]);
                    hasher.update(value.to_be_bytes());
                }
                ValueRef::Real(value) => {
                    hasher.update([2]);
                    hasher.update(value.to_bits().to_be_bytes());
                }
                ValueRef::Text(bytes) => {
                    hasher.update([3]);
                    hasher.update((bytes.len() as u64).to_be_bytes());
                    hasher.update(bytes);
                }
                ValueRef::Blob(bytes) => {
                    hasher.update([4]);
                    hasher.update((bytes.len() as u64).to_be_bytes());
                    hasher.update(bytes);
                }
            }
        }
    }
    hasher.update([0xfd]);
    Ok(())
}
