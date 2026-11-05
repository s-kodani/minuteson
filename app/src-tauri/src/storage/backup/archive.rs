//! Framed, uncompressed, hashed portable data; no SQL/ZIP/host paths to execute.
use super::portable::{self, Table};
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Seek, SeekFrom};

const MAGIC: &[u8; 8] = b"MONBAK02";
const MAX_MANIFEST: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 65_536;
const HEADER_BYTES: u64 = 8 + 4 + 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Encryption {
    None,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HashBasis {
    SourceIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Transform {
    Identity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OmittedReason {
    SourceAudioDeleted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Entry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub asset_id: Option<String>,
    pub meeting_id: Option<String>,
    pub source_size: Option<u64>,
    pub source_sha256: Option<String>,
    transform: Transform,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct OmittedAsset {
    pub asset_id: String,
    pub meeting_id: String,
    pub path: String,
    pub source_size: u64,
    pub source_sha256: String,
    pub(super) reason: OmittedReason,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub format_version: u32,
    pub schema_version: usize,
    pub archive_id: String,
    pub created_at: i64,
    encryption: Encryption,
    hash_basis: HashBasis,
    pub meetings: Vec<Meeting>,
    #[serde(skip)]
    pub(super) tables: Vec<Table>,
    pub(super) entries: Vec<Entry>,
    pub(super) omitted_assets: Vec<OmittedAsset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) deleted_audio: Vec<deleted_audio::Descriptor>,
}

pub(crate) struct VerifiedArchive {
    // This is a validation/source-ID DB, NOT a replacement or a runnable app DB.
    // Connection closes before the private directory is removed on all platforms.
    pub(super) database: Connection,
    pub(super) contents: contents::Contents,
    pub manifest: Manifest,
    pub(super) assets: BTreeMap<String, PathBuf>,
    pub(super) deleted_audio: deleted_audio::Verified,
    database_proof: (u64, String),
    database_identity: Metadata,
    stage: scratch::Stage,
}

impl VerifiedArchive {
    pub(super) fn verify_validation_database(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let path = self.stage.path().join("validation.sqlite3");
        if !same_file(
            &self.database_identity,
            &crate::native_fs::symlink_metadata(&path).map_err(|e| e.to_string())?,
        ) {
            return Err("検証済みバックアップの確認用DBの実体が変更されています".into());
        }
        if digest_file(&path, cancelled)? != self.database_proof {
            return Err("検証済みバックアップの確認用DBが変更されています".into());
        }
        Ok(())
    }
    pub(super) fn owned_asset_path(&self, id: &str) -> Result<PathBuf, String> {
        if !crate::native_fs::symlink_metadata(self.stage.path())
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err("検査専用領域が変更されています".into());
        }
        let asset = self
            .assets
            .get(id)
            .ok_or("検査済み管理ファイルがありません")?;
        let relative = asset
            .strip_prefix(self.stage.path())
            .map_err(|_| "復元ファイルが検査領域外です")?;
        Ok(self
            .stage
            .path()
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join(relative))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackupReceipt {
    pub archive_id: String,
    pub meeting_count: usize,
    pub asset_count: usize,
    pub deleted_audio_meeting_count: usize,
    pub deleted_audio_asset_count: usize,
    pub size: u64,
    pub sha256: String,
}

pub(super) fn private_stage(parent: &Path) -> Result<scratch::Stage, String> {
    scratch::Stage::create(parent)
}

impl Snapshot<'_> {
    fn write_archive(
        &self,
        output: &mut File,
        parent: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BackupReceipt, String> {
        self.verify(cancelled)?;
        let stage = private_stage(self.stage.path())?;
        let data = stage.path().join("contents.json");
        let mut file = private_file(&data)?;
        serde_json::to_writer(&mut file, &self.contents).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        let (size, sha256) = digest_file(&data, cancelled)?;
        if size > contents::MAX_BYTES {
            return Err("会議バックアップの内容が大きすぎます".into());
        }
        let mut entries = vec![Entry {
            path: "contents.json".into(),
            size,
            sha256,
            asset_id: None,
            meeting_id: None,
            source_size: None,
            source_sha256: None,
            transform: Transform::Identity,
        }];
        let mut sources = vec![data];
        let mut omitted = Vec::new();
        for item in &self.assets {
            let a = &item.asset;
            let path = format!(
                "{}/{}",
                if a.state == "missing" {
                    "omitted"
                } else {
                    "assets"
                },
                item.key
            );
            if a.state == "missing" {
                omitted.push(OmittedAsset {
                    asset_id: a.id.clone(),
                    meeting_id: a.meeting_id.clone(),
                    path,
                    source_size: a.size_bytes as u64,
                    source_sha256: a.content_hash.clone(),
                    reason: OmittedReason::SourceAudioDeleted,
                });
            } else {
                entries.push(Entry {
                    path,
                    size: a.size_bytes as u64,
                    sha256: a.content_hash.clone(),
                    asset_id: Some(a.id.clone()),
                    meeting_id: (!a.meeting_id.is_empty()).then(|| a.meeting_id.clone()),
                    source_size: Some(a.size_bytes as u64),
                    source_sha256: Some(a.content_hash.clone()),
                    transform: Transform::Identity,
                });
                sources.push(item.path.clone());
            }
        }
        let lookup = |_meeting: &str, id: &str| -> Result<String, String> {
            entries
                .iter()
                .find(|e| e.asset_id.as_deref() == Some(id))
                .map(|e| e.path.clone())
                .or_else(|| {
                    omitted
                        .iter()
                        .find(|e| e.asset_id == id)
                        .map(|e| e.path.clone())
                })
                .ok_or("会議内容の資産がありません".into())
        };
        let logical = self.contents.write_database(
            &stage.path().join("current.sqlite3"),
            &|_, _, id| Ok(id.to_owned()),
            &lookup,
            cancelled,
        )?;
        let deleted_audio = self.deleted_audio.bind(&logical, cancelled)?;
        deleted_audio::verify(&logical, &deleted_audio, &omitted, cancelled)?;
        let manifest = Manifest {
            format_version: crate::current_formats::BACKUP,
            schema_version: SCHEMA_VERSION,
            archive_id: Uuid::new_v4().to_string(),
            created_at: now_ms()?,
            encryption: Encryption::None,
            hash_basis: HashBasis::SourceIdentity,
            meetings: read_meetings(&logical)?,
            tables: portable::tables(&logical)?,
            entries,
            omitted_assets: omitted,
            deleted_audio,
        };
        validate_manifest(&manifest)?;
        let bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("バックアップmanifestが大きすぎます".into());
        }
        let required = manifest
            .entries
            .iter()
            .try_fold(HEADER_BYTES + bytes.len() as u64, |n, e| {
                n.checked_add(e.size).ok_or("バックアップ容量が範囲外です")
            })?;
        check_capacity(parent, required)?;
        output
            .write_all(MAGIC)
            .and_then(|_| output.write_all(&(bytes.len() as u32).to_le_bytes()))
            .and_then(|_| output.write_all(&Sha256::digest(&bytes)))
            .and_then(|_| output.write_all(&bytes))
            .map_err(|e| e.to_string())?;
        crate::operation_observation::stage("writing_backup", Some(required), "bytes");
        crate::operation_observation::advance(HEADER_BYTES + bytes.len() as u64);
        for (entry, path) in manifest.entries.iter().zip(sources) {
            let (mut source, before) = open_source(&path)?;
            if stream_observed(&mut source, Some(output), entry.size, cancelled)?.1 != entry.sha256
                || !same_file(
                    &before,
                    &crate::native_fs::metadata(&source).map_err(|e| e.to_string())?,
                )
            {
                return Err("バックアップ作成中に入力が変更されています".into());
            }
        }
        output.sync_all().map_err(|e| e.to_string())?;
        crate::operation_observation::stage("verifying_backup", Some(required), "bytes");
        let verified = read(output, stage.path(), cancelled)?;
        output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        crate::operation_observation::stage("verifying_backup", Some(required), "bytes");
        let (_, sha256) = stream_observed(output, None, required, cancelled)?;
        crate::operation_observation::stage("checking_source", None, "bytes");
        self.verify(cancelled)?;
        Ok(BackupReceipt {
            archive_id: verified.manifest.archive_id.clone(),
            meeting_count: verified.manifest.meetings.len(),
            asset_count: verified.assets.len(),
            deleted_audio_meeting_count: verified.manifest.deleted_audio.len(),
            deleted_audio_asset_count: verified.manifest.omitted_assets.len(),
            size: required,
            sha256,
        })
    }
}

impl Storage {
    pub(crate) fn create_backup_archive(
        &self,
        destination: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BackupReceipt, crate::exports::PublishError> {
        let target =
            crate::exports::Destination::selected_backup(&self.root, destination, cancelled)
                .map_err(|message| crate::exports::PublishError {
                    published: false,
                    message,
                })?;
        self.create_backup_selected(&target, cancelled)
    }

    /// Keep the exact selection proof through explicit overwrite confirmation.
    /// Never recapture a changed destination and silently approve its contents.
    pub(crate) fn create_backup_selected(
        &self,
        target: &crate::exports::Destination,
        cancelled: &impl Fn() -> bool,
    ) -> Result<BackupReceipt, crate::exports::PublishError> {
        target
            .validate_backup_owner(&self.root)
            .map_err(|message| crate::exports::PublishError {
                published: false,
                message,
            })?;
        let mut receipt = None;
        let snapshot = self.prepare_backup_snapshot(cancelled).map_err(|message| {
            crate::exports::PublishError {
                published: false,
                message,
            }
        })?;
        target.publish_with(
            |file| {
                receipt = Some(snapshot.write_archive(
                    file,
                    target.path.parent().ok_or("保存先がありません")?,
                    cancelled,
                )?);
                Ok(())
            },
            cancelled,
        )?;
        receipt.ok_or(crate::exports::PublishError {
            published: true,
            message: "バックアップの確定結果を読み取れません".into(),
        })
    }
}

fn key(path: &str, prefix: &str) -> bool {
    let Some(id) = path.strip_prefix(prefix) else {
        return false;
    };
    Uuid::parse_str(id).is_ok_and(|id_value| id_value.to_string() == id)
}

fn identifier(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.contains('\0')
}

fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if m.format_version != crate::current_formats::BACKUP || m.schema_version != SCHEMA_VERSION {
        return Err(
            "このバックアップ形式には対応していません。新しい形式で作成してください".into(),
        );
    }
    if !Uuid::parse_str(&m.archive_id).is_ok_and(|id| id.to_string() == m.archive_id)
        || m.created_at < 0
        || m.entries.is_empty()
        || m.entries.len() + m.omitted_assets.len() > MAX_ENTRIES
        || m.deleted_audio.len() > deleted_audio::MAX_MEETINGS
    {
        return Err("バックアップの形式版・件数が不正です".into());
    }
    let mut meetings = BTreeSet::new();
    for meeting in &m.meetings {
        if !identifier(&meeting.id)
            || !meetings.insert(&meeting.id)
            || meeting.title.trim().is_empty()
            || meeting.title.chars().count() > 200
        {
            return Err("バックアップの会議一覧が不正です".into());
        }
    }
    let mut deleted_meetings = BTreeSet::new();
    for item in &m.deleted_audio {
        item.validate_shape()?;
        if !meetings.contains(&item.meeting_id) || !deleted_meetings.insert(&item.meeting_id) {
            return Err("バックアップの音声削除会議が不正です".into());
        }
    }
    let mut paths = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for (index, e) in m.entries.iter().enumerate() {
        if !portable::valid_hash(&e.sha256) || !paths.insert(&e.path) {
            return Err("バックアップのパス・ハッシュが不正です".into());
        }
        if index == 0 {
            if e.size == 0
                || e.size > contents::MAX_BYTES
                || e.path != "contents.json"
                || e.asset_id.is_some()
                || e.meeting_id.is_some()
                || e.source_size.is_some()
                || e.source_sha256.is_some()
            {
                return Err("バックアップDB記録が不正です".into());
            }
        } else if !key(&e.path, "assets/")
            || e.asset_id
                .as_deref()
                .is_none_or(|id| !identifier(id) || !ids.insert(id))
            || e.meeting_id
                .as_ref()
                .is_some_and(|id| !meetings.contains(id))
            || e.source_size.is_none()
            || e.source_sha256
                .as_deref()
                .is_none_or(|s| !portable::valid_hash(s))
            || e.source_size != Some(e.size)
            || e.source_sha256.as_deref() != Some(&e.sha256)
        {
            return Err("バックアップ管理ファイルの由来が不正です".into());
        }
    }
    for e in &m.omitted_assets {
        if !identifier(&e.asset_id)
            || !ids.insert(&e.asset_id)
            || !meetings.contains(&e.meeting_id)
            || !key(&e.path, "omitted/")
            || !paths.insert(&e.path)
            || !portable::valid_hash(&e.source_sha256)
            || !deleted_meetings.contains(&e.meeting_id)
        {
            return Err("除外ファイルの由来が不正です".into());
        }
    }
    Ok(())
}

pub(crate) fn read(
    input: &mut File,
    parent: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<VerifiedArchive, String> {
    check_cancelled(cancelled)?;
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let before = crate::native_fs::metadata(&input).map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() < HEADER_BYTES {
        return Err("バックアップが短すぎます".into());
    }
    let mut magic = [0; 8];
    input.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if &magic != MAGIC {
        return Err("MinutesOnバックアップ形式ではありません".into());
    }
    let mut length = [0; 4];
    let mut expected = [0; 32];
    input
        .read_exact(&mut length)
        .and_then(|_| input.read_exact(&mut expected))
        .map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_MANIFEST || length as u64 > before.len() - HEADER_BYTES {
        return Err("バックアップmanifest容量が不正です".into());
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    if Sha256::digest(&bytes).as_slice() != expected {
        return Err("バックアップmanifestハッシュが一致しません".into());
    }
    let mut manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate_manifest(&manifest)?;
    let total = manifest
        .entries
        .iter()
        .try_fold(HEADER_BYTES + length as u64, |sum, e| {
            sum.checked_add(e.size)
                .ok_or("バックアップ容量が範囲外です")
        })?;
    if total != before.len() {
        return Err("バックアップの全体容量・末尾が不正です".into());
    }
    let logical = manifest.entries[0].size;
    let required = total
        .checked_add(logical.checked_mul(4).ok_or("検証DB容量が範囲外です")?)
        .ok_or("検証容量が範囲外です")?;
    check_capacity(parent, required)?;
    let stage = private_stage(parent)?;
    scratch::directories(&stage.path().join("assets"))?;
    let mut assets = BTreeMap::new();
    crate::operation_observation::stage("reading_backup", Some(total), "bytes");
    crate::operation_observation::advance(HEADER_BYTES + length as u64);
    for entry in &manifest.entries {
        check_cancelled(cancelled)?;
        let path = stage.path().join(&entry.path);
        let mut output = private_file(&path)?;
        let mut hash = Sha256::new();
        let mut remaining = entry.size;
        let mut buffer = [0_u8; BUFFER_BYTES];
        while remaining != 0 {
            check_cancelled(cancelled)?;
            let count = remaining.min(buffer.len() as u64) as usize;
            input
                .read_exact(&mut buffer[..count])
                .map_err(|e| e.to_string())?;
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
            crate::operation_observation::advance(count as u64);
        }
        if format!("{:x}", hash.finalize()) != entry.sha256 {
            return Err("バックアップの管理ファイルハッシュが一致しません".into());
        }
        output.sync_all().map_err(|e| e.to_string())?;
        if let Some(id) = &entry.asset_id {
            assets.insert(id.clone(), path);
        }
    }
    if !same_file(
        &before,
        &crate::native_fs::metadata(&input).map_err(|e| e.to_string())?,
    ) {
        return Err("検証中にバックアップが変更されています".into());
    }
    if manifest.entries[0].size > contents::MAX_BYTES {
        return Err("会議バックアップの内容が大きすぎます".into());
    }
    let contents: contents::Contents = serde_json::from_reader(
        File::open(stage.path().join("contents.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    contents.verify_audio(&assets, cancelled)?;
    let lookup = |_meeting: &str, id: &str| -> Result<String, String> {
        manifest
            .entries
            .iter()
            .find(|e| e.asset_id.as_deref() == Some(id))
            .map(|e| e.path.clone())
            .or_else(|| {
                manifest
                    .omitted_assets
                    .iter()
                    .find(|e| e.asset_id == id)
                    .map(|e| e.path.clone())
            })
            .ok_or("会議内容の資産がありません".into())
    };
    let database = contents.write_database(
        &stage.path().join("validation.sqlite3"),
        &|_, _, id| Ok(id.to_owned()),
        &lookup,
        cancelled,
    )?;
    manifest.tables = portable::tables(&database)?;
    let deleted_audio = deleted_audio::verify(
        &database,
        &manifest.deleted_audio,
        &manifest.omitted_assets,
        cancelled,
    )?;
    validate_inventory(&database, &manifest, &deleted_audio)?;

    database
        .execute_batch("PRAGMA query_only=ON")
        .map_err(|e| e.to_string())?;
    let database_proof = digest_file(&stage.path().join("validation.sqlite3"), cancelled)?;
    let database_identity =
        crate::native_fs::symlink_metadata(stage.path().join("validation.sqlite3"))
            .map_err(|e| e.to_string())?;
    Ok(VerifiedArchive {
        database,
        contents,
        manifest,
        assets,
        deleted_audio,
        database_proof,
        database_identity,
        stage,
    })
}

fn validate_inventory(
    db: &Connection,
    m: &Manifest,
    deleted: &deleted_audio::Verified,
) -> Result<(), String> {
    if read_meetings(db)? != m.meetings {
        return Err("バックアップの会議一覧とDBが一致しません".into());
    }
    let mut statement = db
        .prepare(
            "SELECT id,meeting_id,relative_path,size_bytes,content_hash,state,media_type FROM file_assets",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let entries: BTreeMap<_, _> = m
        .entries
        .iter()
        .filter_map(|e| e.asset_id.as_ref().map(|id| (id, e)))
        .collect();
    let omitted: BTreeMap<_, _> = m.omitted_assets.iter().map(|e| (&e.asset_id, e)).collect();
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let id: String = row.get(0).map_err(|e| e.to_string())?;
        let meeting: String = row.get(1).map_err(|e| e.to_string())?;
        let path: String = row.get(2).map_err(|e| e.to_string())?;
        let size: i64 = row.get(3).map_err(|e| e.to_string())?;
        let hash: String = row.get(4).map_err(|e| e.to_string())?;
        let state: String = row.get(5).map_err(|e| e.to_string())?;
        let media: String = row.get(6).map_err(|e| e.to_string())?;
        if let Some(e) = entries.get(&id) {
            if e.meeting_id.as_deref() != Some(&meeting)
                || e.path != path
                || e.source_size != Some(size as u64)
                || e.source_sha256.as_deref() != Some(&hash)
                || state != "ready"
            {
                return Err("管理ファイルmanifestとDBが一致しません".into());
            }
        } else if let Some(e) = omitted.get(&id) {
            if e.meeting_id != meeting
                || e.path != path
                || e.source_size != size as u64
                || e.source_sha256 != hash
                || state != "missing"
                || !deleted.contains(&meeting)
                || !deleted_audio::audio_media(&media)
            {
                return Err("除外ファイルの参照が不正です".into());
            }
        } else {
            return Err("manifestにない管理ファイルがあります".into());
        }
        count += 1;
    }
    {
        let mut q=db.prepare("SELECT id,relative_path,size_bytes,content_hash,frame_count,state FROM voice_sample_audio").map_err(|e|e.to_string())?;
        let mut rows = q.query([]).map_err(|e| e.to_string())?;
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let state: String = r.get(5).map_err(|e| e.to_string())?;
            if state == "unavailable" {
                continue;
            }
            if state != "ready" {
                return Err("未保存の登録音声がバックアップにあります".into());
            }
            let id: String = r.get(0).map_err(|e| e.to_string())?;
            let e = entries
                .get(&id)
                .ok_or("登録音声がバックアップにありません")?;
            let path: String = r.get(1).map_err(|e| e.to_string())?;
            let size: i64 = r.get(2).map_err(|e| e.to_string())?;
            let hash: String = r.get(3).map_err(|e| e.to_string())?;
            let frames: i64 = r.get(4).map_err(|e| e.to_string())?;
            if e.meeting_id.is_some()
                || e.path != path
                || e.source_size != Some(size as u64)
                || e.source_sha256.as_deref() != Some(&hash)
                || frames <= 0
                || Some(size) != frames.checked_mul(2).and_then(|n| n.checked_add(44))
            {
                return Err("登録音声のバックアップ情報が一致しません".into());
            }
            count += 1;
        }
    }
    if count != entries.len() + omitted.len() {
        return Err("管理ファイルの件数が一致しません".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "archive_tests.rs"]
pub(super) mod tests;

#[cfg(all(test, unix))]
#[path = "deleted_audio_tests.rs"]
pub(super) mod deleted_audio_tests;
