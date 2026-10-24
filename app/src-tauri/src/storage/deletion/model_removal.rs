//! Fixed model body removal. Immutable history and proof slots are retained.
//! Normal names are never unlinked; only exact, journaled reserved names are.
use super::super::{capture_locked, hash_file, usages, Entry, Observation};
use super::{active, read_with_progress, Prepared};
use crate::storage::deletion::fs::{Directory, Proof};
use crate::{
    analysis_worker::data_maintenance::DataMaintenanceLease,
    storage::{self, Storage},
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, sync::atomic::Ordering};

#[path = "model_renewal.rs"]
pub(crate) mod renewal;
use renewal::Plan;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    before: Proof,
    staged: Option<Proof>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Removing,
    Completed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Progress {
    version: u32,
    phase: Phase,
    census_hash: String,
    next: usize,
    pending: Option<Pending>,
    confirmation: Option<renewal::Reference>,
    previously_completed: bool,
}
impl Progress {
    fn encode(&self) -> Result<String, String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > 4096 {
            return Err("モデル削除の進捗が容量上限を超えています".into());
        }
        Ok(json)
    }
    fn state(&self) -> &'static str {
        if self.phase == Phase::Completed || self.previously_completed {
            "completed"
        } else {
            "installing"
        }
    }
    fn validate(&self, saved: &Prepared, plan: &Plan) -> Result<(), String> {
        let entries = &plan.observations;
        if self.version != 4
            || match &self.confirmation {
                None => self.previously_completed,
                Some(r) => !r.valid(),
            }
            || self.census_hash != saved.header.census_hash
            || self.next > entries.len()
            || (self.phase == Phase::Completed
                && (self.next != entries.len() || self.pending.is_some()))
        {
            return Err("モデル削除の進捗・固定対象が不正です".into());
        }
        if let Some(pending) = &self.pending {
            let target = entries
                .iter()
                .rev()
                .nth(self.next)
                .ok_or("モデル削除カーソルが範囲外です")?;
            let original = target
                .proof
                .as_ref()
                .ok_or("欠落モデルを移動対象へ採用しません")?;
            pending.before.validate(target.entry.directory)?;
            if (target.entry.directory && pending.before.identity != original.identity)
                || (!target.entry.directory && pending.before != *original)
            {
                return Err("モデル移動前の証拠が確認対象と一致しません".into());
            }
            if let Some(staged) = &pending.staged {
                staged.validate(target.entry.directory)?;
                if !staged.matches_after_rename(&pending.before) {
                    return Err("モデル移動後の実体が不正です".into());
                }
            }
        }
        Ok(())
    }
    fn reserved(&self, operation: &str, index: usize) -> String {
        match &self.confirmation {
            Some(reference) => format!(
                ".mon-delete-model-{operation}-g{}-{index}",
                reference.generation
            ),
            None => format!(".mon-delete-model-{operation}-{index}"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Receipt {
    saved: Prepared,
    progress: Progress,
    plan: Plan,
}
impl Receipt {
    pub(in crate::storage::deletion) fn operation(&self) -> &str {
        &self.saved.header.operation_id
    }
    pub(in crate::storage::deletion::models) fn startup_audits(
        &self,
        db: &Connection,
    ) -> Result<Vec<String>, String> {
        renewal::audit_ids(db, &self.saved, &self.progress)
    }
    pub(in crate::storage::deletion::models) fn recorded_policy(
        &self,
        storage: &Storage,
    ) -> Result<bool, String> {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        self.verify_saved(&db)?;
        renewal::recorded_policy(&db, &self.saved, &self.progress)
    }
    pub(in crate::storage::deletion::models) fn confirmed(&self) -> &Prepared {
        &self.saved
    }
    pub(in crate::storage::deletion::models) fn entries(&self) -> Vec<Entry> {
        self.plan
            .observations
            .iter()
            .map(|o| o.entry.clone())
            .collect()
    }
    pub(in crate::storage::deletion::models) fn processed_paths(&self) -> usize {
        self.progress.next
    }
    pub(in crate::storage::deletion::models) fn previously_completed(&self) -> bool {
        self.completed() || self.progress.previously_completed
    }
    pub(in crate::storage::deletion::models) fn roots(&self) -> Result<Vec<String>, String> {
        renewal::review_roots(&self.plan, &self.progress, &self.saved.header.operation_id)
    }
    pub(in crate::storage::deletion::models) fn verify_live(
        &self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        verify_remaining(storage, &db, self, &|| check() || lease.cancelled())?;
        active(lease, check)
    }
    pub(in crate::storage::deletion) fn load(
        db: &Connection,
        operation: &str,
    ) -> Result<Self, String> {
        let (state, json): (String, String) = db
            .query_row(
                "SELECT state,progress_json FROM maintenance_operations WHERE id=?1",
                params![operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let progress: Progress = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if progress.state() != state || progress.encode() != Ok(json.clone()) {
            return Err("モデル削除の進捗が正規形式と一致しません".into());
        }
        let saved = read_with_progress(db, operation, Some((&state, &json)))?;
        let plan = renewal::active_plan(db, &saved, &progress)?;
        progress.validate(&saved, &plan)?;
        Ok(Self {
            saved,
            progress,
            plan,
        })
    }
    pub(in crate::storage::deletion) fn verify_saved(&self, db: &Connection) -> Result<(), String> {
        if Self::load(db, &self.saved.header.operation_id)? != *self {
            return Err("モデル削除の保存進捗が変更されています".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn verify_anchor(
        &self,
        storage: &Storage,
    ) -> Result<(), String> {
        if storage.retired.load(Ordering::Acquire)
            || storage.maintenance.load(Ordering::Acquire)
            || storage.managed_anchor.identity()? != self.plan.identity(&self.saved)
        {
            return Err("モデル削除の現在の同意・保存先・DB実体が一致しません".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn verify_review_anchor(
        &self,
        storage: &Storage,
        db: &Connection,
    ) -> Result<(), String> {
        if self.verify_anchor(storage).is_ok() {
            return Ok(());
        }
        let identity = self.plan.identity(&self.saved);
        if storage.retired.load(Ordering::Acquire)
            || storage.maintenance.load(Ordering::Acquire)
            || !self.completed()
            || !storage.managed_anchor.matches_model_reconfirmation_root(
                db,
                self.operation(),
                &identity.0,
                &identity.1,
            )?
        {
            return Err("モデル削除の再確認用の所有対応が一致しません".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn verify_history_anchor(
        &self,
        db: &Connection,
        anchor: &crate::storage::deletion::fs::RootAnchor,
    ) -> Result<(), String> {
        let identity = self.plan.identity(&self.saved);
        if !anchor.matches_history_root(db, self.operation(), &identity.0, &identity.1)? {
            return Err("モデル削除の有効な同意と保存領域が一致しません".into());
        }
        Ok(())
    }
    pub(crate) fn completed(&self) -> bool {
        self.progress.phase == Phase::Completed
    }
    fn target(&self) -> Option<&Observation> {
        self.plan.observations.iter().rev().nth(self.progress.next)
    }
    fn reserved(&self, index: usize) -> String {
        self.progress
            .reserved(&self.saved.header.operation_id, index)
    }
    fn metadata(&self, storage: &Storage, db: &Connection) -> Result<(), String> {
        self.verify_anchor(storage)?;
        self.metadata_common(storage, db)
    }
    fn metadata_review(&self, storage: &Storage, db: &Connection) -> Result<(), String> {
        self.verify_review_anchor(storage, db)?;
        self.metadata_common(storage, db)
    }
    fn metadata_common(&self, storage: &Storage, db: &Connection) -> Result<(), String> {
        // Cold startup deliberately leaves unrelated stale SQL work unchanged.
        // Only previously completed sources may use this lane; all public
        // callers still require the actual active shared maintenance lease.
        let cold_completed =
            storage.deletion_resume_restart_required() && self.previously_completed();
        if !cold_completed {
            crate::storage::deletion::database::stopped_for_model_consent(db)?;
        }
        let (profiles, jobs, active_jobs) = usages(db, &self.saved.header.package_id)?;
        if crate::storage::deletion::fence::other_pending(
            db,
            Some(&self.saved.header.operation_id),
        )? || storage::model_placements::load_history(db, &self.saved.header.package_id)?
            != Some((
                self.saved.snapshot.record.clone(),
                self.saved.snapshot.placed,
            ))
            || profiles != self.saved.snapshot.profile_references
            || jobs != self.saved.snapshot.job_references
            || (!cold_completed && active_jobs != self.saved.snapshot.active_jobs)
        {
            return Err("モデル削除の配置・履歴・実行状態が変更されています".into());
        }
        Ok(())
    }
    fn save(
        &mut self,
        storage: &Storage,
        db: &Connection,
        next: Progress,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        self.verify_saved(db)?;
        self.metadata(storage, db)?;
        next.validate(&self.saved, &self.plan)?;
        let valid = if next.next == self.progress.next + 1 {
            next.phase == Phase::Removing
                && next.pending.is_none()
                && (self.target().is_some_and(|o| o.proof.is_none())
                    || self
                        .progress
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.staged.is_some()))
        } else if next.next == self.progress.next && next.phase == Phase::Completed {
            self.progress.phase == Phase::Removing
                && next.next == self.plan.observations.len()
                && next.pending.is_none()
                && self.progress.pending.is_none()
        } else if next.next == self.progress.next && next.phase == Phase::Removing {
            match (&self.progress.pending, &next.pending) {
                (None, Some(p)) => p.staged.is_none(),
                (Some(old), Some(p)) => {
                    old.before == p.before && old.staged.is_none() && p.staged.is_some()
                }
                _ => false,
            }
        } else {
            false
        };
        if !valid
            || next.version != self.progress.version
            || next.census_hash != self.progress.census_hash
            || next.confirmation != self.progress.confirmation
            || next.previously_completed != self.progress.previously_completed
        {
            return Err("モデル削除の進捗遷移が不正です".into());
        }
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if tx.execute("UPDATE maintenance_operations SET state=?2,progress_json=?3,updated_at=max(updated_at,?4) WHERE id=?1 AND kind='delete_assets' AND state=?6 AND progress_json=?5",params![self.saved.header.operation_id,next.state(),next.encode()?,storage::now_ms()?,self.progress.encode()?,self.progress.state()]).map_err(|e|e.to_string())?!=1 {
            return Err("モデル削除進捗の確定対象が変更されています".into());
        }
        active(lease, check)?;
        self.verify_anchor(storage)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.progress = next;
        Ok(())
    }
}

fn parent(models: &Directory, plan: &Plan, path: &str) -> Result<Option<Directory>, String> {
    if !path.contains('/') {
        return Ok(None);
    } // Exact selected root uses models itself.
    let mut held: Option<Directory> = None;
    let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
    let mut relative = String::new();
    for component in parent.split('/').filter(|s| !s.is_empty()) {
        relative = if relative.is_empty() {
            component.into()
        } else {
            format!("{relative}/{component}")
        };
        let expected = plan
            .observations
            .iter()
            .find(|o| o.entry.relative_path == relative)
            .ok_or("モデル親の確認対象がありません")?;
        let child = held
            .as_ref()
            .unwrap_or(models)
            .child(component)?
            .ok_or("モデル親が消失しています")?;
        child.verify_identity(
            &expected
                .proof
                .as_ref()
                .ok_or("モデル親の確認実体がありません")?
                .identity,
        )?;
        held = Some(child);
    }
    Ok(held)
}
fn leaf(e: &Entry) -> &str {
    e.relative_path.rsplit('/').next().unwrap()
}
fn observed(dir: &Directory, name: &str, directory: bool) -> Result<Proof, String> {
    if directory {
        dir.child(name)?.ok_or("モデル対象がありません")?.proof()
    } else {
        Proof::read(&crate::native_fs::metadata(&dir.regular(name)?).map_err(|e| e.to_string())?)
    }
}
fn inspect(
    dir: &Directory,
    name: &str,
    target: &Entry,
    proof: &Proof,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    if target.directory {
        let child = dir.child(name)?.ok_or("モデルフォルダーがありません")?;
        child.verify_identity(&proof.identity)?;
        if !child.names(super::super::MAX_ENTRIES)?.is_empty() || child.proof()? != *proof {
            return Err("モデルフォルダーに未削除・未知の内容があります".into());
        }
    } else {
        let (actual, hash) = hash_file(dir, name, check)?;
        if actual != *proof || target.sha256.as_deref() != Some(hash.as_str()) {
            return Err("モデル内容・実体が変更されています".into());
        }
    }
    Ok(())
}
fn verify_remaining(
    storage: &Storage,
    db: &Connection,
    r: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<Directory, String> {
    r.verify_saved(db)?;
    r.metadata(storage, db)?;
    let models = Directory::root(Path::new(&r.saved.header.models_root))?;
    models.verify_identity(&r.saved.snapshot.models_root)?;
    let roots = renewal::review_roots(&r.plan, &r.progress, &r.saved.header.operation_id)?;
    let actual: BTreeMap<String, Observation> =
        renewal::inventory(&models, &r.saved, &roots, check)?
            .into_iter()
            .map(|o| (o.entry.relative_path.clone(), o))
            .collect();
    // Inspect only the model and exact historical/current reservation roots.
    let mut expected = BTreeMap::new();
    for (index, o) in r.plan.observations.iter().rev().enumerate() {
        let path = &o.entry.relative_path;
        let reserved = r.reserved(index);
        let reserved_path = path.rsplit_once('/').map_or_else(
            || reserved.clone(),
            |(parent, _)| format!("{parent}/{reserved}"),
        );
        let original = actual.get(path);
        let staged = actual.get(&reserved_path);
        if index < r.progress.next || o.proof.is_none() {
            if original.is_some() || staged.is_some() {
                return Err("削除済み・欠落モデルが再出現しています".into());
            }
            continue;
        }
        if index == r.progress.next && r.progress.pending.is_some() {
            let pending = r.progress.pending.as_ref().unwrap();
            if let Some(staged) = staged {
                let proof = staged
                    .proof
                    .as_ref()
                    .ok_or("予約モデルの実体がありません")?;
                if original.is_some()
                    || !pending.staged.as_ref().map_or_else(
                        || proof.matches_after_rename(&pending.before),
                        |p| p == proof,
                    )
                {
                    return Err("モデル予約対象が変更されています".into());
                }
                let mut e = o.entry.clone();
                e.relative_path = reserved_path.clone();
                e.registered = false;
                e.changed = false;
                expected.insert(
                    reserved_path,
                    Observation {
                        entry: e,
                        proof: Some(proof.clone()),
                    },
                );
            } else if original.is_some() && pending.staged.is_none() {
                let mut candidate = o.clone();
                candidate.proof = Some(pending.before.clone());
                expected.insert(path.clone(), candidate);
            } else if original.is_some() || pending.staged.is_none() {
                return Err("未確定のモデル移動対象が消失しています".into());
            }
        } else {
            let mut candidate = o.clone();
            if o.entry.directory {
                let proof = original
                    .and_then(|o| o.proof.as_ref())
                    .ok_or("確認モデルフォルダーがありません")?;
                if proof.identity != o.proof.as_ref().unwrap().identity {
                    return Err("モデルフォルダーが置換されています".into());
                }
                candidate.proof = Some(proof.clone());
            }
            expected.insert(path.clone(), candidate);
        }
    }
    if actual != expected {
        return Err("モデル削除対象の追加・変更・消失があります。残りを再確認してください".into());
    }
    models.verify()?;
    Ok(models)
}

pub(crate) fn start(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    active(lease, check)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let saved = super::read(&db, operation)?;
    saved.verify_anchor(storage)?;
    crate::storage::deletion::database::stopped_for_model_consent(&db)?;
    if saved.kept
        || crate::storage::deletion::fence::other_pending(&db, Some(operation))?
        || capture_locked(
            storage,
            &db,
            Path::new(&saved.header.models_root),
            &saved.header.package_id,
            &saved.header.review_hash,
            &|| check() || lease.cancelled(),
        )? != saved.snapshot
    {
        return Err("モデル削除の開始対象が保存した同意と一致しません".into());
    }
    let progress = Progress {
        version: 4,
        phase: Phase::Removing,
        census_hash: saved.header.census_hash.clone(),
        next: 0,
        pending: None,
        confirmation: None,
        previously_completed: false,
    };
    progress.validate(&saved, &Plan::original(&saved))?;
    for (index, o) in saved.snapshot.observations.iter().rev().enumerate() {
        let reserved = format!(".mon-delete-model-{operation}-{index}");
        let path = o
            .entry
            .relative_path
            .rsplit_once('/')
            .map_or_else(|| reserved.clone(), |(p, _)| format!("{p}/{reserved}"));
        if saved
            .snapshot
            .observations
            .iter()
            .any(|e| e.entry.present && e.entry.relative_path == path)
        {
            return Err("モデル削除の予約名が既に確認対象にあります".into());
        }
        if o.entry.relative_path.is_empty()
            && Directory::root(Path::new(&saved.header.models_root))?
                .optional_kind(&reserved)?
                .is_some()
        {
            return Err("モデル削除の予約ルートが既に存在します".into());
        }
    }
    let _writer =
        crate::storage::deletion::fence::write_model_prepared(storage, &db, &saved, lease)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    saved.verify_saved(&tx)?;
    active(lease, check)?;
    if tx.execute("UPDATE maintenance_operations SET state='installing',progress_json=?2,updated_at=max(updated_at,?3) WHERE id=?1 AND state='prepared' AND progress_json=?4",params![operation,progress.encode()?,storage::now_ms()?,super::INITIAL]).map_err(|e|e.to_string())?!=1 { return Err("モデル削除の開始状態が変更されています".into()); }
    active(lease, check)?;
    tx.commit().map_err(|e| e.to_string())?;
    Receipt::load(&db, operation)
}
pub(crate) fn run(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    run_inner(storage, operation, lease, check, &|_| {})
}
fn run_inner(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Receipt, String> {
    active(lease, check)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let mut r = Receipt::load(&db, operation)?;
    let cancelled = || check() || lease.cancelled();
    let _writer = crate::storage::deletion::fence::write_model_removal(storage, &db, &r, lease)?;
    if r.completed() {
        verify_remaining(storage, &db, &r, &cancelled)?;
        super::super::startup::resolve(storage, &db, &r, &cancelled)?;
        return Ok(r);
    }
    while let Some(target) = r.target().cloned() {
        active(lease, check)?;
        let models = verify_remaining(storage, &db, &r, &cancelled)?;
        if target.proof.is_some() {
            let parent = parent(&models, &r.plan, &target.entry.relative_path)?;
            let dir = parent.as_ref().unwrap_or(&models);
            let name = leaf(&target.entry).to_string();
            let reserved = r.reserved(r.progress.next);
            if r.progress.pending.is_none() {
                if dir.optional_kind(&reserved)?.is_some() {
                    return Err("モデル削除予約名が既に存在します".into());
                }
                let before = if target.entry.directory {
                    observed(dir, &name, true)?
                } else {
                    target.proof.clone().unwrap()
                };
                inspect(dir, &name, &target.entry, &before, &cancelled)?;
                let mut next = r.progress.clone();
                next.pending = Some(Pending {
                    before,
                    staged: None,
                });
                r.save(storage, &db, next, lease, check)?;
                event("armed");
            }
            let pending = r.progress.pending.clone().unwrap();
            if pending.staged.is_none() {
                if dir.optional_kind(&name)?.is_some() {
                    inspect(dir, &name, &target.entry, &pending.before, &cancelled)?;
                    event("beforeRename");
                    active(lease, check)?;
                    r.verify_anchor(storage)?;
                    dir.rename_exclusive(&name, &reserved)?;
                    event("renamed");
                } else if dir.optional_kind(&reserved)?.is_none() {
                    return Err("未確定モデルが消失しています".into());
                }
                let staged = observed(dir, &reserved, target.entry.directory)?;
                if !staged.matches_after_rename(&pending.before) {
                    let _ = dir.rename_exclusive(&reserved, &name);
                    return Err("移動モデルが確認実体と一致しません。削除せず保持します".into());
                }
                inspect(dir, &reserved, &target.entry, &staged, &cancelled)?;
                let mut next = r.progress.clone();
                next.pending.as_mut().unwrap().staged = Some(staged);
                r.save(storage, &db, next, lease, check)?;
                event("staged");
            } else if dir.optional_kind(&name)?.is_some() {
                return Err("移動済みモデルの元パスが再出現しました".into());
            }
            event("beforeUnlink");
            active(lease, check)?;
            r.verify_anchor(storage)?;
            let staged = r
                .progress
                .pending
                .as_ref()
                .unwrap()
                .staged
                .as_ref()
                .unwrap();
            if dir.optional_kind(&reserved)?.is_some() {
                inspect(dir, &reserved, &target.entry, staged, &cancelled)?;
                // Verify the entire remaining tree again at the irreversible
                // boundary; callbacks/racing external edits cannot expand it.
                verify_remaining(storage, &db, &r, &cancelled)?;
                dir.unlink_staged(&reserved, target.entry.directory, staged)?;
            } else {
                dir.sync()?;
            }
            event("unlinked");
        }
        let mut next = r.progress.clone();
        next.next += 1;
        next.pending = None;
        r.save(storage, &db, next, lease, check)?;
        event("advanced");
    }
    verify_remaining(storage, &db, &r, &cancelled)?;
    event("beforeComplete");
    active(lease, check)?;
    verify_remaining(storage, &db, &r, &cancelled)?;
    let mut next = r.progress.clone();
    next.phase = Phase::Completed;
    r.save(storage, &db, next, lease, check)?;
    event("completed");
    verify_remaining(storage, &db, &r, &cancelled)?;
    super::super::startup::resolve(storage, &db, &r, &cancelled)?;
    Ok(r)
}

// Informational availability projection, not execution authority. Validate
// each known completed receipt; never trust a producer hint or raw JSON flag.
pub(in crate::storage) fn deleted(db: &Connection, package: &str) -> Result<bool, String> {
    let ids=db.prepare("SELECT id FROM maintenance_operations WHERE kind='delete_assets' AND state='completed' AND json_extract(target_manifest_json,'$.subtype')='modelDeletion' AND json_extract(target_manifest_json,'$.packageId')=?1 ORDER BY id LIMIT 257").map_err(|e|e.to_string())?.query_map(params![package],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if ids.len() > 256 {
        return Err("モデル削除履歴が安全上限を超えています".into());
    }
    for id in &ids {
        let receipt = Receipt::load(db, id)?;
        if !(receipt.completed() || receipt.progress.previously_completed)
            || receipt.saved.header.package_id != package
        {
            return Err("モデル削除の利用状態記録が不正です".into());
        }
    }
    Ok(!ids.is_empty())
}

#[cfg(test)]
pub(in crate::storage::deletion::models) fn run_observed(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<Receipt, String> {
    run_inner(storage, operation, lease, check, event)
}
