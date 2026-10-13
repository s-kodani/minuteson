//! Server-owned confirmation for the shared data controller. Serialized views
//! contain no authority; client IDs never construct a scope or a file permit.
use super::{
    completion,
    database::Scope,
    files::{Census, Manifest},
    intent, removal, renewal,
};
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::Storage;
use rusqlite::params;
use serde::Serialize;
use std::sync::atomic::Ordering;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub confirmation_id: String,
    pub operation_id: String,
    pub meeting_id: String,
    pub title: String,
    pub phase: String,
    pub pending: bool,
    pub execute_allowed: bool,
    pub resume_reason: Option<String>,
    pub keep_allowed: bool,
    pub path_count: usize,
    pub processed_paths: usize,
    pub file_count: usize,
    pub missing_count: usize,
    pub changed_count: usize,
    pub unregistered_count: usize,
    pub bytes: u64,
    pub row_count: u64,
    pub row_phase_committed: bool,
    pub reconfirmation_allowed: bool,
    pub previous_processed_paths: Option<usize>,
    pub previous_path_count: Option<usize>,
    pub reservation_roots: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathView {
    pub relative_path: String,
    pub directory: bool,
    pub registered: bool,
    pub missing: bool,
    pub changed: bool,
    pub stage_hint: Option<String>,
    pub bytes: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathPage {
    pub confirmation_id: String,
    pub offset: usize,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub entries: Vec<PathView>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub operation_id: String,
    pub meeting_id: String,
    pub deleted: bool,
    pub retained: bool,
    pub restart_required: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub operation_id: String,
    pub state: String,
    pub meeting_id: Option<String>,
    pub error_code: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub items: Vec<Item>,
    pub deleting_meeting_ids: Vec<String>,
    pub write_blocked: bool,
    pub restart_required: bool,
    pub reason: Option<String>,
}

enum Target {
    Fresh(Census),
    Unstarted(intent::Prepared),
    Files(removal::Receipt),
    Reconfirmation {
        files: removal::Receipt,
        review: renewal::Review,
    },
    DatabaseReconfirmation {
        database: completion::Receipt,
        review: renewal::Review,
    },
    Database(completion::Receipt),
    Kept(intent::Prepared),
}
impl Target {
    fn fixed(&self) -> (&Manifest, &Scope) {
        match self {
            Self::Fresh(c) => (c.manifest(), c.scope()),
            Self::Unstarted(p) | Self::Kept(p) => (p.manifest(), p.scope()),
            Self::Files(r) => (r.manifest(), r.confirmed().scope()),
            Self::Reconfirmation { files, review } => {
                (review.manifest(), files.confirmed().scope())
            }
            Self::Database(r) => (r.manifest(), r.confirmed().scope()),
            Self::DatabaseReconfirmation { database, review } => {
                (review.manifest(), database.confirmed().scope())
            }
        }
    }
    fn verify_live(&self, storage: &Storage, check: &impl Fn() -> bool) -> Result<(), String> {
        match self {
            Self::Fresh(c) => c.verify(storage, check),
            Self::Unstarted(p) => p.verify_live(storage, check),
            Self::Files(r) => r.verify_live(storage, check),
            Self::Reconfirmation { files, review } => review.verify_live(storage, files, check),
            Self::Database(r) => r.verify_resume(storage, check),
            Self::DatabaseReconfirmation { database, review } => {
                review.verify_database_live(storage, database, check)
            }
            Self::Kept(_) => Err("この削除は未開始のまま保持・中止済みです".into()),
        }
    }
}
pub(crate) struct Prepared {
    preview: Preview,
    target: Target,
}
fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("削除操作を停止しました。既に消したデータは復元しません".into())
    } else {
        Ok(())
    }
}
fn operation_state(storage: &Storage, operation: &str) -> Result<String, String> {
    if !Uuid::parse_str(operation).is_ok_and(|id| id.to_string() == operation) {
        return Err("削除操作IDが不正です".into());
    }
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    db.query_row(
        "SELECT state FROM maintenance_operations WHERE id=?1 AND kind='delete_meeting'",
        params![operation],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
fn load(storage: &Storage, operation: &str, check: &impl Fn() -> bool) -> Result<Target, String> {
    cancelled(check)?;
    let target = match operation_state(storage, operation)?.as_str() {
        "prepared" => Target::Unstarted(intent::read(storage, operation, check)?),
        "installing" => Target::Files(removal::read(storage, operation, check)?),
        "cleanup" | "completed" => {
            if removal::is_database_files(storage, operation)? {
                Target::Files(removal::read(storage, operation, check)?)
            } else {
                Target::Database(completion::read(storage, operation, check)?)
            }
        }
        "recovered" => Target::Kept(intent::read_keepable(storage, operation, check)?),
        _ => return Err("この削除記録は自動採用できません。対象を保持します".into()),
    };
    let m = target.fixed().0;
    if storage.managed_anchor.identity()? != (m.root.clone(), m.database.clone()) {
        // Kept receipts have no execute route. Completed historical receipts
        // reject resume at a different root and require a new current census
        // and separate consent. This is only their read-only preview route.
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let read_only_history = matches!(target, Target::Kept(_))
            || matches!(&target, Target::Database(receipt) if receipt.completed());
        if !read_only_history
            || !storage
                .managed_anchor
                .matches_history_root(&db, operation, &m.root, &m.database)?
        {
            return Err("削除確認時の管理領域・DBと現在の実体が一致しません".into());
        }
    }
    Ok(target)
}
// Called before acquiring the actual shared worker lease. This permits only
// inspection of a canonical receipt bound to this Storage, never ordinary
// writes or a generic bypass of the persistent deletion fence.
pub(crate) fn authorize_review(storage: &Storage, operation: Option<&str>) -> Result<(), String> {
    if storage.retired.load(Ordering::Acquire) || storage.maintenance.load(Ordering::Acquire) {
        return Err("保存領域の切替・保守が進行中です".into());
    }
    if let Some(operation) = operation {
        load(storage, operation, &|| false)?;
    } else {
        storage.require_data_writable()?;
    }
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    let other = super::fence::other_pending(&db, operation)?;
    if other {
        Err("別の未完了保守操作があります。対象を保持します".into())
    } else {
        Ok(())
    }
}
pub(crate) fn listing(storage: &Storage) -> Result<Listing, String> {
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    let mut q = db.prepare(&format!("SELECT id,state,meeting_id,error_code FROM maintenance_operations WHERE kind='delete_meeting' AND {} ORDER BY created_at,id LIMIT 101", super::fence::predicate(&db)?)).map_err(|e| e.to_string())?;
    let mut items = q
        .query_map([], |r| {
            Ok(Item {
                operation_id: r.get(0)?,
                state: r.get(1)?,
                meeting_id: r.get(2)?,
                error_code: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if items.len() > 100 {
        return Err("未完了削除の件数が安全上限を超えています".into());
    }
    drop(q);
    for item in &mut items {
        if item.error_code.is_none() {
            item.error_code = super::startup::reason(&db, &item.operation_id)?;
        }
    }
    drop(db);
    Ok(Listing {
        items,
        deleting_meeting_ids: storage.deleting_meeting_ids()?,
        write_blocked: storage.data_write_blocked(),
        restart_required: storage.restart_required(),
        reason: storage.require_data_writable().err(),
    })
}

impl Prepared {
    fn build(
        storage: &Storage,
        operation: String,
        target: Target,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        cancelled(check)?;
        let (m, scope) = target.fixed();
        let title = storage
            .get_meeting(&m.meeting_id)?
            .map_or_else(|| format!("会議 {}", m.meeting_id), |meeting| meeting.title);
        let mut preview = Preview {
            confirmation_id: Uuid::new_v4().to_string(),
            operation_id: operation,
            meeting_id: m.meeting_id.clone(),
            title,
            phase: String::new(),
            pending: !matches!(target, Target::Fresh(_) | Target::Kept(_)),
            execute_allowed: false,
            resume_reason: None,
            keep_allowed: matches!(target, Target::Unstarted(_) | Target::Kept(_)),
            path_count: m.entries.len(),
            processed_paths: 0,
            file_count: 0,
            missing_count: 0,
            changed_count: 0,
            unregistered_count: 0,
            bytes: 0,
            row_count: scope.tables.iter().map(|t| t.rows).sum(),
            row_phase_committed: false,
            reconfirmation_allowed: false,
            previous_processed_paths: None,
            previous_path_count: None,
            reservation_roots: m.reservation_roots.clone(),
        };
        for e in &m.entries {
            if e.directory {
                continue;
            }
            preview.file_count += 1;
            if let Some(proof) = &e.proof {
                preview.bytes = preview
                    .bytes
                    .checked_add(proof.size)
                    .ok_or("削除対象容量が範囲外です")?;
                if let Some(asset) = &e.registered {
                    if asset.size as u64 != proof.size
                        || e.sha256.as_deref() != Some(asset.sha256.as_str())
                    {
                        preview.changed_count += 1;
                    }
                } else {
                    preview.unregistered_count += 1;
                }
            } else {
                preview.missing_count += 1;
            }
        }
        preview.phase = match &target {
            Target::Fresh(_) => "review",
            Target::Unstarted(_) => "not_started",
            Target::Files(r) => {
                preview.processed_paths = r.processed_paths();
                if let Some(phase) = r.progress.database_phase {
                    preview.row_phase_committed = phase.rows_committed();
                    "database_files"
                } else {
                    "files"
                }
            }
            Target::Reconfirmation { files, .. } => {
                preview.previous_processed_paths = Some(files.processed_paths());
                preview.previous_path_count = Some(files.manifest().entries.len());
                if let Some(phase) = files.progress.database_phase {
                    preview.row_phase_committed = phase.rows_committed();
                    "database_reconfirmation"
                } else {
                    "reconfirmation"
                }
            }
            Target::DatabaseReconfirmation { database, .. } => {
                preview.previous_processed_paths = Some(database.manifest().entries.len());
                preview.previous_path_count = Some(database.manifest().entries.len());
                preview.row_phase_committed = database.row_phase_committed();
                "database_reconfirmation"
            }
            Target::Database(r) => {
                preview.processed_paths = preview.path_count;
                preview.row_phase_committed = r.row_phase_committed();
                if r.completed() {
                    "awaiting_ack"
                } else {
                    "database"
                }
            }
            Target::Kept(_) => "retained",
        }
        .into();
        match target.verify_live(storage, check) {
            Ok(()) => preview.execute_allowed = true,
            Err(reason) => preview.resume_reason = Some(reason),
        }
        if let Target::Files(files) = &target {
            preview.reconfirmation_allowed =
                !preview.execute_allowed && renewal::Review::capture(storage, files, check).is_ok();
        }
        if let Target::Database(database) = &target {
            preview.reconfirmation_allowed = !preview.execute_allowed
                && renewal::Review::capture_database(storage, database, check).is_ok();
        }
        cancelled(check)?;
        Ok(Self { preview, target })
    }
    pub(crate) fn fresh(
        storage: &Storage,
        meeting: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        authorize_review(storage, None)?;
        Self::build(
            storage,
            Uuid::new_v4().to_string(),
            Target::Fresh(Census::capture(storage, meeting, check)?),
            check,
        )
    }
    pub(crate) fn pending(
        storage: &Storage,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        authorize_review(storage, Some(operation))?;
        Self::build(
            storage,
            operation.into(),
            load(storage, operation, check)?,
            check,
        )
    }
    pub(crate) fn preview(&self) -> Preview {
        self.preview.clone()
    }
    pub(crate) fn page(&self, confirmation: &str, offset: usize) -> Result<PathPage, String> {
        if confirmation != self.preview.confirmation_id {
            return Err("削除対象の確認が変更されています".into());
        }
        let entries = &self.target.fixed().0.entries;
        if offset > entries.len() || !offset.is_multiple_of(50) {
            return Err("削除対象一覧の位置が不正です".into());
        }
        let end = offset.saturating_add(50).min(entries.len());
        Ok(PathPage {
            confirmation_id: confirmation.into(),
            offset,
            total: entries.len(),
            next_offset: (end < entries.len()).then_some(end),
            entries: entries[offset..end]
                .iter()
                .map(|e| PathView {
                    relative_path: e.path.clone(),
                    directory: e.directory,
                    registered: e.registered.is_some(),
                    missing: e.proof.is_none(),
                    changed: e.registered.as_ref().is_some_and(|a| {
                        e.proof.as_ref().is_some_and(|p| {
                            p.size != a.size as u64
                                || e.sha256.as_deref() != Some(a.sha256.as_str())
                        })
                    }),
                    stage_hint: e.stage_hint.clone(),
                    // Directory proof size is filesystem metadata, not file
                    // content bytes. Keep its identity proof server-side.
                    bytes: if e.directory {
                        None
                    } else {
                        e.proof.as_ref().map(|p| p.size)
                    },
                })
                .collect(),
        })
    }
    pub(crate) fn refresh(
        &mut self,
        storage: &Storage,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let next = if let Target::Reconfirmation { files, .. } = &self.target {
            let saved = removal::read(storage, files.confirmed().operation(), check)?;
            let review = renewal::Review::capture(storage, &saved, check)?;
            Self::build(
                storage,
                self.preview.operation_id.clone(),
                Target::Reconfirmation {
                    files: saved,
                    review,
                },
                check,
            )?
        } else if let Target::DatabaseReconfirmation { database, .. } = &self.target {
            let saved = completion::read(storage, database.confirmed().operation(), check)?;
            let review = renewal::Review::capture_database(storage, &saved, check)?;
            Self::build(
                storage,
                self.preview.operation_id.clone(),
                Target::DatabaseReconfirmation {
                    database: saved,
                    review,
                },
                check,
            )?
        } else if matches!(self.target, Target::Fresh(_)) && !storage.data_write_blocked() {
            let c = Census::capture(storage, &self.preview.meeting_id, check)?;
            Self::build(
                storage,
                self.preview.operation_id.clone(),
                Target::Fresh(c),
                check,
            )?
        } else {
            Self::pending(storage, &self.preview.operation_id, check)?
        };
        *self = next;
        Ok(())
    }
    pub(crate) fn review_remaining(
        &mut self,
        storage: &Storage,
        admission: &DataMaintenanceLease,
        confirmation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        admission.validate_active()?;
        let check = || check() || admission.cancelled();
        self.confirmation(confirmation)?;
        if !self.preview.reconfirmation_allowed {
            return Err("この段階の残り範囲は再同意できません。元の記録と対象を保持します".into());
        }
        let target = match &self.target {
            Target::Files(files) => {
                let saved = removal::read(storage, files.confirmed().operation(), &check)?;
                let review = renewal::Review::capture(storage, &saved, &check)?;
                Target::Reconfirmation {
                    files: saved,
                    review,
                }
            }
            Target::Database(database) => {
                let saved = completion::read(storage, database.confirmed().operation(), &check)?;
                let review = renewal::Review::capture_database(storage, &saved, &check)?;
                Target::DatabaseReconfirmation {
                    database: saved,
                    review,
                }
            }
            _ => return Err("保存済み段階の状態を再確認してください".into()),
        };
        let next = Self::build(storage, self.preview.operation_id.clone(), target, &check)?;
        *self = next;
        Ok(())
    }
    fn confirmation(&self, id: &str) -> Result<(), String> {
        if id == self.preview.confirmation_id {
            Ok(())
        } else {
            Err("削除の確認内容が変更されています。現在の一覧を再確認してください".into())
        }
    }
    pub(crate) fn execute(
        &mut self,
        storage: &Storage,
        admission: &DataMaintenanceLease,
        confirmation: &str,
        include_unregistered: bool,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Receipt, String> {
        admission.validate_active()?;
        let check = || check() || admission.cancelled();
        self.confirmation(confirmation)?;
        if !self.preview.execute_allowed {
            return Err(self
                .preview
                .resume_reason
                .clone()
                .unwrap_or("削除対象を再確認してください".into()));
        }
        let result = (|| {
            let mut include = include_unregistered;
            let renewed = match &mut self.target {
                Target::Reconfirmation { files, review } => {
                    progress("fixing_delete_scope");
                    renewal::accept(storage, files, review, include, admission, &check)?;
                    true
                }
                Target::DatabaseReconfirmation { database, review } => {
                    progress("fixing_delete_scope");
                    renewal::accept_database(
                        storage, database, review, include, admission, &check,
                    )?;
                    true
                }
                _ => false,
            };
            if renewed {
                *self = Self::build(
                    storage,
                    self.preview.operation_id.clone(),
                    load(storage, &self.preview.operation_id, &|| false)?,
                    &|| false,
                )?;
                progress("delete_scope_fixed");
                include = false; // The just-recorded policy is fixed, not a new request choice.
            }
            self.execute_inner(storage, include, &check, progress)
        })();
        if result.is_err() {
            // Recover durable progress even if a transition crossed its commit
            // before returning an error. Do not adopt changed observations.
            if let Ok(target) = load(storage, &self.preview.operation_id, &|| false) {
                if let Ok(next) =
                    Self::build(storage, self.preview.operation_id.clone(), target, &|| {
                        false
                    })
                {
                    *self = next;
                }
            }
            self.preview.execute_allowed = false;
            self.preview.resume_reason=Some("処理を停止しました。消したデータは復元していません。残りの状態を再検査してください".into());
        }
        result
    }
    fn execute_inner(
        &mut self,
        storage: &Storage,
        include_unregistered: bool,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Receipt, String> {
        cancelled(check)?;
        authorize_review(
            storage,
            (!matches!(self.target, Target::Fresh(_)))
                .then_some(self.preview.operation_id.as_str()),
        )?;
        if matches!(self.target, Target::Files(_) | Target::Database(_)) && include_unregistered {
            return Err("開始済み削除の判断・範囲は変更できません".into());
        }
        if let Target::Fresh(c) = &self.target {
            removal::Reviewed::confirm(c, include_unregistered)?;
            progress("fixing_delete_scope");
            let p = intent::prepare(storage, &self.preview.operation_id, c, check)?;
            self.target = Target::Unstarted(p);
            self.preview.pending = true;
            self.preview.phase = "not_started".into();
            self.preview.keep_allowed = true;
            progress("delete_scope_fixed");
        }
        if let Target::Unstarted(p) = &self.target {
            let reviewed =
                removal::Reviewed::confirm_saved(storage, p, include_unregistered, check)?;
            self.target = Target::Files(removal::start(storage, p, &reviewed, check)?);
            self.preview.phase = "files".into();
            self.preview.keep_allowed = false;
        }
        if let Target::Files(files) = &mut self.target {
            progress("deleting_files");
            removal::run(storage, files, check)?;
            self.target = Target::Database(completion::start(storage, files, check)?);
            self.preview.phase = "database".into();
            self.preview.processed_paths = self.preview.path_count;
        }
        if let Target::Database(database) = &mut self.target {
            progress("deleting_database");
            completion::run(storage, database, check)?;
            // A durable completed marker is not the success receipt: run must
            // also validate the live targets and finish the final checkpoint.
            self.preview.phase = "completed".into();
            self.preview.pending = false;
            self.preview.execute_allowed = false;
            self.preview.row_phase_committed = true;
            Ok(self.receipt(storage, true))
        } else {
            Err("保持済みの会議をこの操作で削除できません".into())
        }
    }
    fn receipt(&self, storage: &Storage, deleted: bool) -> Receipt {
        Receipt {
            operation_id: self.preview.operation_id.clone(),
            meeting_id: self.preview.meeting_id.clone(),
            deleted,
            retained: !deleted,
            restart_required: storage.restart_required(),
        }
    }
    pub(crate) fn keep(
        &mut self,
        storage: &Storage,
        admission: &DataMaintenanceLease,
        confirmation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Receipt, String> {
        admission.validate_active()?;
        let check = || check() || admission.cancelled();
        self.confirmation(confirmation)?;
        authorize_review(storage, Some(&self.preview.operation_id))?;
        match &self.target {
            Target::Unstarted(p) | Target::Kept(p) => intent::keep_unstarted(storage, p, &check)?,
            _ => return Err("保持・中止できるのはファイル削除を開始していない記録だけです".into()),
        }
        self.target = load(storage, &self.preview.operation_id, &|| false)?;
        self.preview.phase = "retained".into();
        self.preview.pending = false;
        self.preview.execute_allowed = false;
        Ok(self.receipt(storage, false))
    }
}

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn reconfirmation_count(storage: &Storage) -> Result<i64, String> {
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    db.query_row("SELECT count(*) FROM maintenance_operations WHERE meeting_id IS NULL AND kind='delete_meeting' AND progress_json='{\"confirmationRecorded\":true}'", [], |r| r.get(0)).map_err(|e| e.to_string())
}
