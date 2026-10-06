//! Reload a persisted mapping by replaying source definitions, not minting IDs.
use super::super::ledger::{BatchState, IdentityMap};
use super::*;

fn uuid(text: &str) -> bool {
    Uuid::parse_str(text).is_ok_and(|id| id.to_string() == text)
}

fn valid_map(map: &IdentityMap) -> bool {
    if !(super::super::ledger::valid_domain(&map.domain) || map.domain == "restore_provenance")
        || map.source.is_empty()
        || map.source.len() > 256
        || map.source.contains('\0')
        || map.target.is_empty()
        || map.target.len() > 256
        || map.target.contains('\0')
        || (map.domain == "meetings" && !map.scope.is_empty())
        || (!map.scope.is_empty() && !uuid(&map.scope))
        || (map.scope.is_empty()
            && !matches!(
                map.domain.as_str(),
                "meetings" | "vocabulary_entries" | "restore_provenance"
            ))
    {
        return false;
    }
    let prefixed = |prefix: &str| map.target.strip_prefix(prefix).is_some_and(uuid);
    match map.domain.as_str() {
        "speakers" if map.source.starts_with("manual:") => prefixed("manual:"),
        "utterances" if map.source.starts_with("split:") => prefixed("split:"),
        "speaker_lineage" if map.source.starts_with("manual:") => {
            uuid(&map.target) || prefixed("manual:")
        }
        "utterance_lineage" if map.source.starts_with("split:") => {
            uuid(&map.target) || prefixed("split:")
        }
        "operation" if map.target.starts_with("flow:") => {
            let parts: Vec<_> = map.target.split(':').collect();
            parts.len() == 3
                && uuid(parts[1])
                && matches!(parts[2], "transcription" | "diarization")
                && map.source.starts_with("flow:")
        }
        _ => uuid(&map.target),
    }
}

impl<'a> RestoreIds<'a> {
    pub(crate) fn prepare_for_batch(
        archive: &'a VerifiedArchive,
        parent: &Path,
        batch: &str,
        destination: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BatchPreparation<'a>, String> {
        let ids = Self::prepare_inner(archive, parent, batch, Some(destination), cancelled)?;
        ids.verify_saved_destination(destination, cancelled)?;
        // Verify again before returning: callbacks must not turn an inspected
        // prepared batch into a different/published record during the replay.
        if super::super::ledger::saved_batch(destination, batch, archive, cancelled)? != ids.saved {
            return Err("確認中に復元バッチが変更されています".into());
        }
        if ids
            .saved
            .as_ref()
            .is_some_and(|s| s.state == BatchState::Published)
        {
            Ok(BatchPreparation::AlreadyPublished {
                batch_id: ids.batch_id.clone(),
                archive_id: ids.archive_id.clone(),
            })
        } else {
            Ok(BatchPreparation::Pending(ids))
        }
    }

    pub(super) fn reused(&self) -> bool {
        self.saved.is_some()
    }

    pub(super) fn load_saved(
        &self,
        db: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let Some(saved) = &self.saved else {
            return Ok(());
        };
        let mut query=db.prepare("SELECT domain,source_scope,source_key,target_key FROM restore_identity_maps WHERE batch_id=?1 ORDER BY domain,source_scope,source_key").map_err(|e|e.to_string())?;
        let mut rows = query
            .query(params![self.batch_id])
            .map_err(|e| e.to_string())?;
        let mut count = 0_i64;
        let mut hash = Sha256::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let map = IdentityMap {
                domain: row.get(0).map_err(|e| e.to_string())?,
                scope: row.get(1).map_err(|e| e.to_string())?,
                source: row.get(2).map_err(|e| e.to_string())?,
                target: row.get(3).map_err(|e| e.to_string())?,
            };
            if !valid_map(&map) {
                return Err("保存済み復元IDの領域・会議範囲・形式が不正です".into());
            }
            count = count
                .checked_add(1)
                .ok_or("保存済み復元ID件数が範囲外です")?;
            if count > saved.count {
                return Err("保存済み復元IDが記録件数を超えています".into());
            }
            hash.update(serde_json::to_vec(&map).map_err(|e| e.to_string())?);
            hash.update(b"\n");
            self.database
                .execute(
                    "INSERT INTO cached_mappings VALUES(?1,?2,?3,?4)",
                    params![map.domain, map.scope, map.source, map.target],
                )
                .map_err(|e| e.to_string())?;
        }
        if count != saved.count || format!("{:x}", hash.finalize()) != saved.hash {
            return Err("保存済み復元ID対応の件数・SHAが一致しません".into());
        }
        check_cancelled(cancelled)
    }

    pub(super) fn check_destination_capacity(&self, db: &Connection) -> Result<(), String> {
        // Disk-backed reservations are bounded by the destination DB's size;
        // this is read-only and never copies models, audio, or host paths.
        let bytes: i64 = db
            .query_row(
                "SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let estimate = u64::try_from(bytes)
            .ok()
            .and_then(|b| b.checked_mul(4))
            .ok_or("保存先ID領域の容量が範囲外です")?;
        check_capacity(self.stage.path(), estimate)?;
        Ok(())
    }

    pub(super) fn reserve_destination(
        &self,
        db: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        for table in portable::tables(db)? {
            for column in rules(db, &table)?.keys() {
                let mut query = db
                    .prepare(&format!(
                        "SELECT \"{column}\" FROM \"{}\" WHERE \"{column}\" IS NOT NULL",
                        table.name
                    ))
                    .map_err(|e| e.to_string())?;
                let mut rows = query.query([]).map_err(|e| e.to_string())?;
                while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                    check_cancelled(cancelled)?;
                    self.reserve(&row.get::<_, String>(0).map_err(|e| e.to_string())?)?;
                }
            }
        }
        self.visit_documents(db, cancelled, 0)?;
        super::super::restore_fixed::visit_definitions(db, cancelled, &mut |definition| {
            self.reserve(&definition.old)
        })?;
        for sql in [
            "SELECT target_key FROM restore_identity_maps WHERE batch_id<>?1",
            "SELECT batch_id FROM restore_batches",
            "SELECT source_key FROM restore_provenance",
            "SELECT source_scope FROM restore_provenance WHERE source_scope<>''",
            "SELECT id FROM maintenance_operations",
            "SELECT id FROM model_packages",
            "SELECT id FROM model_files",
            "SELECT id FROM model_profiles",
            "SELECT id FROM model_catalog_history",
        ] {
            let mut query = db.prepare(sql).map_err(|e| e.to_string())?;
            let mut rows = if sql.contains("?1") {
                query.query(params![self.batch_id])
            } else {
                query.query([])
            }
            .map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check_cancelled(cancelled)?;
                self.reserve(&row.get::<_, String>(0).map_err(|e| e.to_string())?)?;
            }
        }
        for entry in crate::storage::model_profiles::portable::history::read(db, cancelled)? {
            // Imported origin identities never become a new entity's PK.
            for id in entry.origin_ids() {
                check_cancelled(cancelled)?;
                self.reserve(&id)?;
            }
        }
        check_cancelled(cancelled)
    }

    pub(in crate::storage::backup) fn check_publication_destination(
        &self,
        db: &Connection,
        reconstruction: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        // Probe the private disk registry without altering its immutable maps
        // or retaining a conflict after the destination later removes it.
        struct ReadOnly<'a>(&'a Connection);
        impl Drop for ReadOnly<'_> {
            fn drop(&mut self) {
                let _ = self.0.execute_batch("PRAGMA query_only=ON;");
            }
        }
        self.database
            .execute_batch("PRAGMA query_only=OFF;")
            .map_err(|e| e.to_string())?;
        let _read_only = ReadOnly(&self.database);
        let _probe = self
            .database
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        self.reserve_destination(db, cancelled)?;
        let collision: bool = self
            .database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM mappings m JOIN reserved_ids r ON r.id=m.new_id)",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if collision {
            return Err("復元IDが準備後の既存データと競合しています".into());
        }
        let mut query = reconstruction
            .prepare("SELECT id FROM restore_provenance")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            if self.target_reserved(&row.get::<_, String>(0).map_err(|e| e.to_string())?)? {
                return Err("復元由来IDが既存データと競合しています".into());
            }
        }
        check_cancelled(cancelled)
    }

    fn target_reserved(&self, target: &str) -> Result<bool, String> {
        self.database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM reserved_ids WHERE id=?1)",
                params![target],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }

    pub(super) fn cached_target(
        &self,
        domain: &str,
        meeting: &str,
        old: &str,
    ) -> Result<String, String> {
        self.database
            .query_row(
                "SELECT new_id FROM cached_mappings WHERE domain=?1 AND scope=?2 AND old_id=?3",
                params![domain, scope(domain, meeting), old],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "保存済み復元IDが欠損しています。新規IDへの置換は行いません".into())
    }

    pub(super) fn insert_cached(
        &self,
        domain: &str,
        meeting: &str,
        old: &str,
        new: &str,
    ) -> Result<(), String> {
        let reserved: bool = self
            .database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM reserved_ids WHERE id=?1)",
                params![new],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if reserved {
            return Err("保存済み復元IDが元IDまたは既存データと競合しています".into());
        }
        let mut query = self
            .database
            .prepare("SELECT domain,scope,old_id FROM mappings WHERE new_id=?1")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query(params![new]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let prior: String = row.get(0).map_err(|e| e.to_string())?;
            let owner: String = row.get(1).map_err(|e| e.to_string())?;
            let source: String = row.get(2).map_err(|e| e.to_string())?;
            let speaker = old.starts_with("manual:")
                && new.starts_with("manual:")
                && matches!(
                    (domain, prior.as_str()),
                    ("speakers", "speaker_lineage") | ("speaker_lineage", "speakers")
                );
            let utterance = old.starts_with("split:")
                && new.starts_with("split:")
                && matches!(
                    (domain, prior.as_str()),
                    ("utterances", "utterance_lineage") | ("utterance_lineage", "utterances")
                );
            if owner != meeting || source != old || !(speaker || utterance) {
                return Err("保存済み復元IDの共有が別会議・別対象です".into());
            }
        }
        self.database
            .execute("INSERT OR IGNORE INTO issued_ids VALUES(?1)", params![new])
            .map_err(|e| e.to_string())?;
        self.database
            .execute(
                "INSERT INTO mappings VALUES(?1,?2,?3,?4)",
                params![domain, meeting, old, new],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(super) fn verify_replay(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        let Some(saved) = &self.saved else {
            return check_cancelled(cancelled);
        };
        let mut count = 0_i64;
        let mut hash = Sha256::new();
        self.visit_mappings(cancelled, |map| {
            if self.cached_target(&map.domain, &map.scope, &map.source)? != map.target {
                return Err("再構成した復元IDが保存記録と一致しません".into());
            }
            hash.update(serde_json::to_vec(&map).map_err(|e| e.to_string())?);
            hash.update(b"\n");
            count = count.checked_add(1).ok_or("復元ID件数が範囲外です")?;
            Ok(())
        })?;
        if count != saved.count || format!("{:x}", hash.finalize()) != saved.hash {
            return Err("保存済み復元IDに余分または欠損した対応があります".into());
        }
        Ok(())
    }

    pub(super) fn verify_saved_destination(
        &self,
        db: &Connection,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let Some(saved) = &self.saved else {
            return check_cancelled(cancelled);
        };
        let mut count = 0_i64;
        let mut hash = Sha256::new();
        let mut statement=db.prepare("SELECT domain,source_scope,source_key,target_key FROM restore_identity_maps WHERE batch_id=?1 ORDER BY domain,source_scope,source_key").map_err(|e|e.to_string())?;
        let mut rows = statement
            .query(params![self.batch_id])
            .map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let map = IdentityMap {
                domain: row.get(0).map_err(|e| e.to_string())?,
                scope: row.get(1).map_err(|e| e.to_string())?,
                source: row.get(2).map_err(|e| e.to_string())?,
                target: row.get(3).map_err(|e| e.to_string())?,
            };
            if self
                .lookup(&map.domain, &map.scope, &map.source)?
                .as_deref()
                != Some(&map.target)
            {
                return Err("確認中に保存済み復元ID対応が変更されています".into());
            }
            count = count.checked_add(1).ok_or("復元ID件数が範囲外です")?;
            hash.update(serde_json::to_vec(&map).map_err(|e| e.to_string())?);
            hash.update(b"\n");
        }
        if count != saved.count || format!("{:x}", hash.finalize()) != saved.hash {
            return Err("確認中に保存済み復元ID対応が変更されています".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
