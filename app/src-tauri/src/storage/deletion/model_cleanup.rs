//! Automatic retirement of redundant managed packages. Unlike user deletion,
//! this journal never fences unrelated meeting writes. Every unlink still uses
//! the shared descriptor-relative, proof-checked deletion capability.
use super::{Directory, Entry, Observation, Proof, Snapshot};
use crate::{
    analysis_worker::data_maintenance::DataMaintenanceLease, model_placement::CatalogEntry,
    storage::Storage,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::Path,
};

const PREFIX: &str = "model_cleanup_v1/";
const MAX_JOURNAL: usize = 1_048_576;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    before: Proof,
    staged: Option<Proof>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    models_root: String,
    source: Snapshot,
    keeper: Snapshot,
    next: usize,
    pending: Option<Pending>,
}
impl Journal {
    fn complete(&self) -> bool {
        self.next == self.source.observations.len()
    }
    fn validate(&self) -> Result<(), String> {
        self.source.validate()?;
        self.keeper.validate()?;
        if self.version != 1
            || self.source.directory_scope
            || self.keeper.directory_scope
            || !Path::new(&self.models_root).is_absolute()
            || Path::new(&self.models_root).file_name() != Some(std::ffi::OsStr::new("models"))
            || self.source.record.package_id == self.keeper.record.package_id
            || record_identity(&self.source) != record_identity(&self.keeper)
            || record_identity(&self.source).is_none()
            || !self.source.placed
            || !self.keeper.placed
            || self.source.storage_root != self.keeper.storage_root
            || self.source.database != self.keeper.database
            || self.source.models_root != self.keeper.models_root
            || self.source.active_jobs != 0
            || self
                .source
                .observations
                .iter()
                .chain(&self.keeper.observations)
                .any(|o| {
                    !o.entry.registered || o.entry.changed || !o.entry.present || o.proof.is_none()
                })
            || self.next > self.source.observations.len()
            || self.complete() && self.pending.is_some()
        {
            return Err("モデル自動整理の保存対象が不正です。対象を保持します".into());
        }
        if let Some(p) = &self.pending {
            let o = self
                .source
                .observations
                .iter()
                .rev()
                .nth(self.next)
                .ok_or("整理位置が不正です")?;
            p.before.validate(o.entry.directory)?;
            if p.before.identity != o.proof.as_ref().ok_or("整理実体がありません")?.identity
                || !o.entry.directory && Some(&p.before) != o.proof.as_ref()
                || p.staged
                    .as_ref()
                    .is_some_and(|s| !s.matches_after_rename(&p.before))
            {
                return Err("モデル自動整理の移動記録が不正です".into());
            }
        }
        Ok(())
    }
    fn reserved(&self, index: usize) -> String {
        format!(
            ".mon-delete-cleanup-{}-{index}",
            self.source.record.package_id
        )
    }
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn read(db: &Connection, package: &str) -> Result<Option<Journal>, String> {
    let row: Option<(Option<String>, String)> = db.query_row(
        "SELECT CASE WHEN length(CAST(value_json AS BLOB))<=1048576 THEN value_json END,content_hash FROM app_settings WHERE key=?1",
        [format!("{PREFIX}{package}")], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional().map_err(|e| e.to_string())?;
    let Some((json, hash)) = row else {
        return Ok(None);
    };
    let json = json.ok_or("モデル整理記録の容量が不正です")?;
    if digest(&json) != hash {
        return Err("モデル整理記録のSHAが不正です".into());
    }
    let journal: Journal = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    journal.validate()?;
    if journal.source.record.package_id != package {
        return Err("モデル整理記録の対象が一致しません".into());
    }
    Ok(Some(journal))
}
pub(in crate::storage) fn unavailable(db: &Connection, package: &str) -> Result<bool, String> {
    Ok(read(db, package)?.is_some())
}
pub(in crate::storage) fn pending_keepers(db: &Connection) -> Result<BTreeSet<String>, String> {
    let mut keepers = BTreeSet::new();
    for package in journal_ids(db)? {
        let j = read(db, &package)?.ok_or("モデル整理記録がありません")?;
        if !j.complete() {
            keepers.insert(j.keeper.record.package_id);
        }
    }
    Ok(keepers)
}
fn journal_ids(db: &Connection) -> Result<Vec<String>, String> {
    let mut q = db.prepare("SELECT substr(key,18) FROM app_settings WHERE key GLOB 'model_cleanup_v1/*' ORDER BY key LIMIT 10001").map_err(|e| e.to_string())?;
    let ids = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if ids.len() > 10_000 {
        return Err("モデル整理履歴が上限を超えています".into());
    }
    Ok(ids)
}
fn save(
    storage: &Storage,
    journal: &Journal,
    previous: Option<&Journal>,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    active(storage, lease, check)?;
    journal.validate()?;
    let json = serde_json::to_string(journal).map_err(|e| e.to_string())?;
    if json.len() > MAX_JOURNAL {
        return Err("モデル整理記録が上限を超えています".into());
    }
    let db = crate::operation_observation::lock(&storage.connection)?;
    let old = read(&db, &journal.source.record.package_id)?;
    if old
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| e.to_string())?
        != previous
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| e.to_string())?
    {
        return Err("モデル整理の保存状態が変更されています".into());
    }
    scope(storage, &db, journal, check)?;
    let key = format!("{PREFIX}{}", journal.source.record.package_id);
    db.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,1,?4) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=app_settings.lock_version+1,updated_at=excluded.updated_at",
        params![key, json, digest(&json), crate::storage::now_ms()?]).map_err(|e| e.to_string())?;
    Ok(())
}
fn active(
    storage: &Storage,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    lease.validate_model_deletion()?;
    storage.require_global_data_writable()?;
    super::cancel(check)
}
fn protected(db: &Connection, package: &str) -> Result<bool, String> {
    if super::super::super::model_profiles::current_packages(db)?.contains(package) {
        return Ok(true);
    }
    db.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('queued','running','pausing','paused','interrupted') AND json_extract(profile_snapshot_json,'$.managedModel.package.packageId')=?1)",
        [package], |r| r.get(0)).map_err(|e| e.to_string())
}
fn scope(
    storage: &Storage,
    db: &Connection,
    j: &Journal,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    super::cancel(check)?;
    if storage.managed_anchor.identity()?
        != (j.source.storage_root.clone(), j.source.database.clone())
        || super::super::super::model_placements::load_history(db, &j.source.record.package_id)?
            != Some((j.source.record.clone(), true))
        || super::super::super::model_placements::load_history(db, &j.keeper.record.package_id)?
            != Some((j.keeper.record.clone(), true))
        || protected(db, &j.source.record.package_id)?
        || unavailable(db, &j.keeper.record.package_id)?
        || super::intent::removal::deleted(db, &j.keeper.record.package_id)?
    {
        return Err("モデル整理の配置・参照・保存先が変わりました。対象を保持します".into());
    }
    // Keep all verified keeper bytes pinned on Windows; elsewhere check its
    // unchanged object metadata here, with full hashes on every new attempt.
    let models = Directory::root(Path::new(&j.models_root))?;
    models.verify_identity(&j.keeper.models_root)?;
    let keeper = models
        .child(&j.keeper.record.package_id)?
        .ok_or("保持するモデルがありません")?;
    let expected = j
        .keeper
        .observations
        .iter()
        .map(|o| (o.entry.relative_path.as_str(), o))
        .collect();
    super::verify_metadata_tree(&keeper, "", &expected, check)
}
fn identity(
    engine: &str,
    backend: Option<&str>,
    files: impl Iterator<Item = (String, u64, String)>,
) -> Option<String> {
    if engine == "asr" && backend.is_none() {
        return None;
    }
    let mut files: Vec<_> = files
        .filter(|(role, _, _)| {
            matches!(
                role.as_str(),
                "llm" | "asr" | "asr_asset" | "vad" | "segmentation" | "speaker_embedding"
            ) && !(engine == "whisper.cpp" && role == "vad")
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    files.sort();
    serde_json::to_string(&(engine, backend, files)).ok()
}
fn record_identity(s: &Snapshot) -> Option<String> {
    let engine = serde_json::to_value(s.record.manifest.engine).ok()?;
    identity(
        engine.as_str()?,
        s.record.manifest.backend.as_ref().map(|b| b.id.as_str()),
        s.record.files.iter().map(|f| {
            (
                serde_json::to_value(f.role)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .into(),
                f.actual_size,
                f.actual_hash.clone(),
            )
        }),
    )
}
fn entry_identity(e: &CatalogEntry) -> Option<String> {
    identity(
        &e.engine,
        e.backend_id.as_deref(),
        e.model_files.iter().map(|f| {
            (
                serde_json::to_value(f.role)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .into(),
                f.size,
                f.sha256.clone(),
            )
        }),
    )
}
fn keeper<'a>(group: &'a [CatalogEntry], selected: &BTreeSet<String>) -> Option<&'a CatalogEntry> {
    group
        .iter()
        .find(|e| selected.contains(&e.package_id))
        .or_else(|| {
            group.iter().find(|e| {
                e.compatibility
                    .as_ref()
                    .is_some_and(|c| c.state == crate::model_compatibility::CheckState::Succeeded)
            })
        })
}
fn full_path(package: &str, relative: &str) -> String {
    if relative.is_empty() {
        package.into()
    } else {
        format!("{package}/{relative}")
    }
}
fn parent(models: &Directory, j: &Journal, relative: &str) -> Result<Option<Directory>, String> {
    if relative.is_empty() {
        return Ok(None);
    }
    let package = models
        .child(&j.source.record.package_id)?
        .ok_or("整理するモデルがありません")?;
    package.verify_identity(
        &j.source.observations[0]
            .proof
            .as_ref()
            .ok_or("モデル実体がありません")?
            .identity,
    )?;
    let mut held = package;
    if let Some((parents, _)) = relative.rsplit_once('/') {
        let mut path = String::new();
        for component in parents.split('/') {
            path = if path.is_empty() {
                component.into()
            } else {
                format!("{path}/{component}")
            };
            let expected = j
                .source
                .observations
                .iter()
                .find(|o| o.entry.relative_path == path)
                .and_then(|o| o.proof.as_ref())
                .ok_or("整理対象の親記録がありません")?;
            let next = held.child(component)?.ok_or("整理対象の親がありません")?;
            next.verify_identity(&expected.identity)?;
            held = next;
        }
    }
    Ok(Some(held))
}
fn observed(
    dir: &Directory,
    name: &str,
    entry: &Entry,
    check: &impl Fn() -> bool,
) -> Result<Proof, String> {
    if entry.directory {
        let child = dir.child(name)?.ok_or("整理するフォルダーがありません")?;
        if !child.names(super::MAX_ENTRIES)?.is_empty() {
            return Err("整理対象に未登録の内容があります".into());
        }
        child.proof()
    } else {
        let (proof, hash) = super::hash_file(dir, name, check)?;
        if entry.sha256.as_deref() != Some(&hash) {
            return Err("整理対象のSHAが変わっています".into());
        }
        Ok(proof)
    }
}
fn verify_remaining(
    storage: &Storage,
    j: &Journal,
    check: &impl Fn() -> bool,
) -> Result<Directory, String> {
    {
        let db = crate::operation_observation::lock(&storage.connection)?;
        scope(storage, &db, j, check)?;
    }
    let models = Directory::root(Path::new(&j.models_root))?;
    models.verify_identity(&j.source.models_root)?;
    let mut actual: BTreeMap<String, Observation> =
        super::present_inventory(&models, &j.source.record, check)?
            .into_iter()
            .map(|o| {
                (
                    full_path(&j.source.record.package_id, &o.entry.relative_path),
                    o,
                )
            })
            .collect();
    let reserved_root = j.reserved(j.source.observations.len() - 1);
    if let Some(dir) = models.child(&reserved_root)? {
        if !dir.names(super::MAX_ENTRIES)?.is_empty() {
            return Err("整理予約フォルダーに未知の内容があります".into());
        }
        let mut o = j.source.observations[0].clone();
        o.proof = Some(dir.proof()?);
        actual.insert(reserved_root, o);
    }
    let mut wanted = BTreeSet::new();
    for (i, o) in j.source.observations.iter().rev().enumerate().skip(j.next) {
        let original = full_path(&j.source.record.package_id, &o.entry.relative_path);
        let reserved = original
            .rsplit_once('/')
            .map_or_else(|| j.reserved(i), |(p, _)| format!("{p}/{}", j.reserved(i)));
        let (path, expected) = if i == j.next {
            if let Some(p) = &j.pending {
                if actual.contains_key(&reserved) {
                    if actual.contains_key(&original) {
                        return Err("整理済みの元パスが再出現しました".into());
                    }
                    (reserved, p.staged.as_ref().unwrap_or(&p.before))
                } else if p.staged.is_some() {
                    if actual.contains_key(&original) {
                        return Err("整理対象が再出現しました".into());
                    }
                    continue;
                } else {
                    (original, &p.before)
                }
            } else {
                (original, o.proof.as_ref().unwrap())
            }
        } else {
            (original, o.proof.as_ref().unwrap())
        };
        let a = actual.get(&path).ok_or("整理対象が消失しました")?;
        let proof = a.proof.as_ref().ok_or("整理対象の実体がありません")?;
        let same = if o.entry.directory {
            proof.identity == expected.identity
        } else if i == j.next
            && j.pending.as_ref().is_some_and(|p| p.staged.is_none())
            && path.ends_with(&j.reserved(i))
        {
            proof.matches_after_rename(expected)
        } else {
            proof == expected
        };
        if a.entry.directory != o.entry.directory
            || !same
            || !o.entry.directory && a.entry.sha256 != o.entry.sha256
        {
            return Err("整理対象の内容・実体が変更されています".into());
        }
        wanted.insert(path);
    }
    if actual.keys().cloned().collect::<BTreeSet<_>>() != wanted {
        return Err("整理対象に未知・再出現したファイルがあります".into());
    }
    models.verify()?;
    Ok(models)
}
fn run(
    storage: &Storage,
    j: &mut Journal,
    lease: &DataMaintenanceLease,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    active(storage, lease, check)?;
    // Revalidate keeper content once per attempt, including after a process restart.
    let keeper = super::capture_mode(
        storage,
        Path::new(&j.models_root),
        &j.keeper.record.package_id,
        &j.keeper.record.review_hash,
        check,
        false,
    )?;
    if keeper.record != j.keeper.record || keeper.observations != j.keeper.observations {
        return Err("保持するモデルの内容・実体が変わっています".into());
    }
    while let Some(o) = j.source.observations.iter().rev().nth(j.next).cloned() {
        if o.entry.directory {
            super::super::verification::release_pins();
        }
        active(storage, lease, check)?;
        let models = verify_remaining(storage, j, check)?;
        let held = parent(&models, j, &o.entry.relative_path)?;
        let dir = held.as_ref().unwrap_or(&models);
        let name = if o.entry.relative_path.is_empty() {
            j.source.record.package_id.clone()
        } else {
            o.entry.relative_path.rsplit('/').next().unwrap().to_owned()
        };
        let reserved = j.reserved(j.next);
        if j.pending.is_none() {
            let before = observed(dir, &name, &o.entry, check)?;
            if before.identity != o.proof.as_ref().unwrap().identity
                || !o.entry.directory && Some(&before) != o.proof.as_ref()
            {
                return Err("整理する実体が変わっています".into());
            }
            let mut next = j.clone();
            next.pending = Some(Pending {
                before,
                staged: None,
            });
            save(storage, &next, Some(j), lease, check)?;
            *j = next;
        }
        let p = j.pending.clone().unwrap();
        if p.staged.is_none() {
            if dir.optional_kind(&name)?.is_some() {
                if observed(dir, &name, &o.entry, check)? != p.before {
                    return Err("整理直前に対象が変わりました".into());
                }
                active(storage, lease, check)?;
                dir.rename_exclusive(&name, &reserved)?;
            }
            let staged = observed(dir, &reserved, &o.entry, check)?;
            if !staged.matches_after_rename(&p.before) {
                return Err("整理の予約実体が一致しません".into());
            }
            let mut next = j.clone();
            next.pending.as_mut().unwrap().staged = Some(staged);
            save(storage, &next, Some(j), lease, check)?;
            *j = next;
        }
        if dir.optional_kind(&reserved)?.is_some() {
            verify_remaining(storage, j, check)?;
            let staged = j.pending.as_ref().unwrap().staged.as_ref().unwrap();
            if observed(dir, &reserved, &o.entry, check)? != *staged {
                return Err("整理直前に予約実体が変わりました".into());
            }
            active(storage, lease, check)?;
            dir.unlink_staged(&reserved, o.entry.directory, staged)?;
        } else {
            dir.sync()?;
        }
        let mut next = j.clone();
        next.next += 1;
        next.pending = None;
        save(storage, &next, Some(j), lease, check)?;
        *j = next;
    }
    verify_remaining(storage, j, check)?;
    Ok(())
}

/// Catalog order is newest first. Selected packages win; otherwise prefer the
/// newest checked package. Never promote an unchecked replacement to delete the
/// only known usable model, nor retire a package referenced by resumable work.
pub(crate) fn sweep(
    storage: &Storage,
    root: &Path,
    lease: &DataMaintenanceLease,
    failed: &mut HashSet<String>,
    check: &impl Fn() -> bool,
) -> Result<bool, String> {
    active(storage, lease, check)?;
    let _proofs = super::super::verification::Scope::enter();
    let ids = {
        let db = crate::operation_observation::lock(&storage.connection)?;
        journal_ids(&db)?
    };
    let mut changed = false;
    for id in ids {
        let mut journal = {
            let db = crate::operation_observation::lock(&storage.connection)?;
            read(&db, &id)?.ok_or("整理記録がありません")?
        };
        if journal.complete() || failed.contains(&id) {
            continue;
        }
        if journal.models_root != root.to_str().ok_or("モデル保存先が不正です")? {
            return Err("モデル整理の保存先が変わっています".into());
        }
        match run(storage, &mut journal, lease, check) {
            Ok(()) => changed = true,
            Err(error) => {
                if !check() && !lease.cancelled() {
                    failed.insert(id);
                }
                return Err(error);
            }
        }
    }
    let mut groups: BTreeMap<String, Vec<CatalogEntry>> = BTreeMap::new();
    for e in storage.list_model_placements()? {
        if e.placement_recorded && !e.body_removed {
            if let Some(key) = entry_identity(&e) {
                groups.entry(key).or_default().push(e);
            }
        }
    }
    for group in groups.values().filter(|g| g.len() > 1) {
        active(storage, lease, check)?;
        let keeper_id = {
            let db = crate::operation_observation::lock(&storage.connection)?;
            let selected_ids = super::super::super::model_profiles::current_packages(&db)?;
            keeper(group, &selected_ids).map(|e| e.package_id.clone())
        };
        let Some(keeper_id) = keeper_id else {
            continue;
        };
        let keeper_entry = group.iter().find(|e| e.package_id == keeper_id).unwrap();
        if !keeper_entry
            .compatibility
            .as_ref()
            .is_some_and(|c| c.state == crate::model_compatibility::CheckState::Succeeded)
        {
            continue;
        }
        for e in group.iter().filter(|e| e.package_id != keeper_id) {
            if failed.contains(&e.package_id) {
                continue;
            }
            let blocked = {
                let db = crate::operation_observation::lock(&storage.connection)?;
                protected(&db, &e.package_id)? || pending_keepers(&db)?.contains(&e.package_id)
            };
            if blocked {
                continue;
            }
            let result = (|| {
                let source = super::capture_mode(
                    storage,
                    root,
                    &e.package_id,
                    &e.review_hash,
                    check,
                    false,
                )?;
                let keeper = super::capture_mode(
                    storage,
                    root,
                    &keeper_id,
                    &keeper_entry.review_hash,
                    check,
                    false,
                )?;
                let mut j = Journal {
                    version: 1,
                    models_root: root.to_str().ok_or("モデル保存先が不正です")?.into(),
                    source,
                    keeper,
                    next: 0,
                    pending: None,
                };
                save(storage, &j, None, lease, check)?;
                run(storage, &mut j, lease, check)
            })();
            match result {
                Ok(()) => changed = true,
                Err(error) => {
                    if !check() && !lease.cancelled() {
                        failed.insert(e.package_id.clone());
                    }
                    return Err(error);
                }
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
#[path = "model_cleanup_tests.rs"]
mod tests;
