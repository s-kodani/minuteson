//! Read-only model deletion census. No consent, deletion permit or DB mutation.
use super::fs::{Directory, Identity, Proof};
use crate::{model_inspection::MANIFEST, model_placement::PlacementRecord, storage::Storage};
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

const MAX_ENTRIES: usize = 256;
const MAX_DEPTH: usize = 32;
const MAX_PATH: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    pub relative_path: String,
    pub directory: bool,
    pub registered: bool,
    pub present: bool,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub changed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Observation {
    entry: Entry,
    proof: Option<Proof>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
        Err("モデル削除の対象検査を取り消しました。データは変更していません".into())
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
    let mut file = dir.regular(leaf)?;
    let before = Proof::read(&file.metadata().map_err(|e| e.to_string())?)?;
    before.validate(false)?;
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0; 65_536];
    loop {
        cancel(check)?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or("モデル容量が範囲外です")?;
        if size > before.size {
            return Err("モデル検査中にファイルが増大しました".into());
        }
        hash.update(&buffer[..n]);
    }
    dir.verify_file(leaf, &file)?;
    if size != before.size || Proof::read(&file.metadata().map_err(|e| e.to_string())?)? != before {
        return Err("モデル検査中にファイルが変更されました".into());
    }
    Ok((before, format!("{:x}", hash.finalize())))
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
fn inventory(
    models: &Directory,
    record: &PlacementRecord,
    check: &impl Fn() -> bool,
) -> Result<Vec<Observation>, String> {
    let registered = expected(record);
    let mut result = BTreeMap::new();
    if let Some(package) = models.child(&record.package_id)? {
        walk(&package, "", &registered, &mut result, check)?;
    }
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
    cancel(check)?;
    storage.require_data_writable()?;
    if !uuid::Uuid::parse_str(package).is_ok_and(|id| id.to_string() == package)
        || root.file_name() != Some(std::ffi::OsStr::new("models"))
        // Pre-bootstrap installs keep their meeting DB in app-data beside
        // models. Only that exact native child is compatible; nested meeting
        // paths are not model roots. The IPC chooses app-data/models itself.
        || (root.starts_with(storage.root()) && root != storage.root().join("models"))
        || storage.root().starts_with(root)
    {
        return Err("モデル削除の管理領域・対象IDが不正です".into());
    }
    let models = Directory::root(root)?;
    let models_root = models.proof()?.identity;
    let db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let (record, placed) =
        storage::model_placements::load(&db, package)?.ok_or("管理モデルの配置記録がありません")?;
    if record.review_hash != review {
        return Err("モデル削除の確認対象が配置記録と一致しません".into());
    }
    let (storage_root, database) = storage.managed_anchor.identity()?;
    let observations = inventory(&models, &record, check)?;
    let (profile_references, job_references, active_jobs) = usages(&db, package)?;
    if inventory(&models, &record, check)? != observations
        || models.proof()?.identity != models_root
        || storage.managed_anchor.identity()? != (storage_root.clone(), database.clone())
        || storage::model_placements::load(&db, package)? != Some((record.clone(), placed))
        || usages(&db, package)? != (profile_references, job_references, active_jobs)
    {
        return Err("モデル削除の対象・記録が検査中に変更されました".into());
    }
    cancel(check)?;
    storage.require_data_writable()?;
    Ok(Snapshot {
        record,
        placed,
        storage_root,
        database,
        models_root,
        observations,
        profile_references,
        job_references,
        active_jobs,
    })
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
mod tests;
