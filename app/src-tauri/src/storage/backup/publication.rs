//! Additive publication of a sealed reconstruction. Not yet a UI controller.
//! A single mutex and IMMEDIATE transaction cover append, global merge and the
//! terminal batch state. Never execute SQL or overwrite files from an archive.
use super::*;
use ledger::{BatchPlan, BatchState};
use restore_fixed::workspace::seal::SealedWorkspace;
use restore_ids::RestoreIds;
use rusqlite::config::DbConfig;
use rusqlite::types::Value;
use rusqlite::TransactionBehavior;

mod files;
pub(in crate::storage::backup) mod journal;
use files::Installed;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Publication {
    Published {
        meetings: usize,
        vocabulary: crate::storage::vocabulary::MergeReceipt,
    },
    AlreadyPublished,
}

fn schema(db: &Connection) -> Result<Vec<(String, String, String)>, String> {
    db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name")
        .map_err(|e|e.to_string())?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?
        .collect::<Result<_,_>>().map_err(|e|e.to_string())
}

// Historical immutable children and edit chains cannot be recreated by ordinary
// worker INSERT order (the worker was running then, and is interrupted now).
// Suppress INSERT triggers only on this locked connection, not via DROP/DDL.
// The authorizer disallows all UPDATE/DELETE/DDL/ATTACH while that capability is
// held. Constraints/FKs remain enabled; no INSERT OR REPLACE/IGNORE is used.
struct AppendOnly<'a>(&'a Connection);
impl<'a> AppendOnly<'a> {
    fn begin(db: &'a Connection) -> Result<Self, String> {
        if !db
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .map_err(|e| e.to_string())?
        {
            return Err("保存先DBの制約が無効です".into());
        }
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)
            .map_err(|e| e.to_string())?;
        let guard = Self(db);
        db.flush_prepared_statement_cache();
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Insert { table_name }
                if context.database_name == Some("main")
                    && portable::TABLES.contains(&table_name) =>
            {
                Authorization::Allow
            }
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Recursive => Authorization::Allow,
            _ => Authorization::Deny,
        }))
        .map_err(|e| e.to_string())?;
        Ok(guard)
    }
    fn finish(self) -> Result<(), String> {
        self.0
            .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .map_err(|e| e.to_string())?;
        self.0
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|e| e.to_string())
    }
}
impl Drop for AppendOnly<'_> {
    fn drop(&mut self) {
        // Unwinding and all error branches restore the normal guards too.
        // If enabling triggers fails, retain the append-only authorizer.
        if self
            .0
            .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
            .is_ok()
        {
            let _ = self
                .0
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

pub(in crate::storage::backup) fn append(
    db: &Connection,
    source: &Connection,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let guard = AppendOnly::begin(db)?;
    for table in portable::tables(source)? {
        check_cancelled(check)?;
        let columns = table
            .columns
            .iter()
            .map(|c| format!("\"{}\"", c.name))
            .collect::<Vec<_>>()
            .join(",");
        let filter = match table.name.as_str() {
            "vocabulary_scopes" => " WHERE id<>'global'",
            "vocabulary_entries" => " WHERE scope_id<>'global'",
            _ => "",
        };
        let mut query = source
            .prepare(&format!(
                "SELECT {columns} FROM \"{}\"{filter} ORDER BY rowid",
                table.name
            ))
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([]).map_err(|e| e.to_string())?;
        let mut insert = db
            .prepare(&format!(
                "INSERT INTO \"{}\"({columns}) VALUES({})",
                table.name,
                vec!["?"; table.columns.len()].join(",")
            ))
            .map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(check)?;
            let cells = (0..table.columns.len())
                .map(|i| row.get::<_, Value>(i))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            insert
                .execute(rusqlite::params_from_iter(cells))
                .map_err(|e| e.to_string())?;
        }
    }
    guard.finish()
}

impl Storage {
    pub(in crate::storage::backup) fn publish_restore(
        &self,
        sealed: &SealedWorkspace,
        ids: &RestoreIds<'_>,
        check: &impl Fn() -> bool,
    ) -> Result<Publication, String> {
        self.publish_restore_inner(sealed, ids, None, check)
    }

    pub(super) fn publish_restore_confirmed(
        &self,
        sealed: &SealedWorkspace,
        ids: &RestoreIds<'_>,
        confirmation: (&str, u64),
        check: &impl Fn() -> bool,
    ) -> Result<Publication, String> {
        self.publish_restore_inner(sealed, ids, Some(confirmation), check)
    }

    fn publish_restore_inner(
        &self,
        sealed: &SealedWorkspace,
        ids: &RestoreIds<'_>,
        confirmation: Option<(&str, u64)>,
        check: &impl Fn() -> bool,
    ) -> Result<Publication, String> {
        check_cancelled(check)?;
        #[cfg(not(unix))]
        if !ids.source().deleted_audio.is_empty() {
            return Err("Windowsの削除済み音声の追加復元は保留中です".into());
        }
        sealed.matches_ids(ids, check)?;
        // Maintenance is declared first and drops after the mutex/transaction.
        let maintenance = self.backup_maintenance()?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|e| e.to_string())?;
        if !db
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .map_err(|e| e.to_string())?
            || schema(&db)? != schema(&portable::schema(MIGRATIONS.len())?)?
        {
            return Err("保存先DBの表・索引・トリガーが信頼済み定義と一致しません".into());
        }
        validate_database(&mut db, check)?;
        if let Some(registry) = &self.scratch {
            let operation: Option<String> = db
                .query_row(
                    "SELECT id FROM maintenance_operations WHERE restore_batch_id=?1",
                    params![ids.batch_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(id) = operation {
                let evidence = journal::recovery_evidence(&db, &id)?;
                if registry.retained("restore_addition", &id, &evidence.intent_hash)? {
                    return Err("この復元操作は保持して確認済みです。新しい追加復元を明示して選択してください".into());
                }
            }
        }
        let plan = BatchPlan::new(ids, check)?;
        let mut tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if plan.reserve(&mut tx, check)? == BatchState::Published {
            // A retry is terminal even if the restored meeting was since deleted.
            // Do not remerge global terms or install anything in this branch.
            tx.rollback().map_err(|e| e.to_string())?;
            return Ok(Publication::AlreadyPublished);
        }
        sealed.verify(check)?;
        #[cfg(unix)]
        let unavailable = if ids.source().deleted_audio.is_empty() {
            None
        } else {
            Some(super::restored_audio::Seed::prepare(sealed, ids, check)?)
        };
        ids.check_publication_destination(&tx, sealed.connection(), check)?;
        let merge = sealed.global_merge(&tx, check)?;
        let required = required_bytes(&tx, sealed, ids)?;
        if let Some((vocabulary, maximum_bytes)) = confirmation {
            if merge.confirmation_fingerprint() != vocabulary
                || required
                    .checked_add(CAPACITY_RESERVE)
                    .ok_or("追加復元容量が範囲外です")?
                    > maximum_bytes
            {
                return Err(
                    "確認後に共通用語または必要容量が変更されています。再確認してください".into(),
                );
            }
        }
        check_capacity(&self.root, required)?;
        let meetings = read_meetings(sealed.connection())?.len();
        // Persist stable IDs before installing files. Per-path ownership records
        // must be durable outside the later atomic publication transaction.
        tx.commit().map_err(|e| e.to_string())?;
        let journal = journal::Journal::begin(&db, &self.root, sealed, ids)?;
        let mut installed = Installed::prepare_recorded(&self.root, sealed, journal, check)?;
        let mut tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let operation = (|| {
            if plan.reserve(&mut tx, check)? != BatchState::Prepared {
                return Err("復元公開の状態が準備後に変化しています".into());
            }
            ids.check_publication_destination(&tx, sealed.connection(), check)?;
            tx.execute_batch("PRAGMA defer_foreign_keys=ON;")
                .map_err(|e| e.to_string())?;
            append(&tx, sealed.connection(), check)?;
            let vocabulary = merge.apply(&mut tx, check)?;
            portable::validate_scopes(&tx)?;
            ledger::validate_provenance(&tx, check)?;
            if tx
                .prepare("PRAGMA foreign_key_check")
                .map_err(|e| e.to_string())?
                .query([])
                .map_err(|e| e.to_string())?
                .next()
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err("追加復元の参照整合性が不正です".into());
            }
            if schema(&tx)? != schema(sealed.connection())?
                || !tx
                    .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
                    .map_err(|e| e.to_string())?
            {
                return Err("追加復元後のDB制約が変化しています".into());
            }
            installed.verify(check)?;
            sealed.verify(check)?;
            check_cancelled(check)?;
            let changed = tx.execute(
                "UPDATE restore_batches SET state='published',published_at=?1 WHERE batch_id=?2 AND state='prepared'",
                params![now_ms()?,ids.batch_id],
            ).map_err(|e|e.to_string())?;
            if changed != 1 {
                return Err("復元バッチの同時公開を確定できません".into());
            }
            installed.completed(&tx)?;
            #[cfg(unix)]
            if let Some(seed) = &unavailable {
                crate::storage::deletion::audio::restored::retain(
                    &tx,
                    &self.managed_anchor,
                    seed,
                    check,
                )?;
            }
            super::exclusions::retain(&tx, ids, check)?;
            crate::storage::model_metadata::retain(
                &tx,
                &ids.source().manifest.file_references,
                check,
                &|id| ids.unused_identity(id),
            )?;
            if let Some(catalog) = &ids.source().manifest.model_catalog {
                catalog.retain(&tx, check, &|id| ids.unused_identity(id))?;
            }
            #[cfg(unix)]
            if let Some(seed) = &unavailable {
                crate::storage::deletion::audio::restored::verify_retained(
                    &tx,
                    &self.managed_anchor,
                    seed,
                    check,
                )?;
            }
            check_cancelled(check)?;
            Ok(vocabulary)
        })();
        let vocabulary = match operation {
            Ok(receipt) => receipt,
            Err(error) => {
                if let Err(rollback) = tx.rollback() {
                    installed.keep();
                    maintenance.fail_closed();
                    return Err(format!("追加復元の巻き戻し結果が未確定です。追加物を保持します: {error}; {rollback}"));
                }
                return Err(error);
            }
        };
        match tx.commit() {
            Ok(()) => {}
            Err(error) => {
                // Never remove assets if COMMIT's result is uncertain. A live
                // transaction or unreadable terminal state needs reopen/recovery.
                let state = db
                    .query_row(
                        "SELECT state FROM restore_batches WHERE batch_id=?1",
                        params![ids.batch_id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional();
                match state {
                    Ok(Some(state)) if state == "published" && db.is_autocommit() => {}
                    Ok(None) if db.is_autocommit() => return Err(error.to_string()),
                    Ok(Some(state)) if state == "prepared" && db.is_autocommit() => {
                        return Err(error.to_string())
                    }
                    _ => {
                        installed.keep();
                        maintenance.fail_closed();
                        return Err(format!(
                            "復元公開の結果が未確定です。追加物を保持し再確認が必要です: {error}"
                        ));
                    }
                }
            }
        }
        // No cancellation/error after commit may convert a success into cleanup.
        installed.keep();
        Ok(Publication::Published {
            meetings,
            vocabulary,
        })
    }
}

pub(super) fn required_bytes(
    db: &Connection,
    sealed: &SealedWorkspace,
    ids: &RestoreIds<'_>,
) -> Result<u64, String> {
    let live_size: i64 = db
        .query_row(
            "SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let metadata = metadata_bytes(ids)?;
    u64::try_from(live_size)
        .map_err(|_| "保存先DB容量が不正です")?
        .checked_mul(2)
        .and_then(|n| n.checked_add(sealed.required_bytes()))
        .and_then(|n| n.checked_add(metadata))
        .ok_or("追加復元容量が範囲外です".into())
}

// Include JSON overflow pages, indexes and commit/WAL copies conservatively.
// Model metadata capacity is not included in the meeting-only sealed image.
fn metadata_bytes(ids: &RestoreIds<'_>) -> Result<u64, String> {
    let manifest = &ids.source().manifest;
    let json = serde_json::to_vec(&(&manifest.model_catalog, &manifest.file_references))
        .map_err(|e| e.to_string())?;
    (json.len() as u64)
        .checked_mul(8)
        .and_then(|n| {
            n.checked_add(
                (ids.source().deleted_audio.meetings().count() as u64).checked_mul(65_536 * 8)?,
            )
        })
        .and_then(|n| {
            n.checked_add((manifest.file_references.len() as u64).checked_mul(16 * 1024)?)
        })
        .ok_or("追加モデル履歴の容量が範囲外です".into())
}

#[cfg(test)]
pub(super) mod tests;

#[cfg(test)]
mod model_tests;

#[cfg(all(test, unix))]
mod catalog_tests;
