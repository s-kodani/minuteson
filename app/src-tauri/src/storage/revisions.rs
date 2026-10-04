use super::*;
use rusqlite::{types::ValueRef, Params, Transaction, TransactionBehavior};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedAnalysis {
    pub id: String,
    pub content_hash: String,
    pub previous_id: Option<String>,
    pub meeting_lock_version: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedMinutes {
    pub id: String,
    pub content_hash: String,
    pub previous_id: Option<String>,
    pub meeting_lock_version: i64,
}

impl Storage {
    #[allow(dead_code)]
    pub fn commit_analysis_candidate(
        &self,
        meeting_id: &str,
        revision_id: &str,
        expected_meeting_lock_version: i64,
        confirm_replacing_previous: bool,
    ) -> Result<CommittedAnalysis, String> {
        self.commit_analysis_inner(
            meeting_id,
            revision_id,
            expected_meeting_lock_version,
            confirm_replacing_previous,
            None,
        )
    }

    pub(super) fn commit_analysis_inner(
        &self,
        meeting_id: &str,
        revision_id: &str,
        expected_meeting_lock_version: i64,
        confirm_replacing_previous: bool,
        checked: Option<(&str, Option<&super::analysis_ui::DraftStamp>)>,
    ) -> Result<CommittedAnalysis, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let meeting: Option<(i64, Option<String>, Option<String>)> = transaction
            .query_row(
                "SELECT lock_version, current_analysis_id, previous_analysis_id FROM meetings WHERE id = ?1",
                params![meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let (lock_version, current_id, previous_id) = meeting.ok_or("会議が見つかりません")?;
        if lock_version != expected_meeting_lock_version {
            return Err("別の画面で会議が更新されました。再読込してください".into());
        }
        if previous_id.is_some() && !confirm_replacing_previous {
            return Err("直前版が入れ替わります。確認が必要です".into());
        }
        let state: Option<String> = transaction
            .query_row(
                "SELECT publication_state FROM analysis_revisions WHERE id = ?1 AND meeting_id = ?2",
                params![revision_id, meeting_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if state.as_deref() != Some("candidate") {
            return Err("採用可能な解析候補が見つかりません".into());
        }
        let content_hash = analysis_content_hash(&transaction, revision_id)?;
        if let Some((expected_hash, draft)) = checked {
            let (stored_hash, parent_id): (String, Option<String>) = transaction
                .query_row(
                    "SELECT content_hash, parent_revision_id FROM analysis_revisions WHERE id = ?1",
                    params![revision_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| e.to_string())?;
            if content_hash != expected_hash || stored_hash != expected_hash {
                return Err("確認した解析候補の内容が変更されています".into());
            }
            if parent_id != current_id {
                return Err("候補作成後に現在版が切り替わっています。再解析してください".into());
            }
            if let Some(current) = &current_id {
                let expected: String = transaction
                    .query_row(
                        "SELECT content_hash FROM analysis_revisions WHERE id = ?1",
                        params![current],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if analysis_content_hash(&transaction, current)? != expected {
                    return Err("現在の解析版の内容が変更されています".into());
                }
            }
            super::analysis_ui::switch_draft(&transaction, meeting_id, revision_id, draft)?;
        }
        transaction
            .execute(
                "UPDATE analysis_revisions SET content_hash = ?1, publication_state = 'committed' WHERE id = ?2 AND meeting_id = ?3 AND publication_state = 'candidate'",
                params![content_hash, revision_id, meeting_id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE meetings SET current_analysis_id = ?1, previous_analysis_id = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lock_version = ?5",
                params![revision_id, current_id, now_ms()?, meeting_id, lock_version],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(CommittedAnalysis {
            id: revision_id.to_owned(),
            content_hash,
            previous_id: current_id,
            meeting_lock_version: lock_version + 1,
        })
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn commit_minutes_candidate(
        &self,
        meeting_id: &str,
        revision_id: &str,
        expected_meeting_lock_version: i64,
        confirm_replacing_previous: bool,
    ) -> Result<CommittedMinutes, String> {
        self.commit_minutes_inner(
            meeting_id,
            revision_id,
            expected_meeting_lock_version,
            confirm_replacing_previous,
            None,
        )
    }

    pub(super) fn commit_minutes_inner(
        &self,
        meeting_id: &str,
        revision_id: &str,
        expected_meeting_lock_version: i64,
        confirm_replacing_previous: bool,
        checked: Option<(&str, Option<&super::analysis_ui::DraftStamp>)>,
    ) -> Result<CommittedMinutes, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let meeting: Option<(i64, Option<String>, Option<String>)> = transaction.query_row(
            "SELECT lock_version, current_minutes_id, previous_minutes_id FROM meetings WHERE id = ?1",
            params![meeting_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (lock_version, current_id, previous_id) = meeting.ok_or("会議が見つかりません")?;
        if lock_version != expected_meeting_lock_version {
            return Err("別の画面で会議が更新されました。再読込してください".into());
        }
        if previous_id.is_some() && !confirm_replacing_previous {
            return Err("直前版が入れ替わります。確認が必要です".into());
        }
        let candidate: Option<(String, String, String)> = transaction.query_row(
            "SELECT publication_state, input_analysis_id, input_content_hash FROM minutes_revisions WHERE id = ?1 AND meeting_id = ?2",
            params![revision_id, meeting_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (state, input_id, input_hash) = candidate.ok_or("議事録候補が見つかりません")?;
        if state != "candidate" {
            return Err("採用可能な議事録候補が見つかりません".into());
        }
        let committed_input_hash: Option<String> = transaction.query_row(
            "SELECT content_hash FROM analysis_revisions WHERE id = ?1 AND meeting_id = ?2 AND publication_state IN ('committed', 'internal_snapshot')",
            params![input_id, meeting_id],
            |row| row.get(0),
        ).optional().map_err(|error| error.to_string())?;
        if committed_input_hash.as_deref() != Some(input_hash.as_str())
            || analysis_content_hash(&transaction, &input_id)? != input_hash
        {
            return Err("議事録の入力解析版が変更されています".into());
        }
        let content_hash = minutes_content_hash(&transaction, revision_id)?;
        if let Some((expected_hash, draft)) = checked {
            let view = super::minutes::read_content(&transaction, meeting_id, revision_id)?
                .ok_or("確認できる議事録候補がありません")?;
            let parent: Option<String> = transaction
                .query_row(
                    "SELECT parent_revision_id FROM minutes_revisions WHERE id=?1",
                    params![revision_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if view.content_hash != expected_hash || content_hash != expected_hash {
                return Err("確認した議事録候補の内容が変更されています".into());
            }
            if parent != current_id {
                return Err(
                    "候補作成後に現在の議事録版が切り替わっています。再生成してください".into(),
                );
            }
            if let Some(current) = &current_id {
                super::minutes::read_content(&transaction, meeting_id, current)?
                    .ok_or("現在の議事録版がありません")?;
            }
            super::minutes_drafts::switch(&transaction, meeting_id, revision_id, draft)?;
        }
        transaction.execute(
            "UPDATE minutes_revisions SET content_hash = ?1, publication_state = 'committed' WHERE id = ?2 AND meeting_id = ?3 AND publication_state = 'candidate'",
            params![content_hash, revision_id, meeting_id],
        ).map_err(|error| error.to_string())?;
        transaction.execute(
            "UPDATE meetings SET current_minutes_id = ?1, previous_minutes_id = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lock_version = ?5",
            params![revision_id, current_id, now_ms()?, meeting_id, lock_version],
        ).map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(CommittedMinutes {
            id: revision_id.to_owned(),
            content_hash,
            previous_id: current_id,
            meeting_lock_version: lock_version + 1,
        })
    }

    #[allow(dead_code)]
    pub fn verify_minutes_content_hash(&self, revision_id: &str) -> Result<bool, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| error.to_string())?;
        let expected: Option<String> = transaction.query_row(
            "SELECT content_hash FROM minutes_revisions WHERE id = ?1 AND publication_state = 'committed'",
            params![revision_id],
            |row| row.get(0),
        ).optional().map_err(|error| error.to_string())?;
        let expected = expected.ok_or("確定済み議事録版が見つかりません")?;
        Ok(expected == minutes_content_hash(&transaction, revision_id)?)
    }

    #[allow(dead_code)]
    pub fn restore_previous_analysis(
        &self,
        meeting_id: &str,
        expected_meeting_lock_version: i64,
    ) -> Result<CommittedAnalysis, String> {
        self.restore_analysis_inner(meeting_id, expected_meeting_lock_version, None)
    }

    pub(super) fn restore_analysis_inner(
        &self,
        meeting_id: &str,
        expected_meeting_lock_version: i64,
        checked: Option<(&str, &str, Option<&super::analysis_ui::DraftStamp>)>,
    ) -> Result<CommittedAnalysis, String> {
        if expected_meeting_lock_version < 0 {
            return Err("会議の版番号が不正です".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let heads: Option<(i64, Option<String>, Option<String>)> = transaction.query_row(
            "SELECT lock_version, current_analysis_id, previous_analysis_id FROM meetings WHERE id = ?1",
            params![meeting_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (lock_version, current_id, previous_id) = heads.ok_or("会議が見つかりません")?;
        if lock_version != expected_meeting_lock_version {
            return Err("別の画面で会議が更新されました。再読込してください".into());
        }
        let current_id = current_id.ok_or("現在の解析版がありません")?;
        let previous_id = previous_id.ok_or("復元できる直前の解析版がありません")?;
        let current_hash: String = transaction.query_row(
            "SELECT content_hash FROM analysis_revisions WHERE id = ?1 AND meeting_id = ?2 AND publication_state = 'committed'",
            params![current_id, meeting_id],
            |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if current_hash != analysis_content_hash(&transaction, &current_id)? {
            return Err("現在の解析版の内容が変化しています".into());
        }
        let expected_hash: String = transaction.query_row(
            "SELECT content_hash FROM analysis_revisions WHERE id = ?1 AND meeting_id = ?2 AND publication_state = 'committed'",
            params![previous_id, meeting_id],
            |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if expected_hash != analysis_content_hash(&transaction, &previous_id)? {
            return Err("直前の解析版の内容が変化しています".into());
        }
        if let Some((preview_id, preview_hash, draft)) = checked {
            if preview_id != previous_id || preview_hash != expected_hash {
                return Err("確認した直前版と復元対象が一致しません。再確認してください".into());
            }
            super::analysis_ui::switch_draft(&transaction, meeting_id, &previous_id, draft)?;
        }
        transaction.execute(
            "UPDATE meetings SET current_analysis_id = ?1, previous_analysis_id = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lock_version = ?5",
            params![previous_id, current_id, now_ms()?, meeting_id, lock_version],
        ).map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(CommittedAnalysis {
            id: previous_id,
            content_hash: expected_hash,
            previous_id: Some(current_id),
            meeting_lock_version: lock_version + 1,
        })
    }

    #[allow(dead_code)]
    pub fn restore_previous_minutes(
        &self,
        meeting_id: &str,
        expected_meeting_lock_version: i64,
    ) -> Result<CommittedMinutes, String> {
        self.restore_minutes_inner(meeting_id, expected_meeting_lock_version, None)
    }

    pub(super) fn restore_minutes_inner(
        &self,
        meeting_id: &str,
        expected_meeting_lock_version: i64,
        checked: Option<(&str, &str, Option<&super::analysis_ui::DraftStamp>)>,
    ) -> Result<CommittedMinutes, String> {
        if expected_meeting_lock_version < 0 {
            return Err("会議の版番号が不正です".into());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let heads: Option<(i64, Option<String>, Option<String>)> = transaction.query_row(
            "SELECT lock_version, current_minutes_id, previous_minutes_id FROM meetings WHERE id = ?1",
            params![meeting_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|error| error.to_string())?;
        let (lock_version, current_id, previous_id) = heads.ok_or("会議が見つかりません")?;
        if lock_version != expected_meeting_lock_version {
            return Err("別の画面で会議が更新されました。再読込してください".into());
        }
        let current_id = current_id.ok_or("現在の議事録版がありません")?;
        let previous_id = previous_id.ok_or("復元できる直前の議事録版がありません")?;
        let current_hash: String = transaction.query_row(
            "SELECT content_hash FROM minutes_revisions WHERE id = ?1 AND meeting_id = ?2 AND publication_state = 'committed'",
            params![current_id, meeting_id],
            |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if current_hash != minutes_content_hash(&transaction, &current_id)? {
            return Err("現在の議事録版の内容が変化しています".into());
        }
        let expected_hash: String = transaction.query_row(
            "SELECT content_hash FROM minutes_revisions WHERE id = ?1 AND meeting_id = ?2 AND publication_state = 'committed'",
            params![previous_id, meeting_id],
            |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if expected_hash != minutes_content_hash(&transaction, &previous_id)? {
            return Err("直前の議事録版の内容が変化しています".into());
        }
        if let Some((preview_id, preview_hash, draft)) = checked {
            if preview_id != previous_id || preview_hash != expected_hash {
                return Err("確認した議事録直前版と復元対象が一致しません".into());
            }
            super::minutes::read_content(&transaction, meeting_id, &current_id)?
                .ok_or("現在の議事録版がありません")?;
            super::minutes::read_content(&transaction, meeting_id, &previous_id)?
                .ok_or("直前の議事録版がありません")?;
            super::minutes_drafts::switch(&transaction, meeting_id, &previous_id, draft)?;
        }
        transaction.execute(
            "UPDATE meetings SET current_minutes_id = ?1, previous_minutes_id = ?2, lock_version = lock_version + 1, updated_at = ?3 WHERE id = ?4 AND lock_version = ?5",
            params![previous_id, current_id, now_ms()?, meeting_id, lock_version],
        ).map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(CommittedMinutes {
            id: previous_id,
            content_hash: expected_hash,
            previous_id: Some(current_id),
            meeting_lock_version: lock_version + 1,
        })
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

pub(super) fn minutes_content_hash(
    transaction: &Transaction<'_>,
    revision_id: &str,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(b"mon-minutes-content-v1");
    for (marker, sql) in [
        ("revision", "SELECT meeting_id, input_analysis_id, parent_revision_id, revision_no, parent_content_hash, generated_document_json, effective_document_json, document_schema_version, input_content_hash, model_snapshot_json, template_version FROM minutes_revisions WHERE id = ?1"),
        ("items", "SELECT id, analysis_revision_id, kind, order_no, body, assignee_name, assignee_speaker_id, due_date, due_text, source_type FROM minutes_items WHERE minutes_revision_id = ?1 ORDER BY order_no, id"),
        ("evidence", "SELECT id, minutes_item_id, analysis_revision_id, utterance_id, quote_text, start_us, end_us, source_type, verification_state FROM evidence_links WHERE minutes_revision_id = ?1 ORDER BY id"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_commit_hashes_content_and_switches_heads_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("版管理").unwrap();
        {
            let db = storage.connection.lock().unwrap();
            for number in 1..=3 {
                db.execute(
                    "INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES (?1, ?2, ?3, 'engine', 'candidate', ?4, '{}', '{}', ?5, 1, 0)",
                    params![format!("analysis-{number}"), meeting.id, number, "a".repeat(64), "0".repeat(64)],
                ).unwrap();
            }
            let before = {
                let transaction = db.unchecked_transaction().unwrap();
                analysis_content_hash(&transaction, "analysis-2").unwrap()
            };
            db.execute("INSERT INTO speakers (id, analysis_revision_id, lineage_id, label, input_scope, identification_state, created_at) VALUES ('speaker-2', 'analysis-2', 'lineage-2', '話者2', 'all', 'unknown', 0)", []).unwrap();
            let after = {
                let transaction = db.unchecked_transaction().unwrap();
                analysis_content_hash(&transaction, "analysis-2").unwrap()
            };
            assert_ne!(before, after);
        }
        let first = storage
            .commit_analysis_candidate(&meeting.id, "analysis-1", 0, false)
            .unwrap();
        assert_eq!(first.previous_id, None);
        assert_eq!(first.meeting_lock_version, 1);
        assert!(storage.verify_analysis_content_hash("analysis-1").unwrap());
        assert!(storage
            .commit_analysis_candidate(&meeting.id, "analysis-2", 0, false)
            .is_err());
        let second = storage
            .commit_analysis_candidate(&meeting.id, "analysis-2", 1, false)
            .unwrap();
        assert_eq!(second.previous_id.as_deref(), Some("analysis-1"));
        assert!(storage.verify_analysis_content_hash("analysis-2").unwrap());
        assert!(storage
            .commit_analysis_candidate(&meeting.id, "analysis-3", 2, false)
            .is_err());
        assert_eq!(
            storage
                .connection
                .lock()
                .unwrap()
                .query_row(
                    "SELECT publication_state FROM analysis_revisions WHERE id = 'analysis-3'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "candidate"
        );
        let third = storage
            .commit_analysis_candidate(&meeting.id, "analysis-3", 2, true)
            .unwrap();
        assert_eq!(third.previous_id.as_deref(), Some("analysis-2"));
        let db = storage.connection.lock().unwrap();
        let heads: (String, String, i64) = db.query_row("SELECT current_analysis_id, previous_analysis_id, lock_version FROM meetings WHERE id = ?1", params![meeting.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(heads, ("analysis-3".into(), "analysis-2".into(), 3));
        assert_eq!(
            db.query_row(
                "SELECT publication_state FROM analysis_revisions WHERE id = 'analysis-1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "committed"
        );
        drop(db);
        let draft = storage
            .create_transcript_draft(&meeting.id, "analysis-3")
            .unwrap();
        assert!(storage.restore_previous_analysis(&meeting.id, 2).is_err());
        let restored = storage.restore_previous_analysis(&meeting.id, 3).unwrap();
        assert_eq!(restored.id, "analysis-2");
        assert_eq!(restored.previous_id.as_deref(), Some("analysis-3"));
        assert_eq!(restored.meeting_lock_version, 4);
        assert_eq!(
            storage.load_transcript_draft(&meeting.id).unwrap(),
            Some(draft)
        );
        let switched_back = storage.restore_previous_analysis(&meeting.id, 4).unwrap();
        assert_eq!(switched_back.id, "analysis-3");
    }

    #[test]
    fn minutes_candidate_requires_unchanged_analysis_and_confirmation_for_history() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Storage::open(temp.path().to_owned()).unwrap();
        let meeting = storage.create_meeting("議事録").unwrap();
        {
            let db = storage.connection.lock().unwrap();
            db.execute("INSERT INTO analysis_revisions (id, meeting_id, revision_no, origin, publication_state, audio_manifest_hash, profile_snapshot_json, config_json, content_hash, schema_version, created_at) VALUES ('analysis', ?1, 1, 'engine', 'candidate', ?2, '{}', '{}', ?3, 1, 0)", params![meeting.id, "a".repeat(64), "0".repeat(64)]).unwrap();
        }
        let analysis = storage
            .commit_analysis_candidate(&meeting.id, "analysis", 0, false)
            .unwrap();
        {
            let db = storage.connection.lock().unwrap();
            for number in 1..=3 {
                db.execute("INSERT INTO minutes_revisions (id, meeting_id, input_analysis_id, revision_no, publication_state, generated_document_json, effective_document_json, document_schema_version, input_content_hash, content_hash, model_snapshot_json, template_version, created_at) VALUES (?1, ?2, 'analysis', ?3, 'candidate', '{}', '{}', 1, ?4, ?5, '{}', 'v1', 0)", params![format!("minutes-{number}"), meeting.id, number, analysis.content_hash, "0".repeat(64)]).unwrap();
            }
        }
        let first = storage
            .commit_minutes_candidate(&meeting.id, "minutes-1", 1, false)
            .unwrap();
        assert_eq!(first.previous_id, None);
        assert!(storage.verify_minutes_content_hash("minutes-1").unwrap());
        assert!(storage
            .commit_minutes_candidate(&meeting.id, "minutes-2", 1, false)
            .is_err());
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE minutes_revisions SET input_content_hash = ?1 WHERE id = 'minutes-2'",
                params!["f".repeat(64)],
            )
            .unwrap();
        }
        assert!(storage
            .commit_minutes_candidate(&meeting.id, "minutes-2", 2, false)
            .is_err());
        {
            let db = storage.connection.lock().unwrap();
            db.execute(
                "UPDATE minutes_revisions SET input_content_hash = ?1 WHERE id = 'minutes-2'",
                params![analysis.content_hash],
            )
            .unwrap();
            db.execute("INSERT INTO minutes_items (id, minutes_revision_id, analysis_revision_id, kind, order_no, body, source_type) VALUES ('item-2', 'minutes-2', 'analysis', 'decision', 0, '決定', 'generated')", []).unwrap();
        }
        let second = storage
            .commit_minutes_candidate(&meeting.id, "minutes-2", 2, false)
            .unwrap();
        assert_eq!(second.previous_id.as_deref(), Some("minutes-1"));
        assert_ne!(first.content_hash, second.content_hash);
        assert!(storage.verify_minutes_content_hash("minutes-2").unwrap());
        assert!(storage
            .commit_minutes_candidate(&meeting.id, "minutes-3", 3, false)
            .is_err());
        let third = storage
            .commit_minutes_candidate(&meeting.id, "minutes-3", 3, true)
            .unwrap();
        assert_eq!(third.previous_id.as_deref(), Some("minutes-2"));
        let db = storage.connection.lock().unwrap();
        let heads: (String, String, i64) = db.query_row("SELECT current_minutes_id, previous_minutes_id, lock_version FROM meetings WHERE id = ?1", params![meeting.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(heads, ("minutes-3".into(), "minutes-2".into(), 4));
        drop(db);
        assert!(storage.restore_previous_minutes(&meeting.id, 3).is_err());
        let restored = storage.restore_previous_minutes(&meeting.id, 4).unwrap();
        assert_eq!(restored.id, "minutes-2");
        assert_eq!(restored.previous_id.as_deref(), Some("minutes-3"));
        assert_eq!(restored.meeting_lock_version, 5);
        assert_eq!(
            storage.restore_previous_minutes(&meeting.id, 5).unwrap().id,
            "minutes-3"
        );
    }
}
