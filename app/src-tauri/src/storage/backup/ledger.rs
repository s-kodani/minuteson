//! Immutable local batch reservations and portable provenance validation.
//! A prepared reservation is not publication and never starts restored work.
use super::restore_ids::RestoreIds;
use super::*;
use rusqlite::Transaction;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct IdentityMap {
    pub domain: String,
    pub scope: String,
    pub source: String,
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BatchState {
    Prepared,
    Published,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SavedBatch {
    pub archive: String,
    pub fingerprint: String,
    pub hash: String,
    pub count: i64,
    pub state: BatchState,
    pub created_at: i64,
    pub published_at: Option<i64>,
}

pub(super) fn fingerprint(source: &archive::VerifiedArchive) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&source.manifest).map_err(|e| e.to_string())?)
    ))
}

pub(super) fn saved_batch(
    db: &Connection,
    batch: &str,
    source: &archive::VerifiedArchive,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<SavedBatch>, String> {
    check_cancelled(cancelled)?;
    if !uuid(batch) || verify_migration_history(db).map_err(|e| e.to_string())? != MIGRATIONS.len()
    {
        return Err("復元バッチまたは保存先DB形式が不正です".into());
    }
    let row:Option<(String,String,String,i64,String,i64,Option<i64>)>=db.query_row("SELECT archive_id,archive_fingerprint,mapping_hash,mapping_count,state,created_at,published_at FROM restore_batches WHERE batch_id=?1",params![batch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e|e.to_string())?;
    let Some((archive, proof, hash, count, state, created_at, published_at)) = row else {
        let orphan: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM restore_identity_maps WHERE batch_id=?1)",
                params![batch],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if orphan {
            return Err("復元バッチの記録が欠損しています".into());
        }
        return Ok(None);
    };
    if archive != source.manifest.archive_id
        || proof != fingerprint(source)?
        || !portable::valid_hash(&hash)
        || count < 0
    {
        return Err("復元バッチの元アーカイブ・ID対応記録が一致しません".into());
    }
    let state = match (state.as_str(), published_at) {
        ("prepared", None) => BatchState::Prepared,
        ("published", Some(_)) => BatchState::Published,
        _ => return Err("復元バッチの公開状態が不正です".into()),
    };
    check_cancelled(cancelled)?;
    Ok(Some(SavedBatch {
        archive,
        fingerprint: proof,
        hash,
        count,
        state,
        created_at,
        published_at,
    }))
}

pub(super) struct BatchPlan<'a, 'b> {
    ids: &'a RestoreIds<'b>,
    fingerprint: String,
    hash: String,
    count: i64,
}

pub(super) fn valid_domain(domain: &str) -> bool {
    (portable::TABLES.contains(&domain)
        && !matches!(
            domain,
            "restore_provenance"
                | "vocabulary_scopes"
                | "track_inputs"
                | "analysis_input_snapshots"
                | "audio_import_options"
                | "diarization_manual_assignments"
                | "utterance_speakers"
                | "audio_import_commits"
                | "meeting_audio_commits"
        ))
        || matches!(
            domain,
            "operation" | "speaker_lineage" | "utterance_lineage" | "overlap_group"
        )
}

// Read-only origin verification for native restored-audio unavailability. A
// completed maintenance label alone cannot establish a published restore.
pub(in crate::storage) fn verify_restored_audio_origin(
    db: &Connection,
    batch: &str,
    archive: &str,
    fingerprint: &str,
    source: &str,
    target: &str,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<i64, String> {
    let (actual_archive, actual_fingerprint, expected_hash, expected_count, state, created, published):
        (String, String, String, i64, String, i64, Option<i64>) = db.query_row(
        "SELECT archive_id,archive_fingerprint,mapping_hash,mapping_count,state,created_at,published_at FROM restore_batches WHERE batch_id=?1",
        params![batch], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).map_err(|e| e.to_string())?;
    let published = published.ok_or("音声利用不可の復元は未公開です")?;
    let database_bytes: i64 = db
        .query_row(
            "SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if actual_archive != archive
        || actual_fingerprint != fingerprint
        || state != "published"
        || created < 0
        || published < created
        || expected_count < 0
        || expected_count >= database_bytes
        || !portable::valid_hash(&expected_hash)
    {
        return Err("音声利用不可の復元元・公開状態が一致しません".into());
    }
    let mut query = db.prepare("SELECT domain,source_scope,source_key,target_key FROM restore_identity_maps WHERE batch_id=?1 ORDER BY domain,source_scope,source_key LIMIT ?2").map_err(|e|e.to_string())?;
    let mut rows = query
        .query(params![batch, expected_count + 1])
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut count = 0_i64;
    let key = super::restored_audio::operation_key(archive, source);
    let mut found = (false, false);
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(check)?;
        count += 1;
        if count > expected_count {
            return Err("復元ID台帳が記録件数を超えています".into());
        }
        let map = IdentityMap {
            domain: row.get(0).map_err(|e| e.to_string())?,
            scope: row.get(1).map_err(|e| e.to_string())?,
            source: row.get(2).map_err(|e| e.to_string())?,
            target: row.get(3).map_err(|e| e.to_string())?,
        };
        if map.domain == "meetings"
            && map.scope.is_empty()
            && map.source == source
            && map.target == target
        {
            found.0 = true;
        }
        if map.domain == "operation"
            && map.scope == source
            && map.source == key
            && map.target == operation
        {
            found.1 = true;
        }
        hash.update(serde_json::to_vec(&map).map_err(|e| e.to_string())?);
        hash.update(b"\n");
    }
    if count != expected_count
        || format!("{:x}", hash.finalize()) != expected_hash
        || found != (true, true)
    {
        return Err("音声利用不可の復元ID台帳・件数・SHAが一致しません".into());
    }
    check_cancelled(check)?;
    Ok(published)
}

fn uuid(text: &str) -> bool {
    Uuid::parse_str(text).is_ok_and(|id| id.to_string() == text)
}

pub(super) fn validate_provenance(
    db: &Connection,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let present:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='restore_provenance' AND type='table')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !present {
        return check_cancelled(cancelled);
    }
    let mut query=db.prepare("SELECT meeting_id,domain,entity_key,source_archive,source_scope,source_key,restore_batch FROM restore_provenance ORDER BY id").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(cancelled)?;
        let meeting: Option<String> = row.get(0).map_err(|e| e.to_string())?;
        let domain: String = row.get(1).map_err(|e| e.to_string())?;
        let entity: String = row.get(2).map_err(|e| e.to_string())?;
        let archive: String = row.get(3).map_err(|e| e.to_string())?;
        let source_scope: String = row.get(4).map_err(|e| e.to_string())?;
        let source: String = row.get(5).map_err(|e| e.to_string())?;
        let batch: String = row.get(6).map_err(|e| e.to_string())?;
        if !valid_domain(&domain)
            || !uuid(&archive)
            || !uuid(&batch)
            || [&entity, &source]
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.contains('\0'))
            || (!source_scope.is_empty() && !uuid(&source_scope))
            || meeting.as_deref().is_some_and(|m| !uuid(m))
            || (meeting.is_none()
                && !matches!(
                    domain.as_str(),
                    "vocabulary_entries" | "voice_people" | "voice_samples" | "voice_policies"
                ))
            || (domain == "meetings" && meeting.as_deref() != Some(&entity))
        {
            return Err("復元由来の領域・識別子・会議所属が不正です".into());
        }
        // A history key may have no live row after deletion. If it is still a
        // relational entity, never let a different meeting's object masquerade
        // as this meeting's provenance. JSON-only/manual keys stay historical.
        if portable::TABLES.contains(&domain.as_str()) && domain != "meetings" {
            let sql = super::restore_ids::owners_sql(&domain)?;
            let owner:Option<Option<String>>=db.query_row(&format!("SELECT o.meeting_id FROM ({sql}) o JOIN \"{domain}\" d ON d.rowid=o.rowid WHERE d.id=?1"),params![entity],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
            if owner.is_some_and(|owner| owner != meeting) {
                return Err("復元由来が別会議の対象を参照しています".into());
            }
        }
    }
    check_cancelled(cancelled)
}

impl<'a, 'b> BatchPlan<'a, 'b> {
    pub(super) fn proof(&self) -> (String, String, i64) {
        (self.fingerprint.clone(), self.hash.clone(), self.count)
    }

    pub(super) fn new(
        ids: &'a RestoreIds<'b>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let fingerprint = fingerprint(ids.source())?;
        let mut hash = Sha256::new();
        let mut count = 0_i64;
        ids.visit_mappings(cancelled, |map| {
            hash.update(serde_json::to_vec(&map).map_err(|e| e.to_string())?);
            hash.update(b"\n");
            count = count.checked_add(1).ok_or("復元ID件数が範囲外です")?;
            Ok(())
        })?;
        Ok(Self {
            ids,
            fingerprint,
            hash: format!("{:x}", hash.finalize()),
            count,
        })
    }

    // Caller owns the transaction. Publication and the terminal state change
    // must later happen in the same transaction as additive meeting insertion.
    pub(super) fn reserve(
        &self,
        tx: &mut Transaction<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BatchState, String> {
        let savepoint = tx.savepoint().map_err(|e| e.to_string())?;
        let state = self.reserve_inner(&savepoint, cancelled)?;
        savepoint.commit().map_err(|e| e.to_string())?;
        Ok(state)
    }

    fn reserve_inner(
        &self,
        tx: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BatchState, String> {
        check_cancelled(cancelled)?;
        let old:Option<(String,String,String,i64,String)>=tx.query_row("SELECT archive_id,archive_fingerprint,mapping_hash,mapping_count,state FROM restore_batches WHERE batch_id=?1",params![self.ids.batch_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?;
        if let Some((archive, fingerprint, hash, count, state)) = old {
            if archive != self.ids.archive_id
                || fingerprint != self.fingerprint
                || hash != self.hash
                || count != self.count
            {
                return Err(
                    "同じ復元バッチに異なるアーカイブ・ID対応を使うことはできません".into(),
                );
            }
            let count: i64 = tx
                .query_row(
                    "SELECT count(*) FROM restore_identity_maps WHERE batch_id=?1",
                    params![self.ids.batch_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if count != self.count {
                return Err("復元バッチの永続ID対応が欠損しています".into());
            }
            self.ids.visit_mappings(cancelled, |map| {
                let target:Option<String>=tx.query_row("SELECT target_key FROM restore_identity_maps WHERE batch_id=?1 AND domain=?2 AND source_scope=?3 AND source_key=?4",params![self.ids.batch_id,map.domain,map.scope,map.source],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
                if target.as_deref()!=Some(&map.target) {return Err("復元バッチの永続ID対応が変更されています".into());}
                Ok(())
            })?;
            return match state.as_str() {
                "prepared" => Ok(BatchState::Prepared),
                "published" => Ok(BatchState::Published),
                _ => Err("復元バッチの状態が不正です".into()),
            };
        }
        tx.execute("INSERT INTO restore_batches(batch_id,archive_id,archive_fingerprint,mapping_hash,mapping_count,state,created_at) VALUES(?1,?2,?3,?4,?5,'prepared',?6)",params![self.ids.batch_id,self.ids.archive_id,self.fingerprint,self.hash,self.count,now_ms()?]).map_err(|e|e.to_string())?;
        self.ids.visit_mappings(cancelled, |map| {
            tx.execute(
                "INSERT INTO restore_identity_maps VALUES(?1,?2,?3,?4,?5)",
                params![
                    self.ids.batch_id,
                    map.domain,
                    map.scope,
                    map.source,
                    map.target
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })?;
        check_cancelled(cancelled)?;
        Ok(BatchState::Prepared)
    }
}

#[cfg(test)]
pub(in crate::storage) fn remove_restore_schema_for_test(db: &Connection) {
    super::publication::journal::remove_schema_for_test(db);
    // Downgrade only empty, test-owned current images into older fixtures.
    // Never discard real provenance or a reservation to simulate migration.
    for table in [
        "restore_provenance",
        "restore_identity_maps",
        "restore_batches",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    db.execute_batch("DROP TABLE restore_provenance; DROP TABLE restore_identity_maps; DROP TABLE restore_batches; DELETE FROM schema_migrations WHERE version=21;").unwrap();
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
