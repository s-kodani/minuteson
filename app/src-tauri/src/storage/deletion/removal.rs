//! Fixed, explicitly reviewed file removal with a bounded durable cursor.
//! Reconfirmation appends evidence without changing the original intent.
use super::{
    database::Scope,
    files::{Census, Entry, Manifest},
    fs::{Directory, Proof},
    intent::{self, Prepared},
};
use crate::storage::{now_ms, Storage};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("ファイル削除を停止しました。既に消したファイルは復元しません".into())
    } else {
        Ok(())
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn manifest_hash(manifest: &Manifest) -> Result<String, String> {
    Ok(hash(
        &serde_json::to_vec(manifest).map_err(|e| e.to_string())?,
    ))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Policy {
    RegisteredOnly,
    ExplicitUnregistered,
}
// Neither a JSON manifest nor a producer hint can construct this review.
pub(super) struct Reviewed {
    manifest: Manifest,
    scope: Scope,
    policy: Policy,
}
impl Reviewed {
    pub(super) fn confirm(census: &Census, include_unregistered: bool) -> Result<Self, String> {
        let policy = if include_unregistered {
            Policy::ExplicitUnregistered
        } else {
            Policy::RegisteredOnly
        };
        check_policy(census.manifest(), &policy)?;
        Ok(Self {
            manifest: census.manifest().clone(),
            scope: census.scope().clone(),
            policy,
        })
    }
    pub(super) fn confirm_saved(
        storage: &Storage,
        prepared: &Prepared,
        include_unregistered: bool,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        prepared.verify_saved(&db, check)?;
        prepared.verify_locked(storage, &db, check)?;
        let census = Census::capture_locked(
            storage,
            &db,
            &prepared.scope().meeting_id,
            prepared.operation(),
            check,
        )?;
        Self::confirm(&census, include_unregistered)
    }
}
fn check_policy(manifest: &Manifest, policy: &Policy) -> Result<(), String> {
    if *policy == Policy::RegisteredOnly
        && manifest
            .entries
            .iter()
            .any(|e| !e.directory && e.registered.is_none())
    {
        return Err("未登録ファイルの一覧と削除範囲を明示的に確認してください。由来候補だけでは削除しません".into());
    }
    Ok(())
}
fn plan_hash(manifest: &Manifest, policy: &Policy) -> Result<String, String> {
    check_policy(manifest, policy)?;
    let mut digest = Sha256::new();
    digest.update(b"mon-delete-file-plan-v1\0");
    digest.update(serde_json::to_vec(policy).map_err(|e| e.to_string())?);
    digest.update(manifest_hash(manifest)?.as_bytes());
    for entry in manifest.entries.iter().rev() {
        let bytes = serde_json::to_vec(entry).map_err(|e| e.to_string())?;
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Removing,
    Removed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Pending {
    before: Proof,
    staged: Option<Proof>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Progress {
    version: u32,
    file_phase: Phase,
    manifest_hash: String,
    plan_hash: String,
    policy: Policy,
    consent_at: i64,
    next: usize,
    pending: Option<Pending>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) confirmation: Option<super::renewal::Reference>,
}
impl Progress {
    pub(super) fn encode(&self) -> Result<String, String> {
        let s = serde_json::to_string(self).map_err(|e| e.to_string())?;
        if s.len() > 4096 {
            Err("削除進捗が安全上限を超えています".into())
        } else {
            Ok(s)
        }
    }
    pub(super) fn consent_at(&self) -> i64 {
        self.consent_at
    }
    pub(super) fn validate(&self, p: &Prepared, manifest: &Manifest) -> Result<(), String> {
        if !((self.version == 1 && self.confirmation.is_none() && manifest == p.manifest())
            || (self.version == 2 && self.confirmation.as_ref().is_some_and(|r| r.valid())))
            || self.manifest_hash != manifest_hash(manifest)?
            || self.plan_hash != plan_hash(manifest, &self.policy)?
            || self.consent_at < p.consent_at()
        {
            return Err("削除進捗と確認したファイル範囲が一致しません".into());
        }
        self.validate_cursor(manifest)
    }
    fn validate_cursor(&self, manifest: &Manifest) -> Result<(), String> {
        let entries = &manifest.entries;
        if self.next > entries.len()
            || (self.file_phase == Phase::Removed
                && (self.next != entries.len() || self.pending.is_some()))
        {
            return Err("削除進捗と確認したファイル範囲が一致しません".into());
        }
        if let Some(pending) = &self.pending {
            let target = entries
                .len()
                .checked_sub(self.next + 1)
                .and_then(|index| entries.get(index))
                .ok_or("削除カーソルが範囲外です")?;
            let original = target
                .proof
                .as_ref()
                .ok_or("欠落ファイルを移動対象へ採用しません")?;
            pending.before.validate(target.directory)?;
            if (target.directory && pending.before.identity != original.identity)
                || (!target.directory && &pending.before != original)
            {
                return Err("移動前実体が確認対象と一致しません".into());
            }
            if let Some(staged) = &pending.staged {
                staged.validate(target.directory)?;
                if !staged.matches_after_rename(&pending.before) {
                    return Err("移動後実体が確認対象と一致しません".into());
                }
            }
        }
        Ok(())
    }
    pub(super) fn renewed(
        p: &Prepared,
        manifest: &Manifest,
        confirmation: super::renewal::Reference,
        include_unregistered: bool,
        consent_at: i64,
    ) -> Result<Self, String> {
        let policy = if include_unregistered {
            Policy::ExplicitUnregistered
        } else {
            Policy::RegisteredOnly
        };
        let next = Self {
            version: 2,
            file_phase: Phase::Removing,
            manifest_hash: manifest_hash(manifest)?,
            plan_hash: plan_hash(manifest, &policy)?,
            policy,
            consent_at,
            next: 0,
            pending: None,
            confirmation: Some(confirmation),
        };
        next.validate(p, manifest)?;
        Ok(next)
    }
    fn reserved(&self, operation: &str, index: usize) -> String {
        if let Some(c) = &self.confirmation {
            format!(".mon-delete-{operation}-r{}-{index}", c.generation)
        } else {
            format!(".mon-delete-{operation}-{index}")
        }
    }
}
// A stored file-completion proof retains the exact policy, plan and cursor.
// Only the compound DB receipt may parse it, and must revalidate all fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Finished(Progress);
impl Finished {
    pub(super) fn manifest(
        &self,
        db: &Connection,
        confirmed: &Prepared,
        check: &impl Fn() -> bool,
    ) -> Result<Manifest, String> {
        let manifest = super::renewal::active_manifest(db, confirmed, &self.0, check)?;
        if self.0.file_phase != Phase::Removed {
            return Err("ファイル段階の完了を確認できません".into());
        }
        Ok(manifest)
    }
    pub(super) fn verify_remaining(
        &self,
        storage: &Storage,
        db: &Connection,
        confirmed: &Prepared,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let manifest = self.manifest(db, confirmed, check)?;
        verify_remaining(
            storage,
            db,
            &Receipt {
                confirmed: confirmed.clone(),
                progress: self.0.clone(),
                manifest,
            },
            check,
        )
    }
    pub(super) fn verify_absent(
        &self,
        storage: &Storage,
        db: &Connection,
        confirmed: &Prepared,
    ) -> Result<(), String> {
        let manifest = self.manifest(db, confirmed, &|| false)?;
        if storage.managed_anchor.identity()? != (manifest.root.clone(), manifest.database.clone())
        {
            return Err("削除確認時の管理領域・DBの実体が一致しません".into());
        }
        let Some(meetings) = storage.managed_anchor.directory_ref()?.child("meetings")? else {
            return Ok(());
        };
        if meetings.optional_kind(&manifest.meeting_id)?.is_some() {
            return Err("削除済み会議フォルダーが再出現しています。削除せず保持します".into());
        }
        if let Some(index) = manifest
            .entries
            .iter()
            .rev()
            .position(|e| e.path == format!("meetings/{}", manifest.meeting_id))
        {
            let reserved = self.0.reserved(confirmed.operation(), index);
            if meetings.optional_kind(&reserved)?.is_some() {
                return Err("会議フォルダーの削除予約対象が残っています".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct Receipt {
    confirmed: Prepared,
    pub(super) progress: Progress,
    pub(super) manifest: Manifest,
}
impl Receipt {
    pub(super) fn verify_live(
        &self,
        storage: &Storage,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        preflight(storage, &db, self, check)
    }
    pub(super) fn processed_paths(&self) -> usize {
        self.progress.next
    }
    pub(super) fn confirmed(&self) -> &Prepared {
        &self.confirmed
    }
    pub(super) fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub(super) fn review_census(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Census, String> {
        let p = &self.confirmed;
        let other: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE id!=?1 AND state IN ('prepared','installing','cleanup','attention'))",params![p.operation()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if other {
            return Err("別の未完了保守操作があります。再確認を進めません".into());
        }
        // A reserved empty meeting root is outside this meeting's logical tree.
        // Never forget it or include arbitrary sibling meetings in a new plan.
        if self.progress.pending.is_some()
            && self
                .target()
                .is_some_and(|e| e.path == format!("meetings/{}", self.manifest.meeting_id))
        {
            if let Some(meetings) = storage.managed_anchor.directory_ref()?.child("meetings")? {
                if meetings.optional_kind(&self.reserved())?.is_some() {
                    return Err(
                        "移動済み会議ルートの解決が必要です。元の予約対象を保持します".into(),
                    );
                }
            }
        }
        let census =
            Census::capture_locked(storage, db, &p.scope().meeting_id, p.operation(), check)?;
        if census.scope() != p.scope()
            || census.manifest().root != p.manifest().root
            || census.manifest().database != p.manifest().database
        {
            return Err("元のDB範囲・起動時実体を再確認で変更できません".into());
        }
        Ok(census)
    }
    pub(super) fn verify_reservation_names(
        &self,
        manifest: &Manifest,
        progress: &Progress,
    ) -> Result<(), String> {
        let observed: BTreeSet<_> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        for (index, target) in manifest.entries.iter().rev().enumerate() {
            let parent = target.path.rsplit_once('/').ok_or("対象パスが不正です")?.0;
            let reserved = format!(
                "{parent}/{}",
                progress.reserved(self.confirmed.operation(), index)
            );
            if observed.contains(reserved.as_str()) {
                return Err("再確認の削除予約名が既に存在します".into());
            }
        }
        Ok(())
    }
    pub(super) fn matches_completion(
        &self,
        confirmed: &Prepared,
        finished: &Finished,
    ) -> Result<(), String> {
        self.confirmed.matches_saved(confirmed)?;
        if !self.removed() || self.progress != finished.0 {
            return Err("DB段階と元のファイル完了記録が一致しません".into());
        }
        Ok(())
    }
    pub(super) fn progress_json(&self) -> Result<String, String> {
        self.progress.encode()
    }
    pub(super) fn verify_finished(
        &self,
        storage: &Storage,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<Finished, String> {
        if !self.removed() {
            return Err("ファイル削除が未完了です。DB行を削除しません".into());
        }
        preflight(storage, db, self, check)?;
        Ok(Finished(self.progress.clone()))
    }
    fn load(db: &Connection, operation: &str, check: &impl Fn() -> bool) -> Result<Self, String> {
        let confirmed = intent::read_removing(db, operation, check)?;
        let json: String = db
            .query_row(
                "SELECT progress_json FROM maintenance_operations WHERE id=?1",
                params![operation],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let progress: Progress = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let manifest = super::renewal::active_manifest(db, &confirmed, &progress, check)?;
        if progress.encode()? != json {
            return Err("削除進捗が標準形式と一致しません".into());
        }
        Ok(Self {
            confirmed,
            progress,
            manifest,
        })
    }
    pub(super) fn verify_saved(
        &self,
        db: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let current = Self::load(db, self.confirmed.operation(), check)?;
        self.confirmed.matches_saved(&current.confirmed)?;
        if self.progress != current.progress || self.manifest != current.manifest {
            return Err("削除進捗が変更されています。保存済み状態を再確認してください".into());
        }
        Ok(())
    }
    pub(super) fn removed(&self) -> bool {
        self.progress.file_phase == Phase::Removed
    }
    fn reserved(&self) -> String {
        self.progress
            .reserved(self.confirmed.operation(), self.progress.next)
    }
    fn target(&self) -> Option<&Entry> {
        let entries = &self.manifest.entries;
        entries
            .len()
            .checked_sub(self.progress.next + 1)
            .and_then(|index| entries.get(index))
    }
    fn save(
        &mut self,
        storage: &Storage,
        db: &Connection,
        next: Progress,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        cancel(check)?;
        storage.managed_anchor.verify()?;
        // Immutable path observations were fully validated before acquiring
        // the writer. This entire run owns Storage's mutex and SQL guards.
        self.confirmed
            .verify_metadata(db, &self.progress.encode()?)?;
        if next.version != self.progress.version
            || next.manifest_hash != self.progress.manifest_hash
            || next.plan_hash != self.progress.plan_hash
            || next.policy != self.progress.policy
            || next.consent_at != self.progress.consent_at
            || next.confirmation != self.progress.confirmation
        {
            return Err("削除進捗の固定条件は変更できません".into());
        }
        next.validate_cursor(&self.manifest)?;
        let transition = if next.next == self.progress.next + 1 {
            next.pending.is_none()
                && next.file_phase == Phase::Removing
                && (self.target().is_some_and(|e| e.proof.is_none())
                    || self
                        .progress
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.staged.is_some()))
        } else if next.next == self.progress.next && next.file_phase == Phase::Removed {
            self.progress.file_phase == Phase::Removing
                && next.next == self.manifest.entries.len()
                && self.progress.pending.is_none()
                && next.pending.is_none()
        } else if next.next == self.progress.next && next.file_phase == Phase::Removing {
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
        if !transition {
            return Err("削除進捗の遷移が不正です".into());
        }
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let old = self.progress.encode()?;
        if tx.execute("UPDATE maintenance_operations SET progress_json=?2,error_code=NULL,updated_at=max(updated_at,?3) WHERE id=?1 AND kind='delete_meeting' AND state='installing' AND progress_json=?4",params![self.confirmed.operation(),next.encode()?,now_ms()?,old]).map_err(|e|e.to_string())?!=1 { return Err("削除進捗の確定対象が変更されています".into()); }
        cancel(check)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.progress = next;
        Ok(())
    }
}
pub(super) fn read(
    storage: &Storage,
    operation: &str,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    storage.managed_anchor.verify()?;
    Receipt::load(&db, operation, check)
}
pub(super) fn start(
    storage: &Storage,
    confirmed: &Prepared,
    reviewed: &Reviewed,
    check: &impl Fn() -> bool,
) -> Result<Receipt, String> {
    cancel(check)?;
    if confirmed.manifest() != &reviewed.manifest || confirmed.scope() != &reviewed.scope {
        return Err("削除可能範囲の確認対象が一致しません".into());
    }
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    confirmed.verify_saved(&db, check)?;
    confirmed.verify_locked(storage, &db, check)?;
    let progress = Progress {
        version: 1,
        file_phase: Phase::Removing,
        manifest_hash: manifest_hash(&reviewed.manifest)?,
        plan_hash: plan_hash(&reviewed.manifest, &reviewed.policy)?,
        policy: reviewed.policy.clone(),
        consent_at: now_ms()?.max(confirmed.consent_at()),
        next: 0,
        pending: None,
        confirmation: None,
    };
    progress.validate(confirmed, confirmed.manifest())?;
    // Reserve names must be absent even when the user explicitly included
    // unregistered objects. Never overwrite an already observed object.
    let observed: BTreeSet<_> = reviewed
        .manifest
        .entries
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    for (index, target) in reviewed.manifest.entries.iter().rev().enumerate() {
        let parent = target.path.rsplit_once('/').ok_or("対象パスが不正です")?.0;
        let reserved = format!("{parent}/.mon-delete-{}-{index}", confirmed.operation());
        if observed.contains(reserved.as_str()) {
            return Err("削除用予約名が既に存在します".into());
        }
    }
    let _writer = super::fence::write_prepared(storage, &db, confirmed, check)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    confirmed.verify_saved(&tx, check)?;
    cancel(check)?;
    if tx.execute("UPDATE maintenance_operations SET state='installing',progress_json=?2,updated_at=max(updated_at,?3) WHERE id=?1 AND state='prepared' AND progress_json='{\"filePhase\":\"notStarted\"}'",params![confirmed.operation(),progress.encode()?,now_ms()?]).map_err(|e|e.to_string())?!=1 { return Err("削除開始の確認状態が変更されています".into()); }
    cancel(check)?;
    tx.commit().map_err(|e| e.to_string())?;
    Receipt::load(&db, confirmed.operation(), check)
}

fn parent(storage: &Storage, manifest: &Manifest, path: &str) -> Result<Option<Directory>, String> {
    let (parent, _) = path.rsplit_once('/').ok_or("管理パスが不正です")?;
    let root = storage.managed_anchor.directory_ref()?;
    let mut held: Option<Directory> = None;
    let mut relative = String::new();
    for component in parent.split('/') {
        let current = held.as_ref().unwrap_or(root);
        let Some(child) = current.child(component)? else {
            return Ok(None);
        };
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(component);
        if let Ok(index) = manifest.entries.binary_search_by(|e| e.path.cmp(&relative)) {
            let expected = &manifest.entries[index];
            if !expected.directory {
                return Err("確認した親はフォルダーではありません".into());
            }
            child.verify_identity(
                &expected
                    .proof
                    .as_ref()
                    .ok_or("親フォルダー実体がありません")?
                    .identity,
            )?;
        }
        held = Some(child);
    }
    Ok(held)
}
fn physical(dir: &Directory, leaf: &str) -> Result<bool, String> {
    Ok(dir.optional_kind(leaf)?.is_some())
}
fn inspect(
    dir: &Directory,
    leaf: &str,
    target: &Entry,
    expected: &Proof,
    check: &impl Fn() -> bool,
) -> Result<Proof, String> {
    cancel(check)?;
    if target.directory {
        let child = dir.child(leaf)?.ok_or("削除対象フォルダーがありません")?;
        child.verify_identity(&expected.identity)?;
        if !child.names(super::files::MAX_ENTRIES)?.is_empty() {
            return Err("削除対象フォルダーに未削除・未知の内容があります".into());
        }
        let proof = child.proof()?;
        if proof != *expected {
            return Err("削除対象フォルダーが検査中に変更されています".into());
        }
        Ok(proof)
    } else {
        let mut file = dir.regular(leaf)?;
        let before = Proof::read(&file.metadata().map_err(|e| e.to_string())?)?;
        if before != *expected {
            return Err("削除対象の実体・内容条件が変更されています".into());
        }
        let mut digest = Sha256::new();
        let mut length = 0u64;
        let mut buffer = vec![0u8; 128 * 1024];
        loop {
            cancel(check)?;
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            length = length.checked_add(n as u64).ok_or("容量が範囲外です")?;
            if length > before.size {
                return Err("削除検査中にファイルが拡張されました".into());
            }
            digest.update(&buffer[..n]);
        }
        dir.verify_file(leaf, &file)?;
        if length != before.size
            || Proof::read(&file.metadata().map_err(|e| e.to_string())?)? != before
            || target.sha256.as_deref() != Some(format!("{:x}", digest.finalize()).as_str())
        {
            return Err("削除対象の内容SHA・実体が一致しません".into());
        }
        Ok(before)
    }
}
fn observed_proof(dir: &Directory, leaf: &str, directory: bool) -> Result<Proof, String> {
    if directory {
        dir.child(leaf)?.ok_or("移動対象がありません")?.proof()
    } else {
        Proof::read(&dir.regular(leaf)?.metadata().map_err(|e| e.to_string())?)
    }
}
fn missing(entry: &Entry) -> Option<Entry> {
    entry.registered.as_ref().map(|_| {
        let mut e = entry.clone();
        e.proof = None;
        e.sha256 = None;
        e.stage_hint = None;
        e
    })
}
fn preflight(
    storage: &Storage,
    db: &Connection,
    receipt: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    receipt.verify_saved(db, check)?;
    verify_remaining(storage, db, receipt, check)
}
fn verify_remaining(
    storage: &Storage,
    db: &Connection,
    receipt: &Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    let p = &receipt.confirmed;
    let other_pending: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM maintenance_operations WHERE id!=?1 AND state IN ('prepared','installing','cleanup','attention'))",
            params![p.operation()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if other_pending {
        return Err("別の未完了保守操作があります。ファイル削除を再開しません".into());
    }
    let reserved = receipt
        .target()
        .filter(|_| receipt.progress.pending.is_some())
        .map(|e| {
            format!(
                "{}/{}",
                e.path.rsplit_once('/').unwrap().0,
                receipt.reserved()
            )
        });
    let current = Census::capture_removal_locked(
        storage,
        db,
        &receipt.manifest.meeting_id,
        p.operation(),
        check,
        reserved.as_deref(),
    )?;
    if current.scope() != p.scope()
        || current.manifest().root != receipt.manifest.root
        || current.manifest().database != receipt.manifest.database
    {
        return Err("削除中にDB範囲または起動時実体が変更されています".into());
    }
    let mut actual: BTreeMap<_, _> = current
        .manifest()
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.clone()))
        .collect();
    // A reserved meeting root is outside the original meeting subtree. Read
    // just that exact empty sibling; never inventory another meeting.
    if let Some(target) = receipt.target().filter(|e| {
        e.path == format!("meetings/{}", p.manifest().meeting_id)
            && receipt.progress.pending.is_some()
    }) {
        let dir =
            parent(storage, &receipt.manifest, &target.path)?.ok_or("確認した親がありません")?;
        let leaf = receipt.reserved();
        if physical(&dir, &leaf)? {
            let child = dir.child(&leaf)?.ok_or("予約フォルダーがありません")?;
            if !child.names(super::files::MAX_ENTRIES)?.is_empty() {
                return Err("予約フォルダーに未知の内容があります".into());
            }
            let mut entry = target.clone();
            entry.path = reserved.clone().unwrap();
            entry.proof = Some(child.proof()?);
            entry.registered = None;
            entry.stage_hint = None;
            actual.insert(entry.path.clone(), entry);
        }
    }
    let mut expected = BTreeMap::new();
    for (index, entry) in receipt.manifest.entries.iter().rev().enumerate() {
        let candidate = if index < receipt.progress.next {
            missing(entry)
        } else if index == receipt.progress.next && receipt.progress.pending.is_some() {
            let pending = receipt.progress.pending.as_ref().unwrap();
            let (parent, _) = entry.path.rsplit_once('/').ok_or("対象パスが不正です")?;
            let reserved = format!("{parent}/{}", receipt.reserved());
            let original_exists = actual.get(&entry.path).is_some_and(|e| e.proof.is_some());
            if let Some(staged) = actual.get(&reserved) {
                if original_exists {
                    return Err("削除予約名と元のパスが同時に存在します".into());
                }
                let proof = staged.proof.as_ref().ok_or("予約対象の実体がありません")?;
                let valid = if let Some(saved) = &pending.staged {
                    proof == saved
                } else {
                    proof.matches_after_rename(&pending.before)
                };
                if !valid {
                    return Err("削除予約対象が差し替えられています".into());
                }
                let mut expected_staged = entry.clone();
                expected_staged.path = reserved.clone();
                expected_staged.proof = Some(proof.clone());
                expected_staged.registered = None;
                expected_staged.stage_hint = None;
                expected.insert(reserved, expected_staged);
                missing(entry)
            } else if original_exists && pending.staged.is_none() {
                let mut original = entry.clone();
                original.proof = Some(pending.before.clone());
                Some(original)
            } else if !original_exists && pending.staged.is_some() {
                missing(entry)
            } else {
                return Err("未確定の移動対象が消失しています。自動採用しません".into());
            }
        } else {
            Some(entry.clone())
        };
        if let Some(mut candidate) = candidate {
            if candidate.directory
                && !(index == receipt.progress.next && receipt.progress.pending.is_some())
            {
                let proof = actual
                    .get(&candidate.path)
                    .and_then(|e| e.proof.as_ref())
                    .ok_or("確認したフォルダーが消失しています")?;
                if proof.identity
                    != candidate
                        .proof
                        .as_ref()
                        .ok_or("確認実体がありません")?
                        .identity
                {
                    return Err("確認したフォルダーが差し替えられています".into());
                }
                candidate.proof = Some(proof.clone());
            }
            expected.insert(candidate.path.clone(), candidate);
        }
    }
    if actual != expected {
        return Err(
            "削除対象の追加・再出現・消失・変更があります。残りの範囲を再確認してください".into(),
        );
    }
    Ok(())
}
pub(super) fn run(
    storage: &Storage,
    receipt: &mut Receipt,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    run_inner(storage, receipt, check, &|_| {})
}
fn run_inner(
    storage: &Storage,
    receipt: &mut Receipt,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    cancel(check)?;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let _writer = super::fence::write_removal(storage, &db, receipt, check)?;
    preflight(storage, &db, receipt, check)?;
    if receipt.removed() {
        return Ok(());
    }
    while let Some(target) = receipt.target().cloned() {
        cancel(check)?;
        storage.managed_anchor.verify()?;
        let leaf = target.path.rsplit_once('/').ok_or("対象パスが不正です")?.1;
        let dir = parent(storage, &receipt.manifest, &target.path)?;
        if target.proof.is_none() {
            if dir
                .as_ref()
                .map(|d| physical(d, leaf))
                .transpose()?
                .unwrap_or(false)
            {
                return Err("確認時に欠落していたパスが再出現しました".into());
            }
        } else {
            let dir = dir.ok_or("確認した親フォルダーが消失しました")?;
            let reserved = receipt.reserved();
            if receipt.progress.pending.is_none() {
                if physical(&dir, &reserved)? {
                    return Err("削除用予約名が既に存在します".into());
                }
                let before = if target.directory {
                    let proof = observed_proof(&dir, leaf, true)?;
                    if proof.identity != target.proof.as_ref().unwrap().identity {
                        return Err("フォルダーが差し替えられました".into());
                    }
                    proof
                } else {
                    target.proof.clone().unwrap()
                };
                inspect(&dir, leaf, &target, &before, check)?;
                let mut next = receipt.progress.clone();
                next.pending = Some(Pending {
                    before,
                    staged: None,
                });
                receipt.save(storage, &db, next, check)?;
                event("armed");
            }
            let pending = receipt.progress.pending.clone().unwrap();
            let original_exists = physical(&dir, leaf)?;
            let staged_exists = physical(&dir, &reserved)?;
            if original_exists && staged_exists {
                return Err("予約名と元のパスが同時に存在します".into());
            }
            if pending.staged.is_none() {
                if original_exists {
                    inspect(&dir, leaf, &target, &pending.before, check)?;
                    cancel(check)?;
                    event("beforeRename");
                    storage.managed_anchor.verify()?;
                    dir.rename_exclusive(leaf, &reserved)?;
                    event("renamed");
                } else if !staged_exists {
                    return Err("未確定の移動対象が消失しました".into());
                }
                let staged = observed_proof(&dir, &reserved, target.directory)?;
                if !staged.matches_after_rename(&pending.before) {
                    // Restore a raced name only without replacing anything.
                    // If restoration is impossible, retain it for inspection.
                    let _ = dir.rename_exclusive(&reserved, leaf);
                    return Err("移動した対象が確認実体と一致しません。削除せず保持します".into());
                }
                inspect(&dir, &reserved, &target, &staged, check)?;
                let mut next = receipt.progress.clone();
                next.pending.as_mut().unwrap().staged = Some(staged);
                receipt.save(storage, &db, next, check)?;
                event("staged");
            } else if original_exists {
                return Err("移動済み対象の元パスが再出現しました".into());
            }
            let staged = receipt
                .progress
                .pending
                .as_ref()
                .unwrap()
                .staged
                .as_ref()
                .unwrap();
            event("beforeUnlink");
            cancel(check)?;
            storage.managed_anchor.verify()?;
            if physical(&dir, &reserved)? {
                inspect(&dir, &reserved, &target, staged, check)?;
                dir.unlink_staged(&reserved, target.directory, staged)?;
            } else {
                dir.sync()?;
            }
            event("unlinked");
        }
        let mut next = receipt.progress.clone();
        next.next += 1;
        next.pending = None;
        receipt.save(storage, &db, next, check)?;
        event("advanced");
    }
    preflight(storage, &db, receipt, check)?;
    cancel(check)?;
    let mut next = receipt.progress.clone();
    next.file_phase = Phase::Removed;
    receipt.save(storage, &db, next, check)?;
    event("filesRemoved");
    Ok(())
}
#[cfg(test)]
pub(super) fn run_observed(
    storage: &Storage,
    receipt: &mut Receipt,
    check: &impl Fn() -> bool,
    event: &impl Fn(&str),
) -> Result<(), String> {
    run_inner(storage, receipt, check, event)
}
#[cfg(test)]
#[path = "removal_tests.rs"]
mod tests;
