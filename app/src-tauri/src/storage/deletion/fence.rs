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
pub(in crate::storage) const UNFINISHED: &str = "(state IN ('prepared','installing','cleanup','attention') OR (state='completed' AND kind IN ('delete_meeting','delete_assets') AND CASE WHEN length(CAST(progress_json AS BLOB))>4096 OR NOT json_valid(progress_json) THEN 1 ELSE json_extract(progress_json,'$.version')=4 AND (kind='delete_meeting' OR json_extract(progress_json,'$.phase')='removing') END))";
pub(in crate::storage) const MANIFEST_MEETING: &str = "CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 AND json_valid(target_manifest_json) THEN json_extract(target_manifest_json,'$.scope.meetingId') END";
pub(in crate::storage) const MANIFEST_AUDIO_MEETING: &str = "CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 AND json_valid(target_manifest_json) THEN coalesce(json_extract(target_manifest_json,'$.meetingId'),json_extract(target_manifest_json,'$.meeting')) END";

#[derive(Default)]
struct StartupScopes {
    meetings: std::collections::BTreeSet<String>,
    audio: std::collections::BTreeSet<String>,
    models: bool,
    history: bool,
    unknown: bool,
    conflicting_shared_target: bool,
}

fn canonical_meeting(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|id_value| id_value.to_string() == id)
}

// This classification only restricts ordinary access. It does not validate a
// receipt or grant any ability to resume a deletion. Corroborating IDs can
// scope a restriction even if the rest of the receipt is damaged.
fn startup_scopes(db: &Connection) -> Result<StartupScopes, String> {
    let pending = predicate(db)?;
    let mut statement = db.prepare(&format!(
        "SELECT kind,meeting_id,CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 THEN target_manifest_json END,state
         FROM maintenance_operations WHERE kind IN ('delete_meeting','delete_assets') AND {pending}"
    )).map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut scopes = StartupScopes::default();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let decoded = (|| -> rusqlite::Result<_> {
            let kind: String = row.get(0)?;
            let meeting: Option<String> = row.get(1)?;
            let json: Option<String> = row.get(2)?;
            let state: String = row.get(3)?;
            Ok((kind, meeting, json, state))
        })();
        let Ok((kind, meeting, json, state)) = decoded else {
            scopes.unknown = true;
            continue;
        };
        let header: serde_json::Value = json
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        let claimed = match (kind.as_str(), header["subtype"].as_str()) {
            ("delete_meeting", None) => header["scope"]["meetingId"].as_str(),
            ("delete_assets", Some("audioOnlyDeletion")) => header["meetingId"].as_str(),
            _ => None,
        }
        .filter(|id| canonical_meeting(id));
        let historical = matches!(state.as_str(), "completed" | "recovered");
        if !historical
            && claimed
                .zip(meeting.as_deref().filter(|id| canonical_meeting(id)))
                .is_some_and(|(manifest, column)| manifest != column)
        {
            scopes.conflicting_shared_target = true;
            continue;
        }
        let claims = |id: Option<&str>| {
            id.filter(|id| canonical_meeting(id))
                .filter(|id| meeting.as_deref().is_none_or(|column| column == *id))
                .map(str::to_owned)
        };
        match (kind.as_str(), header["subtype"].as_str()) {
            ("delete_meeting", None) => {
                if let Some(target) = claims(claimed) {
                    // Reappeared or unattached historical roots need the
                    // existing whole-store inspection before consent can be
                    // reviewed again. A live confirmation has a meeting FK.
                    if historical || meeting.is_none() {
                        scopes.history = true;
                    } else {
                        scopes.meetings.insert(target);
                    }
                } else {
                    scopes.unknown = true;
                }
            }
            ("delete_assets", Some("audioOnlyDeletion")) => {
                if let Some(target) = claims(claimed) {
                    if historical || meeting.is_none() {
                        scopes.history = true;
                    } else {
                        scopes.audio.insert(target);
                    }
                } else {
                    scopes.unknown = true;
                }
            }
            ("delete_assets", Some("restoredAudioUnavailable"))
                if claims(header["meeting"].as_str()).is_some() =>
            {
                scopes.history = true;
            }
            ("delete_assets", Some("modelDeletion")) if meeting.is_none() => {
                scopes.models = true;
            }
            _ => scopes.unknown = true,
        }
    }
    Ok(scopes)
}

pub(in crate::storage) fn audio_pending(db: &Connection, meeting: &str) -> Result<bool, String> {
    db.query_row(
        &format!("SELECT NOT ({})", audio_unreserved(db, "?1")?),
        [meeting],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

// One list read may inspect many meetings. Reuse only the protected IDs, never
// a file observation or permission to mutate. Every use is under the live DB
// mutex; local writes (including TEMP), DDL, and other connections invalidate it.
#[derive(Default)]
pub(in crate::storage) struct AudioPendingScopes {
    stamp: Option<AudioScopeStamp>,
    meetings: std::collections::HashSet<Vec<u8>>,
}

#[derive(PartialEq, Eq)]
struct AudioScopeStamp {
    connection: usize,
    changes: u64,
    data: i64,
    main_schema: i64,
    temp_schema: i64,
}

impl AudioPendingScopes {
    pub(in crate::storage) fn contains(
        &mut self,
        db: &Connection,
        meeting: &str,
    ) -> Result<bool, String> {
        // A rollback does not undo total_changes. Do not reuse a snapshot of
        // uncommitted changes, even when a caller later rolls those changes back.
        let stamp = db
            .is_autocommit()
            .then(|| -> Result<_, rusqlite::Error> {
                Ok(AudioScopeStamp {
                    connection: connection_identity(db),
                    changes: db.total_changes(),
                    data: db.query_row("PRAGMA main.data_version", [], |r| r.get(0))?,
                    main_schema: db.query_row("PRAGMA main.schema_version", [], |r| r.get(0))?,
                    temp_schema: db.query_row("PRAGMA temp.schema_version", [], |r| r.get(0))?,
                })
            })
            .transpose()
            .map_err(|error| error.to_string())?;
        if stamp.is_none() || self.stamp != stamp {
            let pending = predicate(db)?;
            let mut query = db
                .prepare(&format!(
                    "SELECT meeting_id,{MANIFEST_AUDIO_MEETING} FROM maintenance_operations
                     WHERE kind='delete_assets' AND {pending}"
                ))
                .map_err(|error| error.to_string())?;
            let mut rows = query.query([]).map_err(|error| error.to_string())?;
            let mut meetings = std::collections::HashSet::new();
            while let Some(row) = rows.next().map_err(|error| error.to_string())? {
                for index in 0..2 {
                    // Match the existing TEXT bind's SQLite equality without
                    // requiring UUIDs or decoding corrupt UTF-8. Non-TEXT values
                    // cannot equal a Rust string. Keep BOTH claims on conflict.
                    if let rusqlite::types::ValueRef::Text(value) =
                        row.get_ref(index).map_err(|error| error.to_string())?
                    {
                        meetings.insert(value.to_vec());
                    }
                }
            }
            self.meetings = meetings;
            // Stamp BEFORE the query: an outside commit during the scan must
            // invalidate the next use, never get attached to an older result.
            self.stamp = stamp;
        }
        Ok(self.meetings.contains(meeting.as_bytes()))
    }
}

pub(in crate::storage) fn audio_unreserved(
    db: &Connection,
    column: &str,
) -> Result<String, String> {
    let pending = predicate(db)?;
    Ok(format!(
        "NOT EXISTS(SELECT 1 FROM maintenance_operations
         WHERE kind='delete_assets' AND (meeting_id={column} OR {MANIFEST_AUDIO_MEETING}={column}) AND {pending})"
    ))
}
// These connection-local hints are not durable consent or a deletion permit.
// Keep other_pending below restricted to actual unfinished operations: several
// independent completed receipts can be reviewed one at a time under the real
// shared lease, while ordinary writes stay blocked until all hints resolve.
// Invalid retained records also fence writes; a recovered state must not hide
// a startup-detected orphan or make its historical consent executable again.
pub(in crate::storage) fn predicate(db: &Connection) -> Result<String, String> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_schema WHERE type='table' AND name='mon_deletion_startup_attention')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    let hint = if exists {
        " OR id IN (SELECT operation_id FROM temp.mon_deletion_startup_attention)"
    } else {
        ""
    };
    Ok(format!("({UNFINISHED} OR (kind IN ('delete_meeting','delete_assets') AND state IN ('completed','recovered') AND (typeof(id)!='text' OR length(CAST(id AS BLOB))!=36{hint})))"))
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
                | "data_version"
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
    unknown_startup_scope: bool,
    conflicting_shared_startup_target: bool,
    startup_meeting_scopes: std::collections::BTreeSet<String>,
    startup_audio_scopes: std::collections::BTreeSet<String>,
    // Startup skipped mutable recovery. Resolving the receipt is not proof that
    // those recoveries ran; ordinary use still requires a clean reopen.
    startup_deferred: bool,
}
impl Fence {
    pub(in crate::storage) fn open(db: &Connection) -> Result<Self, String> {
        let (pending, mut scopes) = match has_pending(db) {
            Ok(false) => (false, StartupScopes::default()),
            Ok(true) => (
                true,
                startup_scopes(db).unwrap_or_else(|_| StartupScopes {
                    unknown: true,
                    ..StartupScopes::default()
                }),
            ),
            Err(_) => (
                true,
                StartupScopes {
                    unknown: true,
                    ..StartupScopes::default()
                },
            ),
        };
        if !scopes.unknown
            && !scopes.conflicting_shared_target
            && !scopes.models
            && !scopes.history
            && !scopes.meetings.is_empty()
        {
            let install = (|| -> Result<(), String> {
                super::database::trusted_schema(db)?;
                let tx = db
                    .unchecked_transaction()
                    .map_err(|error| error.to_string())?;
                super::super::meeting_access::install(&tx)?;
                for meeting in &scopes.meetings {
                    tx.execute(
                        "INSERT INTO temp.mon_meeting_deletion_guard VALUES(?1,0)",
                        [meeting],
                    )
                    .map_err(|error| error.to_string())?;
                }
                tx.commit().map_err(|error| error.to_string())
            })();
            if let Err(error) = install {
                crate::internal_logs::report_error("deletion_fence", &error);
                scopes.unknown = true;
            }
        }
        let conflicting_shared_startup_target = scopes.conflicting_shared_target
            && !scopes.unknown
            && !scopes.models
            && !scopes.history
            && scopes.meetings.is_empty()
            && scopes.audio.is_empty();
        let unknown_startup_scope = scopes.unknown || scopes.conflicting_shared_target;
        let startup_deferred = unknown_startup_scope || scopes.models || scopes.history;
        let scoped = pending && !startup_deferred;
        let result = Self {
            connection_identity: connection_identity(db),
            pending: AtomicBool::new(pending),
            writing: AtomicBool::new(false),
            scoped: AtomicBool::new(scoped),
            unknown_startup_scope,
            conflicting_shared_startup_target,
            startup_meeting_scopes: scopes.meetings,
            startup_audio_scopes: scopes.audio,
            startup_deferred,
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
    pub(in crate::storage) fn unknown_startup_scope(&self) -> bool {
        self.unknown_startup_scope
    }
    pub(in crate::storage) fn conflicting_shared_startup_target(&self) -> bool {
        self.conflicting_shared_startup_target && self.pending()
    }
    pub(in crate::storage) fn preserve_startup_meeting(&self, meeting: &str) -> bool {
        self.startup_meeting_scopes.contains(meeting)
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
        match self.scopes_allow_writes(db) {
            Ok(scoped) => self.scoped.store(scoped, Ordering::Release),
            Err(error) => {
                self.fail_closed(db);
                return Err(error);
            }
        }
        self.install(db)?;
        result
    }

    fn scopes_allow_writes(&self, db: &Connection) -> Result<bool, String> {
        let scopes = startup_scopes(db)?;
        if scopes.unknown
            || scopes.models
            || scopes.history
            || !scopes.audio.is_subset(&self.startup_audio_scopes)
        {
            return Ok(false);
        }
        for meeting in &scopes.meetings {
            if !super::super::meeting_access::reserved(db, meeting)? {
                return Ok(false);
            }
        }
        Ok(!scopes.audio.is_empty() || super::super::meeting_access::any_reserved(db)?)
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
            || storage.common_write_hold()
            || storage.deletion_fence.unknown_startup_scope
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
        let normalized = fence.startup_deferred
            || fence.startup_audio_scopes.is_empty()
            || (rolled_back
                && guards
                && anchored
                && pending.is_ok()
                && (|| -> Result<(), String> {
                    let tx = self
                        .db
                        .unchecked_transaction()
                        .map_err(|error| error.to_string())?;
                    for meeting in &fence.startup_audio_scopes {
                        if !audio_pending(&tx, meeting)?
                            && !super::super::meeting_access::reserved(&tx, meeting)?
                        {
                            super::super::normalize_startup_audio_states(&tx, meeting)?;
                        }
                    }
                    tx.commit().map_err(|error| error.to_string())
                })()
                .is_ok());
        let guarded = super::super::meeting_access::bypass(self.db, false).is_ok();
        if !rolled_back || !guards || !anchored || pending.is_err() || !guarded {
            fence.scoped.store(false, Ordering::Release);
            fence.pending.store(true, Ordering::Release);
            let _ = self
                .db
                .authorizer(Some(|_: AuthContext<'_>| Authorization::Deny));
        } else if !normalized {
            fence.fail_closed(self.db);
        } else {
            fence.pending.store(pending.unwrap(), Ordering::Release);
            if !fence.startup_deferred {
                let scoped = fence.scopes_allow_writes(self.db).unwrap_or(false);
                fence.scoped.store(scoped, Ordering::Release);
            }
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

#[cfg(any(unix, windows))]
pub(super) fn write_completion<'a>(
    storage: &'a Storage,
    db: &'a Connection,
    confirmed: &super::completion::Receipt,
    check: &impl Fn() -> bool,
) -> Result<Writer<'a>, String> {
    confirmed.verify_saved(db, check)?;
    // A completed acknowledgement retry can checkpoint only after validating
    // its full receipt.
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
