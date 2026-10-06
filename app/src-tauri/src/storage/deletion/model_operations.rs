//! Server-held model deletion selections. IDs and serialized views are not permits.
use super::intent::{
    self,
    removal::{self, renewal},
};
use super::{capture, capture_locked, Entry, Snapshot};
use crate::{analysis_worker::data_maintenance::DataMaintenanceLease, storage::Storage};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub confirmation_id: String,
    pub operation_id: String,
    pub package_id: String,
    pub review_hash: String,
    pub model_name: String,
    pub model_version: String,
    pub models_root: PathBuf,
    pub phase: String,
    pub pending: bool,
    pub execute_allowed: bool,
    pub resume_reason: Option<String>,
    pub keep_allowed: bool,
    pub reconfirmation_allowed: bool,
    pub include_unregistered_recorded: Option<bool>,
    pub path_count: usize,
    pub processed_paths: usize,
    pub file_count: usize,
    pub missing_count: usize,
    pub changed_count: usize,
    pub unregistered_count: usize,
    pub bytes: u64,
    pub profile_references: u64,
    pub job_references: u64,
    pub active_jobs: u64,
    pub previous_processed_paths: Option<usize>,
    pub previous_path_count: Option<usize>,
    pub prior_deletion_completed: bool,
    pub reservation_roots: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathPage {
    pub confirmation_id: String,
    pub offset: usize,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub operation_id: String,
    pub package_id: String,
    pub body_deleted: bool,
    pub retained: bool,
    pub history_retained: bool,
    pub restart_required: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub operation_id: String,
    pub state: String,
    pub package_id: Option<String>,
    pub model_name: Option<String>,
    pub review_allowed: bool,
    pub error_message: Option<String>,
    pub startup_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub items: Vec<Item>,
    pub write_blocked: bool,
    pub restart_required: bool,
    pub reason: Option<String>,
}

enum Target {
    Fresh(Snapshot),
    Unstarted(intent::Prepared),
    Files(removal::Receipt),
    Reconfirmation(renewal::Review),
    Kept(intent::Prepared),
}
impl Target {
    fn snapshot(&self) -> &Snapshot {
        match self {
            Self::Fresh(s) => s,
            Self::Unstarted(p) | Self::Kept(p) => p.snapshot(),
            Self::Files(r) => r.confirmed().snapshot(),
            Self::Reconfirmation(v) => v.previous().confirmed().snapshot(),
        }
    }
    fn entries(&self) -> Vec<Entry> {
        match self {
            Self::Files(r) => r.entries(),
            Self::Reconfirmation(v) => v.entries(),
            _ => {
                let s = self.snapshot();
                s.observations
                    .iter()
                    .map(|o| {
                        let mut e = o.entry.clone();
                        e.relative_path = if e.relative_path.is_empty() {
                            s.record.package_id.clone()
                        } else {
                            format!("{}/{}", s.record.package_id, e.relative_path)
                        };
                        e
                    })
                    .collect()
            }
        }
    }
    fn verify(
        &self,
        storage: &Storage,
        root: &Path,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        match self {
            Self::Fresh(s) => {
                storage.require_data_writable()?;
                let db = storage
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                super::super::database::stopped_for_model_consent(&db)?;
                if capture_locked(
                    storage,
                    &db,
                    root,
                    &s.record.package_id,
                    &s.record.review_hash,
                    &|| check() || lease.cancelled(),
                )? != *s
                {
                    return Err("モデル削除の確認後に対象が変わりました。再確認してください".into());
                }
                active(lease, check)
            }
            Self::Unstarted(p) => p.verify_current(storage, lease, check),
            Self::Files(r) => r.verify_live(storage, lease, check),
            Self::Reconfirmation(v) => v.verify_live(storage, lease, check),
            Self::Kept(_) => Err("このモデル削除は未開始のまま保持・中止済みです".into()),
        }
    }
}
pub(crate) struct Prepared {
    preview: Preview,
    root: PathBuf,
    target: Target,
}
fn active(lease: &DataMaintenanceLease, check: &impl Fn() -> bool) -> Result<(), String> {
    lease.validate_active()?;
    if check() {
        Err("モデル削除の操作を停止しました。既に消したファイルは復元しません".into())
    } else {
        Ok(())
    }
}
pub(crate) fn authorize_review(storage: &Storage, operation: Option<&str>) -> Result<(), String> {
    if let Some(id) = operation {
        intent::authorize_review(storage, id)
    } else {
        storage.require_data_writable()?;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        storage.managed_anchor.verify()?;
        if super::super::fence::other_pending(&db, None)? {
            Err("別の未完了保守操作を先に確認してください".into())
        } else {
            Ok(())
        }
    }
}
fn load(storage: &Storage, root: &Path, operation: &str) -> Result<Target, String> {
    if !Uuid::parse_str(operation).is_ok_and(|id| id.to_string() == operation) {
        return Err("モデル削除の操作IDが不正です".into());
    }
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    let state: String = db
        .query_row(
            "SELECT state FROM maintenance_operations WHERE id=?1 AND kind='delete_assets'",
            params![operation],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let target = match state.as_str() {
        "prepared" => Target::Unstarted(intent::read(&db, operation)?),
        "installing" | "completed" => Target::Files(removal::Receipt::load(&db, operation)?),
        "recovered" => Target::Kept(intent::read(&db, operation)?),
        _ => return Err("このモデル削除記録は自動採用できません。対象を保持します".into()),
    };
    let saved = match &target {
        Target::Unstarted(p) | Target::Kept(p) => p,
        Target::Files(r) => r.confirmed(),
        _ => unreachable!(),
    };
    if saved.models_root() != root {
        return Err("モデル削除の会議DB・ネイティブモデル領域と保存記録が一致しません".into());
    }
    match &target {
        Target::Files(r) => r.verify_review_anchor(storage, &db)?,
        Target::Unstarted(p) | Target::Kept(p) => {
            if storage.managed_anchor.identity()?
                != (
                    p.snapshot().storage_root.clone(),
                    p.snapshot().database.clone(),
                )
            {
                return Err("未開始モデル削除の保存先・DB実体が一致しません".into());
            }
        }
        _ => unreachable!(),
    }
    Ok(target)
}
pub(crate) fn listing(storage: &Storage, root: &Path) -> Result<Listing, String> {
    let rows = {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        storage.managed_anchor.verify()?;
        // Routing only: audio rows remain visible in the audio list, including
        // invalid/restored receipts. No hint exempts them from the global fence.
        let mut q=db.prepare(&format!("SELECT id,state FROM maintenance_operations WHERE kind='delete_assets' AND {} AND coalesce(CASE WHEN length(CAST(target_manifest_json AS BLOB))<=65536 AND json_valid(target_manifest_json) THEN json_extract(target_manifest_json,'$.subtype') ELSE NULL END,'') NOT IN ('audioOnlyDeletion','restoredAudioUnavailable') ORDER BY created_at,id LIMIT 101",super::super::fence::predicate(&db)?)).map_err(|e|e.to_string())?;
        let result = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        result
    };
    if rows.len() > 100 {
        return Err("未完了モデル削除の件数が安全上限を超えています".into());
    }
    let items = rows
        .into_iter()
        .map(|(operation_id, state)| -> Result<Item, String> {
            let startup_reason = {
                let db = storage
                    .connection
                    .lock()
                    .map_err(|_| "DBロックを取得できません")?;
                super::super::startup::reason(&db, &operation_id)?.filter(|reason| {
                    matches!(
                        reason.as_str(),
                        "model_completed_scope_reappeared"
                            | "model_completed_receipt_invalid"
                            | "model_completed_scope_uninspectable"
                    )
                })
            };
            Ok(match load(storage, root, &operation_id) {
                Ok(target) => Item {
                    operation_id,
                    state,
                    package_id: Some(target.snapshot().record.package_id.clone()),
                    model_name: Some(target.snapshot().record.manifest.model_name.clone()),
                    review_allowed: true,
                    error_message: None,
                    startup_reason,
                },
                Err(reason) => Item {
                    operation_id,
                    state,
                    package_id: None,
                    model_name: None,
                    review_allowed: false,
                    error_message: Some(reason),
                    startup_reason,
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Listing {
        items,
        write_blocked: storage.data_write_blocked(),
        restart_required: storage.restart_required(),
        reason: storage.require_data_writable().err(),
    })
}
impl Prepared {
    fn static_preview(
        storage: &Storage,
        root: &Path,
        operation: &str,
        target: &Target,
    ) -> Result<Preview, String> {
        let s = target.snapshot();
        let entries = target.entries();
        let mut p = Preview {
            confirmation_id: Uuid::new_v4().to_string(),
            operation_id: operation.into(),
            package_id: s.record.package_id.clone(),
            review_hash: s.record.review_hash.clone(),
            model_name: s.record.manifest.model_name.clone(),
            model_version: s.record.manifest.model_version.clone(),
            models_root: root.into(),
            phase: String::new(),
            pending: !matches!(target, Target::Fresh(_) | Target::Kept(_)),
            execute_allowed: false,
            resume_reason: None,
            keep_allowed: matches!(target, Target::Unstarted(_) | Target::Kept(_)),
            reconfirmation_allowed: false,
            include_unregistered_recorded: None,
            path_count: entries.len(),
            processed_paths: 0,
            file_count: 0,
            missing_count: 0,
            changed_count: 0,
            unregistered_count: 0,
            bytes: 0,
            profile_references: s.profile_references,
            job_references: s.job_references,
            active_jobs: s.active_jobs,
            previous_processed_paths: None,
            previous_path_count: None,
            prior_deletion_completed: false,
            reservation_roots: Vec::new(),
        };
        for e in entries {
            p.file_count += usize::from(!e.directory);
            p.missing_count += usize::from(!e.present);
            p.changed_count += usize::from(e.changed);
            p.unregistered_count += usize::from(!e.registered);
            p.bytes = p
                .bytes
                .checked_add(e.bytes.unwrap_or(0))
                .ok_or("モデル削除の容量が範囲外です")?;
        }
        p.phase = match target {
            Target::Fresh(_) => "review",
            Target::Unstarted(saved) => {
                p.include_unregistered_recorded = Some(saved.include_unregistered());
                "not_started"
            }
            Target::Kept(_) => "retained",
            Target::Files(r) => {
                p.processed_paths = r.processed_paths();
                p.prior_deletion_completed = r.previously_completed();
                p.reservation_roots = r.roots()?;
                p.include_unregistered_recorded = Some(r.recorded_policy(storage)?);
                if r.completed() {
                    "awaiting_ack"
                } else {
                    "files"
                }
            }
            Target::Reconfirmation(v) => {
                let r = v.previous();
                p.previous_processed_paths = Some(r.processed_paths());
                p.previous_path_count = Some(r.entries().len());
                p.prior_deletion_completed = r.previously_completed();
                p.reservation_roots = v.roots();
                "reconfirmation"
            }
        }
        .into();
        Ok(p)
    }
    fn build(
        storage: &Storage,
        root: PathBuf,
        operation: String,
        target: Target,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        let mut preview = Self::static_preview(storage, &root, &operation, &target)?;
        match target.verify(storage, &root, lease, check) {
            Ok(()) => preview.execute_allowed = true,
            Err(reason) => preview.resume_reason = Some(reason),
        };
        if matches!(target, Target::Files(_)) && !preview.execute_allowed {
            preview.reconfirmation_allowed =
                renewal::review(storage, &operation, lease, check).is_ok();
        }
        active(lease, check)?;
        Ok(Self {
            preview,
            root,
            target,
        })
    }
    pub(crate) fn fresh(
        storage: &Storage,
        root: &Path,
        package: &str,
        review: &str,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        authorize_review(storage, None)?;
        let s = capture(storage, root, package, review, &|| {
            check() || lease.cancelled()
        })?;
        Self::build(
            storage,
            root.into(),
            Uuid::new_v4().to_string(),
            Target::Fresh(s),
            lease,
            check,
        )
    }
    pub(crate) fn pending(
        storage: &Storage,
        root: &Path,
        operation: &str,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        active(lease, check)?;
        authorize_review(storage, Some(operation))?;
        Self::build(
            storage,
            root.into(),
            operation.into(),
            load(storage, root, operation)?,
            lease,
            check,
        )
    }
    pub(crate) fn preview(&self) -> Preview {
        self.preview.clone()
    }
    pub(crate) fn page(&self, confirmation: &str, offset: usize) -> Result<PathPage, String> {
        self.confirmation(confirmation)?;
        let entries = self.target.entries();
        if offset > entries.len() || offset % 50 != 0 {
            return Err("モデル削除一覧の位置が不正です".into());
        }
        let end = offset.saturating_add(50).min(entries.len());
        Ok(PathPage {
            confirmation_id: confirmation.into(),
            offset,
            total: entries.len(),
            next_offset: (end < entries.len()).then_some(end),
            entries: entries[offset..end].to_vec(),
        })
    }
    fn confirmation(&self, id: &str) -> Result<(), String> {
        if id == self.preview.confirmation_id {
            Ok(())
        } else {
            Err("モデル削除の確認内容が変更されています。現在の一覧を再確認してください".into())
        }
    }
    pub(crate) fn refresh(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        let target = if let Target::Reconfirmation(_) = &self.target {
            Target::Reconfirmation(renewal::review(
                storage,
                &self.preview.operation_id,
                lease,
                check,
            )?)
        } else if matches!(self.target, Target::Fresh(_)) && !storage.data_write_blocked() {
            Target::Fresh(capture(
                storage,
                &self.root,
                &self.preview.package_id,
                &self.preview.review_hash,
                &|| check() || lease.cancelled(),
            )?)
        } else {
            load(storage, &self.root, &self.preview.operation_id)?
        };
        *self = Self::build(
            storage,
            self.root.clone(),
            self.preview.operation_id.clone(),
            target,
            lease,
            check,
        )?;
        Ok(())
    }
    pub(crate) fn review_remaining(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        confirmation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        self.confirmation(confirmation)?;
        if !self.preview.reconfirmation_allowed {
            return Err("このモデル削除段階では残りへの再同意を開始できません".into());
        }
        let target = Target::Reconfirmation(renewal::review(
            storage,
            &self.preview.operation_id,
            lease,
            check,
        )?);
        *self = Self::build(
            storage,
            self.root.clone(),
            self.preview.operation_id.clone(),
            target,
            lease,
            check,
        )?;
        Ok(())
    }
    pub(crate) fn execute(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        confirmation: &str,
        include_unregistered: bool,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Receipt, String> {
        active(lease, check)?;
        self.confirmation(confirmation)?;
        if !self.preview.execute_allowed {
            return Err(self
                .preview
                .resume_reason
                .clone()
                .unwrap_or("モデル削除対象を再確認してください".into()));
        }
        if include_unregistered
            && !matches!(self.target, Target::Fresh(_) | Target::Reconfirmation(_))
        {
            return Err("保存済みモデル削除の範囲・未登録物方針を変更できません".into());
        }
        let result = self.execute_inner(storage, lease, include_unregistered, check, progress);
        if result.is_err() {
            if let Ok(target) = load(storage, &self.root, &self.preview.operation_id) {
                self.target = target;
            }
            if let Ok(next) = Self::static_preview(
                storage,
                &self.root,
                &self.preview.operation_id,
                &self.target,
            ) {
                self.preview = next;
            }
            self.preview.execute_allowed = false;
            self.preview.resume_reason = Some(
                "処理を停止しました。消したファイルは復元していません。残りを再検査してください"
                    .into(),
            );
            if matches!(self.target, Target::Files(_)) {
                self.preview.reconfirmation_allowed = true;
            }
        }
        result
    }
    fn execute_inner(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        include: bool,
        check: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Receipt, String> {
        active(lease, check)?;
        if let Target::Fresh(s) = &self.target {
            let hash = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(s).map_err(|e| e.to_string())?)
            );
            progress("fixing_model_delete_scope");
            let saved = intent::prepare(
                storage,
                &self.root,
                &self.preview.operation_id,
                &self.preview.package_id,
                &self.preview.review_hash,
                &hash,
                include,
                lease,
                check,
            )?;
            self.target = Target::Unstarted(saved);
            self.preview.pending = true;
            self.preview.keep_allowed = true;
            progress("model_delete_scope_fixed");
        }
        if let Target::Reconfirmation(view) = &self.target {
            progress("fixing_model_delete_scope");
            let r = renewal::accept(storage, view, view.census_hash(), include, lease, check)?;
            self.target = Target::Files(r);
            self.preview.pending = true;
            self.preview.keep_allowed = false;
            progress("model_delete_scope_fixed");
        }
        if matches!(self.target, Target::Unstarted(_)) {
            active(lease, check)?;
            let r = removal::start(storage, &self.preview.operation_id, lease, check)?;
            self.target = Target::Files(r);
            self.preview.keep_allowed = false;
        }
        if matches!(self.target, Target::Files(_)) {
            progress("deleting_model_files");
            let r = removal::run(storage, &self.preview.operation_id, lease, check)?;
            if !r.completed() {
                return Err("モデル削除の最終状態を検証できません".into());
            }
            self.target = Target::Files(r);
            self.preview = Self::static_preview(
                storage,
                &self.root,
                &self.preview.operation_id,
                &self.target,
            )?;
            self.preview.phase = "completed".into();
            self.preview.pending = false;
            self.preview.execute_allowed = false;
            progress("model_delete_completed");
            Ok(self.receipt(storage, true))
        } else {
            Err("保持・中止済みモデルは削除できません".into())
        }
    }
    fn receipt(&self, storage: &Storage, deleted: bool) -> Receipt {
        Receipt {
            operation_id: self.preview.operation_id.clone(),
            package_id: self.preview.package_id.clone(),
            body_deleted: deleted,
            retained: !deleted,
            history_retained: true,
            restart_required: storage.restart_required(),
        }
    }
    pub(crate) fn keep(
        &mut self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        confirmation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<Receipt, String> {
        active(lease, check)?;
        self.confirmation(confirmation)?;
        if !matches!(self.target, Target::Unstarted(_) | Target::Kept(_)) {
            return Err("保持・中止できるのはモデル削除を開始していない記録だけです".into());
        }
        intent::keep_unstarted(storage, &self.preview.operation_id, lease, check)?;
        self.target = load(storage, &self.root, &self.preview.operation_id)?;
        self.preview = Self::static_preview(
            storage,
            &self.root,
            &self.preview.operation_id,
            &self.target,
        )?;
        self.preview.phase = "retained".into();
        self.preview.pending = false;
        self.preview.execute_allowed = false;
        Ok(self.receipt(storage, false))
    }
}
