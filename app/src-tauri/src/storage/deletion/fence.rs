//! Persistent deletion admission. Ordinary writes never clear a pending intent.
use crate::storage::Storage;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection,
};
use std::sync::atomic::{AtomicBool, Ordering};

// Meeting/model v3 residual plans keep the completed SQL state immutable. This predicate
// is only a conservative read/admission hint; typed receipt validation still
// precedes every privileged writer and all file/row mutation.
pub(in crate::storage) const UNFINISHED: &str = "(state IN ('prepared','installing','cleanup','attention') OR (kind='delete_meeting' AND state='completed' AND json_extract(progress_json,'$.version')=4) OR (kind='delete_assets' AND state='completed' AND json_extract(progress_json,'$.version')=4 AND json_extract(progress_json,'$.phase')='removing'))";
// These connection-local hints are not durable consent or a deletion permit.
// Keep other_pending below restricted to actual unfinished operations: several
// independent completed receipts can be reviewed one at a time under the real
// shared lease, while ordinary writes stay blocked until all hints resolve.
// Invalid retained records also fence writes; a recovered state must not hide
// a startup-detected orphan or make its historical consent executable again.
pub(in crate::storage) fn predicate(db: &Connection) -> Result<String, String> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE type='table' AND name='mon_deletion_startup_attention')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    Ok(if exists {
        format!("({UNFINISHED} OR (kind IN ('delete_meeting','delete_assets') AND state IN ('completed','recovered') AND id IN (SELECT operation_id FROM temp.mon_deletion_startup_attention)))")
    } else {
        UNFINISHED.into()
    })
}
pub(super) fn other_pending(db: &Connection, operation: Option<&str>) -> Result<bool, String> {
    db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE (?1 IS NULL OR id!=?1) AND {UNFINISHED})"), rusqlite::params![operation], |r| r.get(0)).map_err(|e|e.to_string())
}

pub(in crate::storage) fn has_pending(db: &Connection) -> Result<bool, String> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='maintenance_operations')", [], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !exists {
        return Ok(false);
    }
    let pending = predicate(db)?;
    db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE kind IN ('delete_meeting','delete_assets') AND {pending})"), [], |r|r.get(0)).map_err(|e|e.to_string())
}
pub(in crate::storage) fn readonly(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        AuthAction::Function { function_name }
            if !matches!(function_name, "load_extension" | "writefile") =>
        {
            Authorization::Allow
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value: None,
        } if matches!(
            pragma_name,
            "foreign_keys"
                | "foreign_key_check"
                | "user_version"
                | "schema_version"
                | "page_count"
                | "page_size"
                | "query_only"
                | "journal_mode"
                | "secure_delete"
                | "freelist_count"
        ) =>
        {
            Authorization::Allow
        }
        AuthAction::Pragma { pragma_name, .. }
            if matches!(
                pragma_name,
                "table_info" | "table_xinfo" | "foreign_key_list" | "index_list" | "index_info"
            ) =>
        {
            Authorization::Allow
        }
        _ => Authorization::Deny,
    }
}
pub(in crate::storage) struct Fence {
    connection_identity: usize,
    pending: AtomicBool,
    writing: AtomicBool,
    scoped: AtomicBool,
    // Startup skipped mutable recovery. Resolving the receipt is not proof that
    // those recoveries ran; ordinary use still requires a clean reopen.
    startup_deferred: bool,
}
impl Fence {
    pub(in crate::storage) fn open(db: &Connection) -> Result<Self, String> {
        let pending = has_pending(db)?;
        let result = Self {
            connection_identity: connection_identity(db),
            pending: AtomicBool::new(pending),
            writing: AtomicBool::new(false),
            scoped: AtomicBool::new(false),
            startup_deferred: pending,
        };
        if pending {
            result.install(db)?;
        }
        Ok(result)
    }
    pub(in crate::storage) fn blocked(&self) -> bool {
        (self.pending.load(Ordering::Acquire) && !self.scoped.load(Ordering::Acquire))
            || self.startup_deferred
    }
    pub(in crate::storage) fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }
    pub(in crate::storage) fn restart_required(&self) -> bool {
        self.startup_deferred && !self.pending()
    }
    pub(in crate::storage) fn resume_restart_required(&self) -> bool {
        self.startup_deferred
    }
    fn install(&self, db: &Connection) -> Result<(), String> {
        db.flush_prepared_statement_cache();
        let result = if self.blocked() {
            db.authorizer(Some(readonly))
        } else if self.scoped.load(Ordering::Acquire) {
            db.authorizer(Some(scoped_authorizer))
        } else {
            db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        };
        if let Err(error) = result {
            self.scoped.store(false, Ordering::Release);
            self.pending.store(true, Ordering::Release);
            let _ = db.authorizer(Some(|_: AuthContext<'_>| Authorization::Deny));
            return Err(format!("削除の書込停止を維持できません: {error}"));
        }
        Ok(())
    }
    pub(super) fn arm(&self, db: &Connection) -> Result<(), String> {
        self.pending.store(true, Ordering::Release);
        if self.writing.load(Ordering::Acquire) {
            return Ok(());
        }
        self.install(db)
    }

    pub(in crate::storage) fn scoped_update(
        &self,
        db: &Connection,
        action: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.startup_deferred {
            return Err("再起動後の削除は専用の回復操作で確認してください".into());
        }
        db.flush_prepared_statement_cache();
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|e| e.to_string())?;
        let result = action();
        if result.is_err() {
            self.fail_closed(db);
            return result;
        }
        let reserved = super::super::meeting_access::any_reserved(db);
        match reserved {
            Ok(scoped) => self.scoped.store(scoped, Ordering::Release),
            Err(error) => {
                self.fail_closed(db);
                return Err(error);
            }
        }
        self.install(db)?;
        result
    }

    pub(in crate::storage) fn fail_closed(&self, db: &Connection) {
        self.scoped.store(false, Ordering::Release);
        self.pending.store(true, Ordering::Release);
        db.flush_prepared_statement_cache();
        let _ = db.authorizer(Some(readonly));
    }
}

// The trigger definitions enforce row ownership. Ordinary callers cannot
// change their control rows, schema, or protection PRAGMAs.
fn scoped_authorizer(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Insert { table_name }
        | AuthAction::Update { table_name, .. }
        | AuthAction::Delete { table_name }
            if context.database_name == Some("main")
                && !matches!(
                    table_name,
                    "maintenance_operations" | "maintenance_paths" | "schema_migrations"
                ) =>
        {
            Authorization::Allow
        }
        _ => readonly(context),
    }
}

fn connection_identity(db: &Connection) -> usize {
    // SAFETY: only compare the live SQLite handle's address; never dereference
    // it or call SQLite through it. Moving Connection does not change its handle.
    // Storage retains this Connection for the lifetime of its Fence.
    unsafe { db.handle() as usize }
}

// A connection-wide privileged scope is internal and borrows the locked DB.
// Replacing the authorizer on BOTH boundaries invalidates already-prepared VMs.
// It is not a capability reconstructed from a client-provided ID or JSON.
pub(super) struct Writer<'a> {
    storage: &'a Storage,
    db: &'a Connection,
}
impl Writer<'_> {
    fn begin<'a>(
        storage: &'a Storage,
        db: &'a Connection,
        require_pending: bool,
    ) -> Result<Writer<'a>, String> {
        if storage.retired.load(Ordering::Acquire)
            || storage.recovery_hold
            || storage.maintenance.load(Ordering::Acquire)
            || (require_pending && !storage.deletion_fence.pending())
            || (!require_pending && has_pending(db)?)
            || !db.is_autocommit()
            || storage.deletion_fence.connection_identity != connection_identity(db)
        {
            return Err("削除確認記録の保守状態が一致しません".into());
        }
        #[cfg(any(unix, windows))]
        storage.managed_anchor.verify()?;
        if storage.deletion_fence.writing.swap(true, Ordering::AcqRel) {
            return Err("削除の保守書込みが既に進行中です".into());
        }
        db.flush_prepared_statement_cache();
        if let Err(error) = db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>) {
            storage
                .deletion_fence
                .writing
                .store(false, Ordering::Release);
            storage.deletion_fence.fail_closed(db);
            return Err(error.to_string());
        }
        if let Err(error) = super::super::meeting_access::bypass(db, true) {
            storage
                .deletion_fence
                .writing
                .store(false, Ordering::Release);
            storage.deletion_fence.fail_closed(db);
            return Err(error);
        }
        Ok(Writer { storage, db })
    }
}
impl Drop for Writer<'_> {
    fn drop(&mut self) {
        let fence = &self.storage.deletion_fence;
        // Never hand an uncommitted write transaction back to ordinary callers.
        let rolled_back = self.db.is_autocommit() || self.db.execute_batch("ROLLBACK").is_ok();
        let guards = self
            .db
            .db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .is_ok_and(|enabled| enabled)
            && self
                .db
                .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
                .is_ok_and(|enabled| enabled);
        #[cfg(any(unix, windows))]
        let anchored = self.storage.managed_anchor.verify().is_ok();
        #[cfg(not(any(unix, windows)))]
        let anchored = true;
        let pending = has_pending(self.db);
        let guarded = super::super::meeting_access::bypass(self.db, false).is_ok();
        if !rolled_back || !guards || !anchored || pending.is_err() || !guarded {
            fence.scoped.store(false, Ordering::Release);
            fence.pending.store(true, Ordering::Release);
            let _ = self
                .db
                .authorizer(Some(|_: AuthContext<'_>| Authorization::Deny));
        } else {
            fence.pending.store(pending.unwrap(), Ordering::Release);
            let _ = fence.install(self.db);
        }
        fence.writing.store(false, Ordering::Release);
    }
}

pub(super) fn write_fresh<'a>(
    storage: &'a Storage,
    db: &'a Connection,
) -> Result<Writer<'a>, String> {
    Writer::begin(storage, db, has_pending(db)?)
}

#[cfg(any(unix, windows))]
pub(super) fn write_prepared<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    confirmed: &super::intent::Prepared,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    confirmed.verify_saved(db, check)?;
    Writer::begin(storage, db, true)
}

#[cfg(any(unix, windows))]
pub(super) fn write_audio_prepared<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    confirmed: &super::audio::intent::Prepared,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    lease.validate_active()?;
    confirmed.verify_saved(db, check)?;
    Writer::begin(storage, db, true)
}

#[cfg(any(unix, windows))]
pub(super) fn write_audio_reconfirmation<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    review: &super::audio::intent::reconfirmation::Review,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    lease.validate_active()?;
    review.verify(storage, db, check)?;
    Writer::begin(storage, db, has_pending(db)?)
}

#[cfg(any(unix, windows))]
pub(super) fn write_audio_renewal<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    review: &super::audio::intent::removal::renewal::Review,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    lease.validate_active()?;
    review.verify(storage, db, check)?;
    Writer::begin(storage, db, true)
}

#[cfg(any(unix, windows))]
pub(super) fn write_audio_removal<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    receipt: &super::audio::intent::removal::Receipt,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    lease.validate_active()?;
    receipt.verify_saved(db, check)?;
    receipt.verify_anchor(storage)?;
    Writer::begin(storage, db, true)
}

#[cfg(any(unix, windows))]
pub(super) fn write_model_prepared<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    confirmed: &super::models::intent::Prepared,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
) -> Result<Writer<'a>, String> {
    lease.validate_model_deletion()?;
    confirmed.verify_saved(db)?;
    Writer::begin(storage, db, true)
}

#[cfg(any(unix, windows))]
pub(super) fn write_model_reconfirmation<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    review: &super::models::intent::removal::renewal::Review,
    confirmed: &super::models::intent::removal::Receipt,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    lease.validate_model_deletion()?;
    confirmed.verify_saved(db)?;
    review.verify(storage, db, check)?;
    let discovered = super::startup::contains(db, confirmed.operation())?;
    let cold_pending =
        storage.deletion_fence.resume_restart_required() && storage.deletion_fence.pending();
    Writer::begin(
        storage,
        db,
        !confirmed.completed() || discovered || cold_pending,
    )
}

#[cfg(any(unix, windows))]
pub(super) fn write_model_removal<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    confirmed: &super::models::intent::removal::Receipt,
    lease: &crate::analysis_worker::data_maintenance::DataMaintenanceLease,
) -> Result<Writer<'a>, String> {
    lease.validate_model_deletion()?;
    confirmed.verify_saved(db)?;
    confirmed.verify_anchor(storage)?;
    let discovered = super::startup::contains(db, confirmed.operation())?;
    let cold_pending =
        storage.deletion_fence.resume_restart_required() && storage.deletion_fence.pending();
    Writer::begin(
        storage,
        db,
        !confirmed.completed() || discovered || cold_pending,
    )
}

#[cfg(any(unix, windows))]
pub(super) fn write_removal<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    confirmed: &super::removal::Receipt,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    confirmed.verify_saved(db, check)?;
    Writer::begin(storage, db, true)
}

// Old Scope-only receipts exist only in the row-phase fixtures, not a public
// deletion format. Tests still exercise the real persistent fence and permit.
#[cfg(test)]
pub(super) fn write_scope_fixture<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    operation: &str,
    scope: &super::database::Scope,
) -> Result<Writer<'a>, String> {
    use rusqlite::params;
    scope.validate_format(db)?;
    let (json,hash,consent):(String,String,Option<i64>) = db.query_row("SELECT target_manifest_json,target_hash,consent_at FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting' AND state IN ('cleanup','completed')", params![operation], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if json != scope.encode()? || hash != scope.hash()? || consent.is_none_or(|n| n < 0) {
        return Err("DB段階fixtureの確認記録が一致しません".into());
    }
    Writer::begin(storage, db, true)
}

#[cfg(any(unix, windows))]
pub(super) fn write_completion<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    confirmed: &super::completion::Receipt,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    confirmed.verify_saved(db, check)?;
    // A completed acknowledgement retry can checkpoint only after validating
    // its full receipt; it never grants the old raw Scope fixture admission.
    let discovered = super::startup::contains(db, confirmed.confirmed().operation())?;
    // A cold continuation can have reached its canonical final wrapper while
    // independent completed histories still need attention. It may finish its
    // verified acknowledgement without unlocking those other histories.
    let cold_pending =
        storage.deletion_fence.resume_restart_required() && storage.deletion_fence.pending();
    Writer::begin(
        storage,
        db,
        !confirmed.completed() || discovered || cold_pending,
    )
}
