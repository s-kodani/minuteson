//! Audio files only, retaining every directory, text and historical row.
//! FilesRemoved is NOT operation completion: the durable write fence remains.
use super::super::{audio, MeetingCensus};
use super::{active, read_with_progress, Prepared};
use crate::analysis_worker::data_maintenance::DataMaintenanceLease;
use crate::storage::deletion::fs::NativeProof as _;
use crate::storage::{
    deletion::{
        files::{Entry, MAX_ENTRIES},
        fs::{Directory, Proof},
    },
    Storage,
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

#[path = "audio_completion.rs"]
pub(crate) mod completion;
#[path = "audio_renewal.rs"]
pub(crate) mod renewal;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    before: Proof,
    staged: Option<Proof>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    RemovingAudio,
    AudioFilesRemoved,
    AudioDeletionCompleted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Progress {
    version: u32,
    phase: Phase,
    census_hash: String,
    next: usize,
    pending: Option<Pending>,
    final_audio: Option<Vec<crate::storage::deletion::database::Table>>,
    confirmation: Option<renewal::Reference>,
}
impl Progress {
    fn state(&self) -> &'static str {
        if self.phase == Phase::AudioDeletionCompleted {
            "completed"
        } else if self.phase == Phase::AudioFilesRemoved {
            "cleanup"
        } else {
            "installing"
        }
    }
    fn encode(&self) -> Result<String, String> {
        let json = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if json.len() > 4096 {
            return Err("音声削除の進捗が容量上限を超えています".into());
        }
        Ok(json)
    }
    fn validate(&self, saved: &Prepared, targets: &[Entry]) -> Result<(), String> {
        if self.version != 4
            || self.confirmation.as_ref().is_some_and(|r| !r.valid())
            || self.census_hash != saved.header.census_hash
            || self.next > targets.len()
            || (self.phase != Phase::RemovingAudio
                && (self.next != targets.len() || self.pending.is_some()))
        {
            return Err("音声削除の固定対象・進捗が不正です".into());
        }
        match (&self.phase, &self.final_audio) {
            (Phase::AudioDeletionCompleted, Some(tables)) => {
                completion::validate_tables(saved, tables)?
            }
            (Phase::RemovingAudio | Phase::AudioFilesRemoved, None) => {}
            _ => return Err("音声利用不可の確定証拠と段階が一致しません".into()),
        }
        if let Some(p) = &self.pending {
            let original = targets
                .get(self.next)
                .and_then(|t| t.proof.as_ref())
                .ok_or("欠落音声を移動対象へ採用しません")?;
            p.before.validate(false)?;
            if &p.before != original {
                return Err("移動前の音声実体が削除要求と一致しません".into());
            }
            if let Some(staged) = &p.staged {
                staged.validate(false)?;
                if !staged.matches_after_rename(&p.before) {
                    return Err("予約音声の実体が不正です".into());
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Receipt {
    saved: Prepared,
    progress: Progress,
    targets: Vec<Entry>,
    plan: super::super::Snapshot,
    policy: super::Policy,
    historical_paths: BTreeSet<String>,
    audits: Vec<String>,
    proof_bytes: usize,
}
fn targets(snapshot: &super::super::Snapshot) -> Result<Vec<Entry>, String> {
    snapshot
        .manifest
        .entries
        .iter()
        .filter(|e| !e.directory)
        .filter_map(|e| match snapshot.disposition(e) {
            Ok(d) if audio(&d) => Some(Ok(e.clone())),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .collect()
}
// Provisional canonical metadata, not a removal receipt. Source ancestry must
// be verified oldest-first before this private frame can yield a Receipt.
pub(super) struct SourceFrame(Receipt);
impl SourceFrame {
    pub(super) fn source_saved(&self) -> &Prepared {
        &self.0.saved
    }
    pub(super) fn source_seal(&self, db: &Connection) -> Result<String, String> {
        self.0.source_seal(db)
    }
    pub(super) fn source_bytes(&self) -> usize {
        self.0.proof_bytes
    }
    pub(super) fn verify_available(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        completion::verify_available_state_inner(db, &self.0, check)
    }
    pub(super) fn verify_superseded(
        &self,
        db: &Connection,
        proof: &crate::storage::deletion::audio_supersession::Proof,
    ) -> Result<(), String> {
        let manifest = &self.0.saved.snapshot.manifest;
        if !self.0.completed()
            || !proof.matches_root(
                db,
                self.0.saved.operation(),
                self.0.saved.meeting(),
                &manifest.root,
                &manifest.database,
            )?
        {
            return Err("引き継ぐ音声削除の完了状態・管理領域が一致しません".into());
        }
        Ok(())
    }
    pub(super) fn validate(
        self,
        db: &Connection,
        ancestors: &BTreeSet<String>,
        check: &impl Fn() -> bool,
    ) -> Result<Receipt, String> {
        let verified = renewal::load(db, &self.0.saved, &self.0.progress, Some(ancestors), check)?;
        if verified.snapshot != self.0.plan
            || verified.policy != self.0.policy
            || verified.paths != self.0.historical_paths
            || verified.audits != self.0.audits
            || verified.bytes != self.0.proof_bytes
        {
            return Err("祖先の確認中に残り音声の固定記録が変更されました".into());
        }
        Ok(self.0)
    }
}
impl Receipt {
    pub(in crate::storage::deletion) fn load(
        db: &Connection,
        operation: &str,
        check: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        // One callback type for the substantial metadata reader. Every check
        // still calls the original cancellation probe; only code duplication
        // across hundreds of caller-specific closures is avoided.
        let check = &|| check();
        let frame = Self::source_frame(db, operation, check)?;
        let source = frame
            .0
            .saved
            .header
            .origin
            .as_ref()
            .map(|_| super::reconfirmation::Source::for_prepared(db, &frame.0.saved, check))
            .transpose()?;
        let ancestors = source.as_ref().map_or_else(BTreeSet::new, |s| s.paths());
        if source.as_ref().is_some_and(|s| {
            s.source_bytes()
                .checked_add(frame.source_bytes())
                .is_none_or(|n| n > super::MAX_BYTES)
        }) {
            return Err("音声再確認の証拠合計が容量上限を超えています".into());
        }
        frame.validate(db, &ancestors, check)
    }
    pub(super) fn source_frame(
        db: &Connection,
        operation: &str,
        check: &impl Fn() -> bool,
    ) -> Result<SourceFrame, String> {
        Ok(SourceFrame(Self::load_frame(db, operation, None, check)?))
    }
    pub(super) fn superseded_frame(
        db: &Connection,
        operation: &str,
        proof: &crate::storage::deletion::audio_supersession::Proof,
        check: &impl Fn() -> bool,
    ) -> Result<SourceFrame, String> {
        let frame = SourceFrame(Self::load_frame(db, operation, Some(proof), check)?);
        frame.verify_superseded(db, proof)?;
        Ok(frame)
    }
    fn load_frame(
        db: &Connection,
        operation: &str,
        supersession: Option<&crate::storage::deletion::audio_supersession::Proof>,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        super::super::cancel(check)?;
        let (state,json,len):(String,String,i64)=db.query_row("SELECT state,CASE WHEN length(CAST(progress_json AS BLOB))<=4096 THEN progress_json ELSE '' END,length(CAST(progress_json AS BLOB)) FROM maintenance_operations WHERE id=?1",params![operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
        if !(1..=4096).contains(&len) {
            return Err("音声削除進捗の容量が不正です".into());
        }
        let progress: Progress = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if progress.state() != state || progress.encode()? != json {
            return Err("音声削除進捗が正規形式ではありません".into());
        }
        let saved = match supersession {
            None => read_with_progress(db, operation, Some((&state, &json)), check)?,
            Some(proof) => {
                super::read_frame(db, operation, Some((&state, &json)), Some(proof), check)?
            }
        };
        let plan = renewal::load(db, &saved, &progress, None, check)?;
        let targets = targets(&plan.snapshot)?;
        progress.validate(&saved, &targets)?;
        Ok(Self {
            saved,
            progress,
            targets,
            plan: plan.snapshot,
            policy: plan.policy,
            historical_paths: plan.paths,
            audits: plan.audits,
            proof_bytes: plan.bytes,
        })
    }
    pub(in crate::storage::deletion) fn verify_saved(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if Self::load(db, self.saved.operation(), check)? != *self {
            return Err("音声削除の保存進捗が変更されています".into());
        }
        Ok(())
    }
    pub(in crate::storage::deletion) fn verify_anchor(
        &self,
        storage: &Storage,
    ) -> Result<(), String> {
        self.saved.verify_anchor(storage)
    }
    pub(crate) fn files_removed(&self) -> bool {
        self.progress.phase != Phase::RemovingAudio
    }
    pub(crate) fn completed(&self) -> bool {
        self.progress.phase == Phase::AudioDeletionCompleted
    }
    pub(in crate::storage::deletion) fn source_saved(&self) -> &Prepared {
        &self.saved
    }
    pub(in crate::storage::deletion) fn source_paths(&self) -> BTreeSet<String> {
        self.historical_paths.clone()
    }
    pub(in crate::storage::deletion) fn reconfirmation_source(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Option<super::reconfirmation::Source>, String> {
        self.saved
            .header
            .origin
            .as_ref()
            .map(|_| super::reconfirmation::Source::for_prepared(db, &self.saved, check))
            .transpose()
    }
    pub(super) fn source_bytes(&self) -> usize {
        self.proof_bytes
    }
    pub(super) fn source_seal(&self, db: &Connection) -> Result<String, String> {
        let row: (i64, i64) = db
            .query_row(
                "SELECT rowid,updated_at FROM maintenance_operations WHERE id=?1",
                params![self.saved.operation()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&(
            "completed_audio_source_v4",
            row,
            &self.saved.header,
            &self.saved.snapshot,
            self.saved.consent,
            &self.progress,
            &self.plan,
            &self.policy,
            &self.historical_paths,
        ))
        .map_err(|e| e.to_string())?;
        Ok(super::hash(&bytes))
    }
    pub(crate) fn processed_files(&self) -> usize {
        self.progress.next
    }
    pub(in crate::storage::deletion::audio) fn confirmed(&self) -> &Prepared {
        &self.saved
    }
    pub(in crate::storage::deletion::audio) fn snapshot(&self) -> &super::super::Snapshot {
        &self.plan
    }
    pub(in crate::storage::deletion::audio) fn policy(&self) -> &super::Policy {
        &self.policy
    }
    pub(in crate::storage::deletion::audio) fn verify_live(
        &self,
        storage: &Storage,
        lease: &DataMaintenanceLease,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        active(lease, check)?;
        let db = crate::operation_observation::lock(&storage.connection)?;
        verify_remaining(storage, &db, self, &|| check() || lease.cancelled())?;
        active(lease, check)
    }
    fn reserved(&self, index: usize) -> String {
        renewal::reserved(
            self.saved.operation(),
            self.progress
                .confirmation
                .as_ref()
                .map_or(0, |r| r.generation),
            index,
        )
    }
    fn reserved_path(&self, index: usize) -> String {
        format!(
            "{}/{}",
            self.targets[index].path.rsplit_once('/').unwrap().0,
            self.reserved(index)
        )
    }
    fn metadata(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.verify_anchor(storage)?;
        if crate::storage::deletion::fence::other_pending(db, Some(self.saved.operation()))?
            || self
                .saved
                .capture_scope(db, self.saved.meeting(), self.saved.operation(), check)?
                != self.saved.snapshot.scope
        {
            return Err("保持する音声・文書・依存履歴が変更されています".into());
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
        self.verify_saved(db, check)?;
        self.metadata(storage, db, check)?;
        next.validate(&self.saved, &self.targets)?;
        let valid = if next.next == self.progress.next + 1 {
            next.phase == Phase::RemovingAudio
                && next.pending.is_none()
                && (self
                    .targets
                    .get(self.progress.next)
                    .is_some_and(|e| e.proof.is_none())
                    || self
                        .progress
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.staged.is_some()))
        } else if next.next == self.progress.next && next.phase == Phase::AudioFilesRemoved {
            self.progress.phase == Phase::RemovingAudio
                && next.next == self.targets.len()
                && next.pending.is_none()
                && self.progress.pending.is_none()
        } else if next.next == self.progress.next && next.phase == Phase::RemovingAudio {
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
            || next.final_audio != self.progress.final_audio
            || next.confirmation != self.progress.confirmation
        {
            return Err("音声削除の進捗遷移が不正です".into());
        }
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if tx.execute("UPDATE maintenance_operations SET state=?2,progress_json=?3,updated_at=max(updated_at,?4) WHERE id=?1 AND kind='delete_assets' AND state=?5 AND progress_json=?6",params![self.saved.operation(),next.state(),next.encode()?,crate::storage::now_ms()?,self.progress.state(),self.progress.encode()?]).map_err(|e|e.to_string())?!=1 { return Err("音声削除の進捗確定対象が変更されています".into()); }
        active(lease, check)?;
        self.verify_anchor(storage)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.progress = next;
        Ok(())
    }
}
// Only a typed full-meeting supersession can admit a detached historical FK.
// This does not expose a Receipt or any deletion/available-source capability.
pub(in crate::storage::deletion) fn validate_superseded(
    db: &Connection,
    operation: &str,
    proof: &crate::storage::deletion::audio_supersession::Proof,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    super::reconfirmation::validate_superseded(db, operation, proof, check)
}

pub(in crate::storage::deletion) fn superseded_audit_member(
    db: &Connection,
    operation: &str,
    audit: &str,
    proof: &crate::storage::deletion::audio_supersession::Proof,
) -> Result<bool, String> {
    super::reconfirmation::validate_superseded(db, operation, proof, &|| false)?;
    let receipt = Receipt::load_frame(db, operation, Some(proof), &|| false)?;
    if !receipt.completed() {
        return Err("再確認元の音声履歴が完了していません".into());
    }
    Ok(receipt.audits.iter().any(|id| id == audit))
}

pub(in crate::storage::deletion) fn inspect_audit_member(
    db: &Connection,
    anchor: &crate::storage::deletion::fs::RootAnchor,
    operation: &str,
    audit: &str,
) -> Result<(), String> {
    if crate::storage::deletion::audio_supersession::remaining_audit_member(
        db, anchor, operation, audit,
    )? {
        return Ok(());
    }
    let receipt = Receipt::load(db, operation, &|| false)?;
    if !receipt.audits.iter().any(|id| id == audit)
        || !anchor.matches_history_root(
            db,
            operation,
            &receipt.saved.snapshot.manifest.root,
            &receipt.saved.snapshot.manifest.database,
        )?
    {
        return Err("元の音声削除に接続していない確認記録です".into());
    }
    Ok(())
}

// Compare the complete fixed tree, not only the next audio path. Only exact
// journaled leaf moves may alter direct parent directory timestamps/size.
fn verify_remaining(
    storage: &Storage,
    db: &Connection,
    r: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    r.verify_saved(db, check)?;
    if r.completed() {
        completion::verify_available_state(db, r, check)?;
        if !completion::absent(db, &storage.managed_anchor, r, check)? {
            return Err("削除済み音声が再出現しました。再確認してください".into());
        }
        return r.verify_anchor(storage);
    }
    r.metadata(storage, db, check)?;
    verify_tree(storage, db, r, &r.saved.snapshot.scope, false, check)
}
fn verify_tree(
    storage: &Storage,
    db: &Connection,
    r: &Receipt,
    scope: &crate::storage::deletion::database::Scope,
    unavailable: bool,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let reserved = r
        .progress
        .pending
        .as_ref()
        .map(|_| r.reserved_path(r.progress.next));
    let current =
        MeetingCensus::capture_audio_remaining_locked(storage, db, r, check, reserved.as_deref())?;
    verify_observation(storage, r, scope, unavailable, &current, check)
}
fn verify_observation(
    storage: &Storage,
    r: &Receipt,
    scope: &crate::storage::deletion::database::Scope,
    unavailable: bool,
    current: &MeetingCensus,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    if current.scope() != scope {
        return Err("音声削除の保持範囲が変更されています".into());
    }
    let actual: BTreeMap<_, _> = current
        .manifest()
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.clone()))
        .collect();
    let mut expected: BTreeMap<_, _> = r
        .plan
        .manifest
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.clone()))
        .collect();
    let mut touched = BTreeSet::new();
    for (index, target) in r.targets.iter().enumerate() {
        let staged_path = r.reserved_path(index);
        let original = actual.get(&target.path).filter(|e| e.proof.is_some());
        let staged = actual.get(&staged_path).filter(|e| e.proof.is_some());
        let mut missing = target.clone();
        missing.proof = None;
        missing.sha256 = None;
        missing.stage_hint = None;
        if unavailable {
            if let Some(asset) = &mut missing.registered {
                asset.state = "missing".into();
            }
        }
        let mut absent = || {
            if missing.registered.is_some() {
                expected.insert(target.path.clone(), missing.clone());
            } else {
                expected.remove(&target.path);
            }
        };
        if index < r.progress.next || target.proof.is_none() {
            if original.is_some() || staged.is_some() {
                return Err("削除済み・欠落音声が再出現しています".into());
            }
            absent();
        } else if index == r.progress.next {
            if let Some(p) = &r.progress.pending {
                if let Some(staged) = staged {
                    let proof = staged.proof.as_ref().ok_or("予約音声の証拠がありません")?;
                    if original.is_some()
                        || !p
                            .staged
                            .as_ref()
                            .map_or_else(|| proof.matches_after_rename(&p.before), |s| s == proof)
                    {
                        return Err("音声の予約実体・元パスが変更されています".into());
                    }
                    absent();
                    let mut moved = target.clone();
                    moved.path = staged_path.clone();
                    moved.registered = None;
                    moved.stage_hint = None;
                    moved.proof = Some(proof.clone());
                    expected.insert(staged_path, moved);
                } else if original.is_some() && p.staged.is_none() {
                    // Original exact proof and hash remain in expected.
                } else if original.is_none() && p.staged.is_some() {
                    absent(); // Unlink committed before cursor advancement.
                } else {
                    return Err("未確定の音声移動対象が消失・再出現しています".into());
                }
            }
        }
        if target.proof.is_some()
            && (index < r.progress.next
                || (index == r.progress.next && r.progress.pending.is_some()))
        {
            touched.insert(target.path.rsplit_once('/').unwrap().0.to_string());
        }
    }
    for path in touched {
        let original = expected.get_mut(&path).ok_or("音声の親観測がありません")?;
        let live = actual.get(&path).ok_or("音声の親が消失しています")?;
        if !original.directory
            || !live.directory
            || original.proof.as_ref().map(|p| &p.identity)
                != live.proof.as_ref().map(|p| &p.identity)
        {
            return Err("音声の親フォルダーが置換されています".into());
        }
        original.proof = live.proof.clone();
    }
    if actual != expected {
        return Err(
            "音声削除の確認後に音声・保持ファイルが追加・変更・消失しました。対象を保持します"
                .into(),
        );
    }
    r.verify_anchor(storage)
}
fn check_unlocked(storage: &Storage, r: &Receipt, check: &impl Fn() -> bool) -> Result<(), String> {
    let db = crate::operation_observation::lock(&storage.connection)?;
    r.verify_saved(&db, check)?;
    r.metadata(storage, &db, check)
}
fn verify_unlocked(
    storage: &Storage,
    r: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    if r.completed() {
        let db = crate::operation_observation::lock(&storage.connection)?;
        return verify_remaining(storage, &db, r, check);
    }
    check_unlocked(storage, r, check)?;
    let reserved = r
        .progress
        .pending
        .as_ref()
        .map(|_| r.reserved_path(r.progress.next));
    let current =
        MeetingCensus::capture_audio_remaining_unlocked(storage, r, check, reserved.as_deref())?;
    verify_observation(storage, r, &r.saved.snapshot.scope, false, &current, check)?;
    check_unlocked(storage, r, check)
}
fn save_unlocked(
    storage: &Storage,
    r: &mut Receipt,
    next: Progress,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let db = crate::operation_observation::lock(&storage.connection)?;
    let _writer =
        crate::storage::deletion::fence::write_audio_removal(storage, &db, r, lease, check)?;
    r.save(storage, &db, next, lease, check)
}
fn parent(storage: &Storage, r: &Receipt, path: &str) -> Result<Directory, String> {
    let root = storage.managed_anchor.directory_ref()?;
    let meetings = root.child("meetings")?.ok_or("会議領域がありません")?;
    let mut dir = meetings
        .child(r.saved.meeting())?
        .ok_or("対象会議領域がありません")?;
    let mut relative = format!("meetings/{}", r.saved.meeting());
    let root_entry = r
        .plan
        .manifest
        .entries
        .iter()
        .find(|e| e.path == relative)
        .ok_or("会議ルート観測がありません")?;
    dir.verify_identity(
        &root_entry
            .proof
            .as_ref()
            .ok_or("会議ルート実体がありません")?
            .identity,
    )?;
    let rest = path
        .strip_prefix(&format!("{relative}/"))
        .ok_or("会議外の音声パスです")?;
    if let Some((parents, _)) = rest.rsplit_once('/') {
        for component in parents.split('/') {
            relative = format!("{relative}/{component}");
            let e = r
                .plan
                .manifest
                .entries
                .iter()
                .find(|e| e.path == relative)
                .ok_or("音声親の観測がありません")?;
            dir = dir.child(component)?.ok_or("音声の親がありません")?;
            dir.verify_identity(&e.proof.as_ref().ok_or("音声親の実体がありません")?.identity)?;
        }
    }
    Ok(dir)
}
fn inspect(
    dir: &Directory,
    name: &str,
    target: &Entry,
    proof: &Proof,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    super::super::cancel(check)?;
    let (actual, sha256) = crate::storage::deletion::verification::hash(dir, name, check)?;
    if actual != *proof || target.sha256.as_deref() != Some(sha256.as_str()) {
        return Err("音声内容・容量・実体が確認と一致しません".into());
    }
    Ok(())
}

pub(crate) fn start(
    storage: &Storage,
    operation: &str,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    active(lease, check)?;
    let db = crate::operation_observation::lock(&storage.connection)?;
    let saved = super::read(&db, operation, &|| check() || lease.cancelled())?;
    saved.verify_locked(storage, &db, lease, check)?;
    let targets = targets(&saved.snapshot)?;
    // Renaming a registered file adds a missing original and a staged entry.
    if saved.snapshot.manifest.entries.len() >= MAX_ENTRIES
        && targets
            .iter()
            .any(|e| e.proof.is_some() && e.registered.is_some())
    {
        return Err("音声削除予約の観測容量を確保できません".into());
    }
    let progress = Progress {
        version: 4,
        phase: Phase::RemovingAudio,
        census_hash: saved.header.census_hash.clone(),
        next: 0,
        pending: None,
        final_audio: None,
        confirmation: None,
    };
    let plan = renewal::load(&db, &saved, &progress, None, check)?;
    let r = Receipt {
        saved,
        progress,
        targets,
        plan: plan.snapshot,
        policy: plan.policy,
        historical_paths: plan.paths,
        audits: plan.audits,
        proof_bytes: plan.bytes,
    };
    r.progress.validate(&r.saved, &r.targets)?;
    let observed_paths: BTreeSet<_> = r
        .saved
        .snapshot
        .manifest
        .entries
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    for index in 0..r.targets.len() {
        active(lease, check)?;
        let path = r.reserved_path(index);
        if observed_paths.contains(path.as_str()) {
            return Err("音声削除予約名が既に存在します".into());
        }
    }
    let _writer = crate::storage::deletion::fence::write_audio_prepared(
        storage, &db, &r.saved, lease, check,
    )?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    r.saved.verify_saved(&tx, check)?;
    r.saved.verify_locked(storage, &tx, lease, check)?;
    if tx.execute("UPDATE maintenance_operations SET state='installing',progress_json=?2,updated_at=max(updated_at,?3) WHERE id=?1 AND state='prepared' AND progress_json=?4",params![operation,r.progress.encode()?,crate::storage::now_ms()?,super::INITIAL]).map_err(|e|e.to_string())?!=1 {return Err("音声削除の開始状態が変更されています".into());}
    active(lease, check)?;
    r.verify_anchor(storage)?;
    tx.commit().map_err(|e| e.to_string())?;
    Receipt::load(&db, operation, check)
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
    let mut r = {
        let db = crate::operation_observation::lock(&storage.connection)?;
        Receipt::load(&db, operation, &|| check() || lease.cancelled())?
    };
    let cancelled = || check() || lease.cancelled();
    if r.files_removed() {
        verify_unlocked(storage, &r, &cancelled)?;
        return Ok(r);
    }
    crate::operation_observation::stage("deleting_files", Some(r.targets.len() as u64), "paths");
    crate::operation_observation::advance(r.progress.next as u64);
    while let Some(target) = r.targets.get(r.progress.next).cloned() {
        if target.directory {
            crate::storage::deletion::verification::release_pins();
        }
        active(lease, check)?;
        verify_unlocked(storage, &r, &cancelled)?;
        if let Some(before) = &target.proof {
            let dir = parent(storage, &r, &target.path)?;
            let name = target.path.rsplit('/').next().unwrap();
            let reserved = r.reserved(r.progress.next);
            if r.progress.pending.is_none() {
                inspect(&dir, name, &target, before, &cancelled)?;
                let mut next = r.progress.clone();
                next.pending = Some(Pending {
                    before: before.clone(),
                    staged: None,
                });
                save_unlocked(storage, &mut r, next, lease, check)?;
                event("audioArmed");
            }
            let pending = r.progress.pending.clone().unwrap();
            if pending.staged.is_none() {
                if dir.optional_kind(name)?.is_some() {
                    event("beforeAudioRename");
                    active(lease, check)?;
                    verify_unlocked(storage, &r, &cancelled)?;
                    inspect(&dir, name, &target, &pending.before, &cancelled)?;
                    check_unlocked(storage, &r, &cancelled)?;
                    active(lease, check)?;
                    dir.rename_exclusive(name, &reserved)?;
                    event("audioRenamed");
                } else if dir.optional_kind(&reserved)?.is_none() {
                    return Err("未確定音声が消失しています".into());
                }
                let staged = Proof::read(
                    &crate::native_fs::metadata(&dir.regular(&reserved)?)
                        .map_err(|e| e.to_string())?,
                )?;
                if !staged.matches_after_rename(&pending.before) {
                    return Err("移動した音声実体が一致しません。削除せず保持します".into());
                }
                inspect(&dir, &reserved, &target, &staged, &cancelled)?;
                let mut next = r.progress.clone();
                next.pending.as_mut().unwrap().staged = Some(staged);
                save_unlocked(storage, &mut r, next, lease, check)?;
                event("audioStaged");
            }
            event("beforeAudioUnlink");
            active(lease, check)?;
            verify_unlocked(storage, &r, &cancelled)?;
            let staged = r
                .progress
                .pending
                .as_ref()
                .unwrap()
                .staged
                .as_ref()
                .unwrap();
            if dir.optional_kind(&reserved)?.is_some() {
                inspect(&dir, &reserved, &target, staged, &cancelled)?;
                check_unlocked(storage, &r, &cancelled)?;
                active(lease, check)?;
                dir.unlink_staged(&reserved, false, staged)?;
            } else {
                dir.sync()?;
            }
            event("audioUnlinked");
        }
        // Detect callback/external changes before accepting advancement.
        verify_unlocked(storage, &r, &cancelled)?;
        let mut next = r.progress.clone();
        next.next += 1;
        next.pending = None;
        save_unlocked(storage, &mut r, next, lease, check)?;
        crate::operation_observation::advance(1);
        event("audioAdvanced");
    }
    event("beforeAudioFilesRemoved");
    active(lease, check)?;
    verify_unlocked(storage, &r, &cancelled)?;
    let mut next = r.progress.clone();
    next.phase = Phase::AudioFilesRemoved;
    save_unlocked(storage, &mut r, next, lease, check)?;
    event("audioFilesRemoved");
    verify_unlocked(storage, &r, &cancelled)?;
    Ok(r)
}

#[cfg(all(test, unix))]
#[path = "audio_removal_tests.rs"]
mod tests;
