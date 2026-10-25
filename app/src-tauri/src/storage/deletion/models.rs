//! Read-only model deletion census. No consent, deletion permit or DB mutation.
use super::fs::{Directory, Identity, Proof};
use crate::{model_inspection::MANIFEST, model_placement::PlacementRecord, storage::Storage};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

const MAX_ENTRIES: usize = 256;
const MAX_DEPTH: usize = 32;
const MAX_PATH: usize = 4096;

#[path = "model_operations.rs"]
pub(crate) mod operations;

#[path = "model_startup.rs"]
pub(in crate::storage) mod startup;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Entry {
    pub relative_path: String,
    pub directory: bool,
    pub registered: bool,
    pub present: bool,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub changed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    entry: Entry,
    proof: Option<Proof>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    record: PlacementRecord,
    placed: bool,
    storage_root: Identity,
    database: Identity,
    models_root: Identity,
    observations: Vec<Observation>,
    profile_references: u64,
    job_references: u64,
    active_jobs: u64,
}
impl Snapshot {
    fn validate(&self) -> Result<(), String> {
        self.record.validate()?;
        self.storage_root.validate(true)?;
        self.database.validate(false)?;
        self.models_root.validate(true)?;
        let registered = expected(&self.record);
        let mut paths = BTreeMap::new();
        if self.observations.is_empty() || self.observations.len() > MAX_ENTRIES {
            return Err("モデル削除の保存対象数が不正です".into());
        }
        let mut previous: Option<&str> = None;
        for observation in &self.observations {
            let e = &observation.entry;
            let path = e.relative_path.as_str();
            if path.len() > MAX_PATH
                || path.split('/').count() > MAX_DEPTH
                || previous.is_some_and(|p| p >= path)
                || (!path.is_empty()
                    && path.split('/').any(|leaf| {
                        leaf.is_empty()
                            || leaf == "."
                            || leaf == ".."
                            || leaf.len() > 255
                            || leaf.contains(['\\', '\0', ':'])
                    }))
                || e.registered != registered.contains_key(path)
                || registered
                    .get(path)
                    .is_some_and(|v| v.is_none() != e.directory)
                || (!e.present && !e.registered)
                || observation.proof.is_some() != e.present
            {
                return Err("モデル削除の保存対象・順序・実体証拠が不正です".into());
            }
            if let Some(proof) = &observation.proof {
                proof.validate(e.directory)?;
            }
            if e.directory || !e.present {
                if e.bytes.is_some() || e.sha256.is_some() || e.changed {
                    return Err("モデル削除の保存容量・変更分類が不正です".into());
                }
            } else {
                let proof = observation
                    .proof
                    .as_ref()
                    .ok_or("モデル実体証拠がありません")?;
                let hash = e.sha256.as_deref().ok_or("モデル内容証拠がありません")?;
                let changed =
                    registered
                        .get(path)
                        .and_then(Option::as_ref)
                        .is_some_and(|(size, sha)| {
                            size.is_some_and(|n| n != proof.size) || sha != hash
                        });
                if e.bytes != Some(proof.size) || !intent::hash_valid(hash) || e.changed != changed
                {
                    return Err("モデル削除の保存内容・変更分類が不正です".into());
                }
            }
            if !path.is_empty() {
                let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
                let parent: &Entry = paths.get(parent).ok_or("モデル削除の親対象がありません")?;
                if !parent.directory || (e.present && !parent.present) {
                    return Err("モデル削除の親対象が不正です".into());
                }
            } else if !e.directory || !e.registered {
                return Err("モデル削除のルート対象が不正です".into());
            }
            paths.insert(path, e.clone());
            previous = Some(path);
        }
        if registered
            .keys()
            .any(|path| !paths.contains_key(path.as_str()))
        {
            return Err("登録モデルの保存対象が欠落しています".into());
        }
        Ok(())
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preview {
    pub schema_version: u32,
    pub inspection_id: String,
    pub package_id: String,
    pub review_hash: String,
    pub census_hash: String,
    pub model_name: String,
    pub model_version: String,
    pub models_root: String,
    pub entries: Vec<Entry>,
    pub bytes: u64,
    pub missing_files: usize,
    pub changed_files: usize,
    pub unregistered_files: usize,
    pub profile_references: u64,
    pub job_references: u64,
    pub active_jobs: u64,
    pub execute_allowed: bool,
}

fn cancel(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("モデル削除の対象検査を取り消しました。未処理の対象を保持し、既に削除したファイルは復元しません".into())
    } else {
        Ok(())
    }
}
fn expected(record: &PlacementRecord) -> BTreeMap<String, Option<(Option<u64>, String)>> {
    let mut paths = BTreeMap::new();
    paths.insert(String::new(), None);
    paths.insert(
        MANIFEST.into(),
        Some((None, record.source_manifest_hash.clone())),
    );
    for f in &record.files {
        paths.insert(
            f.relative_path.clone(),
            Some((Some(f.actual_size), f.actual_hash.clone())),
        );
        let mut path = f.relative_path.as_str();
        while let Some((parent, _)) = path.rsplit_once('/') {
            paths.entry(parent.into()).or_insert(None);
            path = parent;
        }
    }
    paths
}
fn hash_file(
    dir: &Directory,
    leaf: &str,
    check: &impl Fn() -> bool,
) -> Result<(Proof, String), String> {
    super::verification::hash(dir, leaf, check)
}
fn walk(
    dir: &Directory,
    path: &str,
    registered: &BTreeMap<String, Option<(Option<u64>, String)>>,
    result: &mut BTreeMap<String, Observation>,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    cancel(check)?;
    if path.len() > MAX_PATH || path.split('/').count() > MAX_DEPTH || result.len() >= MAX_ENTRIES {
        return Err(
            "モデル削除の対象検査が件数・深さ・パスの上限を超えました。対象を保持します".into(),
        );
    }
    let proof = dir.proof()?;
    proof.validate(true)?;
    result.insert(
        path.into(),
        Observation {
            entry: Entry {
                relative_path: path.into(),
                directory: true,
                registered: registered.contains_key(path),
                present: true,
                bytes: None,
                sha256: None,
                changed: false,
            },
            proof: Some(proof.clone()),
        },
    );
    for leaf in dir.names(MAX_ENTRIES)? {
        cancel(check)?;
        let relative = if path.is_empty() {
            leaf.clone()
        } else {
            format!("{path}/{leaf}")
        };
        if relative.len() > MAX_PATH
            || relative.split('/').count() > MAX_DEPTH
            || result.len() >= MAX_ENTRIES
        {
            return Err(
                "モデル削除の対象検査が件数・パスの上限を超えました。対象を保持します".into(),
            );
        }
        let directory = dir.kind(&leaf)?;
        if registered
            .get(&relative)
            .is_some_and(|value| value.is_none() != directory)
        {
            return Err(
                "登録モデルのファイルとフォルダーが入れ替わっています。対象を保持します".into(),
            );
        }
        if directory {
            let child = dir
                .child(&leaf)?
                .ok_or("モデルフォルダーが検査中に消失しました")?;
            walk(&child, &relative, registered, result, check)?;
        } else {
            let (proof, hash) = hash_file(dir, &leaf, check)?;
            let expected = registered.get(&relative).and_then(Option::as_ref);
            let changed = expected
                .is_some_and(|(size, sha)| size.is_some_and(|n| n != proof.size) || sha != &hash);
            result.insert(
                relative.clone(),
                Observation {
                    entry: Entry {
                        relative_path: relative,
                        directory: false,
                        registered: expected.is_some(),
                        present: true,
                        bytes: Some(proof.size),
                        sha256: Some(hash),
                        changed,
                    },
                    proof: Some(proof),
                },
            );
        }
    }
    if dir.proof()? != proof {
        return Err("モデル検査中にフォルダーが変更されました".into());
    }
    Ok(())
}
// Removal compares only actual objects; its own staged registered file must
// not consume a second slot as a synthesized missing original at the 256 limit.
fn present_inventory(
    models: &Directory,
    record: &PlacementRecord,
    check: &impl Fn() -> bool,
) -> Result<Vec<Observation>, String> {
    let registered = expected(record);
    let mut result = BTreeMap::new();
    if let Some(package) = models.child(&record.package_id)? {
        walk(&package, "", &registered, &mut result, check)?;
    }
    models.verify()?;
    Ok(result.into_values().collect())
}
fn inventory(
    models: &Directory,
    record: &PlacementRecord,
    check: &impl Fn() -> bool,
) -> Result<Vec<Observation>, String> {
    let mut result: BTreeMap<_, _> = present_inventory(models, record, check)?
        .into_iter()
        .map(|o| (o.entry.relative_path.clone(), o))
        .collect();
    let registered = expected(record);
    for (path, kind) in registered {
        if !result.contains_key(&path) {
            if result.len() >= MAX_ENTRIES {
                return Err("欠落したモデルを含む対象数が上限を超えています".into());
            }
            result.insert(
                path.clone(),
                Observation {
                    entry: Entry {
                        relative_path: path,
                        directory: kind.is_none(),
                        registered: true,
                        present: false,
                        bytes: None,
                        sha256: None,
                        changed: false,
                    },
                    proof: None,
                },
            );
        }
    }
    models.verify()?;
    Ok(result.into_values().collect())
}
fn usages(db: &Connection, package: &str) -> Result<(u64, u64, u64), String> {
    let profiles = db
        .query_row(
            "SELECT count(*) FROM profile_components WHERE package_id=?1",
            params![package],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())?;
    // Both original and explicitly rebound job provenance are historical
    // references, not running-process ownership or permission to delete.
    let jobs = db.query_row("SELECT count(*) FROM processing_jobs WHERE json_extract(profile_snapshot_json,'$.managedModel.package.packageId')=?1 OR json_extract(profile_snapshot_json,'$.runtimeModel.package.packageId')=?1",params![package],|r|r.get::<_, i64>(0)).map_err(|e|e.to_string())?;
    let active = db
        .query_row(
            "SELECT count(*) FROM processing_jobs WHERE state IN ('running','pausing')",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())?;
    let count = |n: i64| u64::try_from(n).map_err(|_| "モデル参照件数が範囲外です".to_string());
    Ok((count(profiles)?, count(jobs)?, count(active)?))
}
fn capture(
    storage: &Storage,
    root: &Path,
    package: &str,
    review: &str,
    check: &impl Fn() -> bool,
) -> Result<Snapshot, String> {
    storage.require_data_writable()?;
    let result = capture_unlocked(storage, root, package, review, check)?;
    storage.require_data_writable()?;
    Ok(result)
}
fn capture_unlocked(
    storage: &Storage,
    root: &Path,
    package: &str,
    review: &str,
    check: &impl Fn() -> bool,
) -> Result<Snapshot, String> {
    validate_root(storage, root, package)?;
    let metadata = || -> Result<_, String> {
        let db = crate::operation_observation::lock(&storage.connection)?;
        Ok((
            storage::model_placements::load_history(&db, package)?
                .ok_or("管理モデルの配置記録がありません")?,
            usages(&db, package)?,
        ))
    };
    let ((record, placed), counts) = metadata()?;
    if record.review_hash != review {
        return Err("モデル削除の確認対象が配置記録と一致しません".into());
    }
    let models = Directory::root(root)?;
    let models_root = models.proof()?.identity;
    let (storage_root, database) = storage.managed_anchor.identity()?;
    let observations = inventory(&models, &record, check)?;
    if inventory(&models, &record, check)? != observations
        || metadata()? != ((record.clone(), placed), counts)
        || models.proof()?.identity != models_root
        || storage.managed_anchor.identity()? != (storage_root.clone(), database.clone())
    {
        return Err("モデル削除の対象・記録が検査中に変更されました".into());
    }
    let snapshot = Snapshot {
        record,
        placed,
        storage_root,
        database,
        models_root,
        observations,
        profile_references: counts.0,
        job_references: counts.1,
        active_jobs: counts.2,
    };
    snapshot.validate()?;
    Ok(snapshot)
}
impl Snapshot {
    // This is a freshness check for already hashed consent, never permission
    // to remove a file. Execution still rehashes at irreversible boundaries.
    fn verify_metadata(
        &self,
        storage: &Storage,
        db: &Connection,
        root: &Path,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        cancel(check)?;
        if storage::model_placements::load_history(db, &self.record.package_id)?
            != Some((self.record.clone(), self.placed))
            || usages(db, &self.record.package_id)?
                != (
                    self.profile_references,
                    self.job_references,
                    self.active_jobs,
                )
            || storage.managed_anchor.identity()?
                != (self.storage_root.clone(), self.database.clone())
        {
            return Err("モデル削除の参照・配置・保存領域が変更されています".into());
        }
        let models = Directory::root(root)?;
        models.verify_identity(&self.models_root)?;
        let package = models.child(&self.record.package_id)?;
        let expected: BTreeMap<_, _> = self
            .observations
            .iter()
            .map(|o| (o.entry.relative_path.as_str(), o))
            .collect();
        match package {
            Some(directory) => verify_metadata_tree(&directory, "", &expected, check)?,
            None if self.observations.iter().all(|o| o.proof.is_none()) => {}
            None => return Err("確認したモデルが消失しました".into()),
        }
        models.verify()
    }
}
fn verify_metadata_tree(
    dir: &Directory,
    path: &str,
    expected: &BTreeMap<&str, &Observation>,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    cancel(check)?;
    let before = dir.proof()?;
    if expected.get(path).and_then(|o| o.proof.as_ref()) != Some(&before) {
        return Err("モデルフォルダーの実体が変更されています".into());
    }
    let names = dir.names(MAX_ENTRIES)?;
    let mut wanted = std::collections::BTreeSet::new();
    for (name, o) in expected {
        if o.proof.is_some() && !name.is_empty() {
            let (parent, leaf) = name.rsplit_once('/').unwrap_or(("", name));
            if parent == path {
                wanted.insert(leaf.to_string());
            }
        }
    }
    if names
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        != wanted
    {
        return Err("モデルに追加・欠落したファイルがあります".into());
    }
    for leaf in names {
        cancel(check)?;
        let relative = if path.is_empty() {
            leaf.clone()
        } else {
            format!("{path}/{leaf}")
        };
        let o = expected
            .get(relative.as_str())
            .ok_or("未知のモデルファイルがあります")?;
        if o.entry.directory {
            let child = dir.child(&leaf)?.ok_or("モデルフォルダーが消失しました")?;
            verify_metadata_tree(&child, &relative, expected, check)?;
        } else {
            let file = dir.regular(&leaf)?;
            if o.proof.as_ref()
                != Some(&Proof::read(
                    &crate::native_fs::metadata(&file).map_err(|e| e.to_string())?,
                )?)
            {
                return Err("モデルの内容条件が変更されています".into());
            }
            dir.verify_file(&leaf, &file)?;
        }
    }
    if dir.proof()? != before {
        return Err("検査中にモデルフォルダーが変更されました".into());
    }
    Ok(())
}
fn validate_root(storage: &Storage, root: &Path, package: &str) -> Result<(), String> {
    if !uuid::Uuid::parse_str(package).is_ok_and(|id| id.to_string() == package)
        || root.file_name() != Some(std::ffi::OsStr::new("models"))
        || (root.starts_with(storage.root()) && root != storage.root().join("models"))
        || storage.root().starts_with(root)
    {
        return Err("モデル削除の管理領域・対象IDが不正です".into());
    }
    Ok(())
}

use crate::storage;
pub(crate) fn preview(
    storage: &Storage,
    root: &Path,
    inspection: &str,
    package: &str,
    review: &str,
    check: &impl Fn() -> bool,
) -> Result<Preview, String> {
    if !uuid::Uuid::parse_str(inspection).is_ok_and(|id| id.to_string() == inspection) {
        return Err("モデル検査の操作IDが不正です".into());
    }
    let snapshot = capture(storage, root, package, review, check)?;
    let json = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    if json.len() > 1_048_576 {
        return Err("モデル削除の検査記録が容量上限を超えています".into());
    }
    let entries = snapshot
        .observations
        .into_iter()
        .map(|o| o.entry)
        .collect::<Vec<_>>();
    let mut bytes = 0_u64;
    for e in &entries {
        bytes = bytes
            .checked_add(e.bytes.unwrap_or(0))
            .ok_or("モデル容量が範囲外です")?;
    }
    Ok(Preview {
        schema_version: 1,
        inspection_id: inspection.into(),
        package_id: package.into(),
        review_hash: review.into(),
        census_hash: format!("{:x}", Sha256::digest(json)),
        model_name: snapshot.record.manifest.model_name,
        model_version: snapshot.record.manifest.model_version,
        models_root: root.to_string_lossy().into(),
        bytes,
        missing_files: entries
            .iter()
            .filter(|e| !e.directory && !e.present)
            .count(),
        changed_files: entries.iter().filter(|e| e.changed).count(),
        unregistered_files: entries
            .iter()
            .filter(|e| !e.directory && !e.registered)
            .count(),
        entries,
        profile_references: snapshot.profile_references,
        job_references: snapshot.job_references,
        active_jobs: snapshot.active_jobs,
        execute_allowed: false,
    })
}

#[cfg(test)]
#[path = "model_tests.rs"]
pub(crate) mod tests;

#[path = "model_intent.rs"]
pub(crate) mod intent;
