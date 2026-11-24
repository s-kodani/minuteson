//! Partial rescue. Examine only the caller's private SQLite copy, stage verified
//! media, and reconstruct ordinary application data without replaying old jobs.
use super::backup::contents::{self, Asset, Contents};
use super::publication_files::Directory;
use super::*;
use crate::model_placement::PlacementRecord;
use rusqlite::OpenFlags;
use std::collections::BTreeMap;
use std::io::Write;

mod inspect;
#[cfg(test)]
mod tests;

#[cfg(all(test, any(target_os = "macos", windows)))]
pub(crate) fn seed_cleanup_audio_fixture(storage: &Storage, operation: &str) -> String {
    let meeting = backup::archive::tests::meeting(storage, operation);
    storage
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT f.relative_path FROM meeting_audio_commits a
             JOIN audio_chunks c ON c.track_id=a.track_id
             JOIN file_assets f ON f.id=c.file_asset_id WHERE a.meeting_id=?1",
            [&meeting],
            |row| row.get(0),
        )
        .unwrap()
}

const RESERVE: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Recoverable,
    Partial,
    Unavailable,
    Uninspected,
    Defaulted,
    Absent,
    Deleted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Item {
    kind: String,
    source_id: Option<String>,
    label: String,
    status: Status,
    reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Preview {
    pub(crate) operation_id: String,
    pub(crate) source_root: String,
    pub(crate) required_bytes: u64,
    pub(crate) meeting_total: Option<usize>,
    pub(crate) items: Vec<Item>,
    pub(crate) recoverable_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ResultSummary {
    pub(crate) operation_id: String,
    pub(crate) source_root: String,
    pub(crate) source_models: Option<String>,
    pub(crate) destination_root: String,
    pub(crate) destination_models: String,
    pub(crate) meeting_count: usize,
    pub(crate) voice_person_count: usize,
    pub(crate) model_count: usize,
    pub(crate) items: Vec<Item>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Plan {
    pub(crate) preview: Preview,
    source_models: Option<String>,
    contents: Contents,
    models: Vec<PlacementRecord>,
    settings: Vec<Setting>,
    assets: BTreeMap<String, Staged>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Setting {
    key: String,
    json: String,
    hash: String,
    version: i64,
    updated_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Staged {
    owner: String,
    voice: bool,
    asset: Asset,
    #[cfg(any(target_os = "macos", windows))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_target: Option<super::deletion::salvage_source::Target>,
}

#[cfg(any(target_os = "macos", windows))]
impl Plan {
    pub(crate) fn copied_source_audio(
        &self,
    ) -> impl Iterator<Item = &super::deletion::salvage_source::Target> {
        self.assets
            .values()
            .filter_map(|asset| asset.source_target.as_ref())
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("データ復元を取り消しました。元データを保持しています".into())
    } else {
        Ok(())
    }
}

fn capacity(path: &Path, bytes: u64) -> Result<(), String> {
    if crate::audio_import::available_bytes(path)? < bytes {
        return Err("データ復元に必要な空き容量が不足しています".into());
    }
    Ok(())
}

fn add_bytes(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b)
        .ok_or_else(|| "データ復元の容量が範囲外です".into())
}

fn id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

fn remap(operation: &str, domain: &str, scope: &str, old: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"minuteson-salvage-id-v1");
    for key in [operation, domain, scope, old] {
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key.as_bytes());
    }
    let digest = digest.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // Domain-separated pseudorandom IDs are keyed by the fresh operation UUID.
    // Ordinary saved operation readers require UUIDv4.
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

fn copy_checked(
    source: &Path,
    relative: &str,
    target: &Directory,
    leaf: &str,
    size: u64,
    hash: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    check(cancelled)?;
    let mut source = crate::model_inspection::Source::open(source, relative)?;
    let mut output = target.create(leaf)?;
    let proof = publication_files::Identity::file(&output)?;
    let result = (|| {
        let mut buffer = [0u8; 64 * 1024];
        let mut read = 0u64;
        let mut digest = Sha256::new();
        loop {
            check(cancelled)?;
            let count = source.file.read(&mut buffer).map_err(err)?;
            if count == 0 {
                break;
            }
            read = add_bytes(read, count as u64)?;
            if read > size {
                return Err("復元ファイルの容量が記録と一致しません".into());
            }
            digest.update(&buffer[..count]);
            output.write_all(&buffer[..count]).map_err(err)?;
        }
        if read != size || mon_core::hash::sha256_output_hex(digest.finalize()) != hash {
            return Err("復元ファイルの内容が保存記録と一致しません".into());
        }
        source.verify()?;
        crate::native_fs::sync_handle(&output).map_err(err)?;
        target.verify()?;
        crate::native_fs::sync_directory(&target.path)
    })();
    if result.is_err() {
        drop(output);
        target.remove_owned(leaf, &proof)?;
    }
    result
}

fn directory(parent: &Directory, leaf: &str) -> Result<Directory, String> {
    let path = parent.path.join(leaf);
    match crate::native_fs::symlink_metadata(&path) {
        Ok(info) if info.is_dir() => {
            parent.verify()?;
            Directory::open(&path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => parent.create_child(leaf),
        Ok(_) => Err("復元先のフォルダーを確認できません".into()),
        Err(e) => Err(err(e)),
    }
}

fn target_file(root: &Directory, relative: &str) -> Result<(Directory, String), String> {
    if !crate::model_inspection::relative(relative) {
        return Err("復元先の相対パスが不正です".into());
    }
    let (parent, leaf) = relative.rsplit_once('/').ok_or("復元先の親がありません")?;
    root.verify()?;
    let mut current = Directory::open(&root.path)?;
    for name in parent.split('/') {
        current = directory(&current, name)?;
    }
    root.verify()?;
    Ok((current, leaf.into()))
}

pub(crate) use inspect::inspect;

pub(crate) fn build(
    plan: &Plan,
    workspace: &Path,
    destination_root: &Path,
    destination_models: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<ResultSummary, String> {
    let operation = &plan.preview.operation_id;
    if !id(operation) || plan.preview.recoverable_count == 0 {
        return Err("復元できるデータがないため切り替えません".into());
    }
    check(cancelled)?;
    capacity(
        destination_root.parent().ok_or("復元先の親がありません")?,
        plan.preview.required_bytes,
    )?;
    let workspace = workspace.canonicalize().map_err(err)?;
    let stage = Directory::open(&workspace.join("salvage-assets"))?;
    let root = Directory::open(&destination_root.canonicalize().map_err(err)?)?;
    let models_root = destination_models.canonicalize().map_err(err)?;
    let model_dir = crate::model_placement::fs::Directory::open_existing(&models_root)?;
    let summary = ResultSummary {
        operation_id: operation.clone(),
        source_root: plan.preview.source_root.clone(),
        source_models: plan.source_models.clone(),
        destination_root: root.path.to_str().ok_or("復元先を表示できません")?.into(),
        destination_models: models_root
            .to_str()
            .ok_or("モデル復元先を表示できません")?
            .into(),
        meeting_count: plan.contents.meetings.len(),
        voice_person_count: plan.contents.people.len(),
        model_count: plan.models.len(),
        items: plan.preview.items.clone(),
    };
    let identity = |domain: &str, scope: &str, key: &str| Ok(remap(operation, domain, scope, key));
    let asset_path = |meeting: &str, asset: &str| -> Result<String, String> {
        if meeting.is_empty() {
            let person = &plan
                .assets
                .get(asset)
                .filter(|record| record.voice)
                .ok_or("復元する登録音声がありません")?
                .owner;
            return voice::audio::relative(
                &remap(operation, "voice_people", "", person),
                &remap(operation, "voice_samples", "", asset),
            );
        }
        let mid = remap(operation, "meetings", "", meeting);
        let aid = remap(operation, "file_assets", meeting, asset);
        Ok(format!("meetings/{mid}/restored/{aid}"))
    };

    let mut copied = BTreeMap::new();
    let total = plan.assets.len() as u64;
    for (index, (key, record)) in plan.assets.iter().enumerate() {
        check(cancelled)?;
        if key != &record.asset.id || !id(key) {
            return Err("復元計画の音声IDが不正です".into());
        }
        crate::operation_observation::report("copying_salvage_audio", index as u64, total, "files");
        let meeting = if record.voice {
            ""
        } else {
            record.owner.as_str()
        };
        let relative = asset_path(meeting, key)?;
        let (dir, leaf) = target_file(&root, &relative)?;
        copy_checked(
            &stage.path,
            key,
            &dir,
            &leaf,
            u64::try_from(record.asset.size).map_err(err)?,
            &record.asset.hash,
            cancelled,
        )?;
        copied.insert(key.clone(), dir.path.join(leaf));
    }
    for m in &plan.contents.meetings {
        check(cancelled)?;
        let relative = asset_path(&m.meeting.id, &m.manifest.id)?;
        let (dir, name) = target_file(&root, &relative)?;
        let mut file = dir.create(&name)?;
        file.write_all(contents::manifest_bytes()).map_err(err)?;
        crate::native_fs::sync_handle(&file).map_err(err)?;
        crate::native_fs::sync_directory(&dir.path)?;
        drop(file);
        let mut verified = Vec::new();
        dir.required(&name, false)?
            .take(contents::manifest_bytes().len() as u64 + 1)
            .read_to_end(&mut verified)
            .map_err(err)?;
        if verified != contents::manifest_bytes() {
            return Err("復元先の会議保存記録が一致しません".into());
        }
    }

    let db_name = "meetings.sqlite3";
    let reserved = root.create(db_name)?;
    let proof = publication_files::Identity::file(&reserved)?;
    crate::native_fs::sync_handle(&reserved).map_err(err)?;
    drop(reserved);
    let path = root.path.join(db_name);
    let mut db = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(err)?;
    db.execute_batch(
        "PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
    )
    .map_err(err)?;
    initialize_database_with_guidance(&mut db, true).map_err(err)?;
    plan.contents
        .write_content(&mut db, &identity, &asset_path, cancelled)?;
    let tx = db.transaction().map_err(err)?;
    for setting in &plan.settings {
        if !inspect::known_setting(&setting.key) {
            return Err("復元計画に未対応の設定があります".into());
        }
        tx.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,?4,?5)",
            params![setting.key,setting.json,setting.hash,setting.version,setting.updated_at]).map_err(err)?;
        inspect::validate_setting(&tx, &setting.key)?;
    }
    for (index, model) in plan.models.iter().enumerate() {
        check(cancelled)?;
        crate::operation_observation::report(
            "copying_salvage_models",
            index as u64,
            plan.models.len() as u64,
            "files",
        );
        model.validate()?;
        let source = workspace.join("salvage-models").join(&model.package_id);
        let report = crate::model_inspection::inspect(&source, cancelled)?;
        if report.review_hash != model.review_hash {
            return Err("復元するモデルの内容が事前確認と一致しません".into());
        }
        let destination = model_dir.child(&model.package_id, true)?;
        crate::model_placement::copy_package(&source, &report, &destination, None, cancelled)?;
        if crate::model_inspection::inspect(destination.path(), cancelled)?.review_hash
            != model.review_hash
        {
            return Err("復元先モデルの内容を確認できません".into());
        }
        model_placements::insert_salvaged(&tx, model, &|relative| {
            remap(operation, "model_files", &model.package_id, relative)
        })?;
    }
    tx.execute(
        "INSERT INTO salvage_receipt(singleton,result_json) VALUES(1,?1)",
        [serde_json::to_string(&summary).map_err(err)?],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;

    crate::operation_observation::stage("verifying_salvage", None, "items");
    validate_destination(&mut db, plan, &copied, cancelled)?;
    db.close().map_err(|(_, error)| err(error))?;
    let file = root.required(db_name, false)?;
    proof.require(&file)?;
    crate::native_fs::sync_handle(&file).map_err(err)?;
    model_dir.sync()?;
    crate::native_fs::sync_directory(&root.path)?;
    check(cancelled)?;
    Ok(summary)
}

fn validate_destination(
    db: &mut Connection,
    plan: &Plan,
    copied: &BTreeMap<String, PathBuf>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    verify_database_schema(db).map_err(err)?;
    if db
        .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
        .map_err(err)?
        != "ok"
        || db
            .prepare("PRAGMA foreign_key_check")
            .map_err(err)?
            .query([])
            .map_err(err)?
            .next()
            .map_err(err)?
            .is_some()
    {
        return Err("復元先DBの整合性または外部キーを確認できません".into());
    }
    voice::validate_registry(db)?;
    vocabulary::verify_saved_books(db)?;
    let tx = db.transaction().map_err(err)?;
    for meeting in &plan.contents.meetings {
        check(cancelled)?;
        let id = remap(
            &plan.preview.operation_id,
            "meetings",
            "",
            &meeting.meeting.id,
        );
        if meeting.transcript.is_some() {
            minutes_input::current_working(&tx, &id)?;
        }
        minutes_markdown::read(&tx, &id)?;
        if meeting_audio::read(&tx, &id)?.is_some() != meeting.audio.is_some() {
            return Err("復元先の通常音声の読み取り結果が一致しません".into());
        }
    }
    check(cancelled)?;
    plan.contents.verify_audio(copied, cancelled)
}

impl Storage {
    pub(crate) fn has_current_salvage_result(&self, operation_id: &str) -> Result<bool, String> {
        Ok(self.salvage_result()?.is_some_and(|result| {
            result.operation_id == operation_id
                && Path::new(&result.destination_root) == self.root()
        }))
    }

    pub(crate) fn salvage_result(&self) -> Result<Option<ResultSummary>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        read_result(&db)
    }
}

pub(crate) fn read_result(db: &Connection) -> Result<Option<ResultSummary>, String> {
    db.query_row("SELECT CASE WHEN length(CAST(result_json AS BLOB))<=33554432 THEN result_json END FROM salvage_receipt WHERE singleton=1",
        [], |r| r.get::<_, String>(0)).optional().map_err(err)?
        .map(|json| serde_json::from_str(&json).map_err(err)).transpose()
}

pub(crate) fn read_result_from_path(path: &Path) -> Result<Option<ResultSummary>, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(err)?;
    db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON;")
        .map_err(err)?;
    read_result(&db)
}
