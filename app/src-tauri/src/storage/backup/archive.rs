//! Framed, uncompressed, hashed portable data; no SQL/ZIP/host paths to execute.
use super::portable::{self, FileReference, Table};
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
    PortableJson,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OmittedReason {
    RecreatablePcm,
    NativeLog,
    SourceAudioDeleted,
    SupersededAudio,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) voice_matching_preferences:
        Option<crate::storage::voice_preferences::VoiceMatchingPreferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) speaker_completion_preferences:
        Option<crate::storage::completion_preferences::CompletionPreferences>,
    pub format_version: u32,
    pub schema_version: usize,
    pub archive_id: String,
    pub created_at: i64,
    encryption: Encryption,
    hash_basis: HashBasis,
    pub meetings: Vec<Meeting>,
    pub(super) tables: Vec<Table>,
    pub(super) entries: Vec<Entry>,
    pub(super) omitted_assets: Vec<OmittedAsset>,
    pub(super) file_references: Vec<FileReference>,
    pub(super) model_catalog: crate::storage::model_profiles::portable::history::Bundle,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) deleted_audio: Vec<deleted_audio::Descriptor>,
}

pub(crate) struct VerifiedArchive {
    // This is a validation/source-ID DB, NOT a replacement or a runnable app DB.
    // Connection closes before the private directory is removed on all platforms.
    pub(super) database: Connection,
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
    pub omitted_pcm_count: usize,
    pub omitted_log_count: usize,
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
        let db = read_only(&self.database)?;
        let known = crate::storage::model_metadata::retained_files(&db)?;
        let model_catalog =
            crate::storage::model_profiles::portable::history::Bundle::export(&db, cancelled)?;
        let mut references = model_catalog.file_references()?;
        let mut lookup = BTreeMap::new();
        let mut sources = Vec::new();
        let mut entries = Vec::new();
        let mut omitted = Vec::new();
        for item in &self.assets {
            check_cancelled(cancelled)?;
            let asset = &item.asset;
            let reason = if self.deleted_audio.contains_asset(asset) {
                Some(OmittedReason::SourceAudioDeleted)
            } else if super::super::meeting_audio::superseded_asset(&db, &asset.id)? {
                Some(OmittedReason::SupersededAudio)
            } else if portable::temporary_pcm(&db, asset)? {
                Some(OmittedReason::RecreatablePcm)
            } else if portable::native_log(&db, asset)? {
                Some(OmittedReason::NativeLog)
            } else {
                None
            };
            let temporary = reason.is_some();
            let key = format!(
                "{}/{}",
                if temporary { "omitted" } else { "assets" },
                item.key
            );
            lookup.insert(asset.id.clone(), (key.clone(), temporary));
            if let Some(reason) = reason {
                omitted.push(OmittedAsset {
                    asset_id: asset.id.clone(),
                    meeting_id: asset.meeting_id.clone(),
                    path: key,
                    source_size: asset.size_bytes as u64,
                    source_sha256: asset.content_hash.clone(),
                    reason,
                });
                continue;
            }
            let is_json = asset.media_type == "application/json"
                || asset.media_type == "application/vnd.mon.llama-completion";
            let (source, transform) = if is_json {
                let capture: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE meeting_id=?1 AND audio_manifest_hash=?2)",params![asset.meeting_id,asset.content_hash],|r|r.get(0)).map_err(|e| e.to_string())?;
                let (mut file, metadata) = open_source(&item.path)?;
                if metadata.len() > portable::MAX_ROW_BYTES as u64 {
                    return Err("管理JSONがバックアップ上限を超えています".into());
                }
                let mut bytes = Vec::new();
                Read::by_ref(&mut file)
                    .take(portable::MAX_ROW_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() as u64 != metadata.len()
                    || format!("{:x}", Sha256::digest(&bytes)) != asset.content_hash
                {
                    return Err("管理JSONが変更されています".into());
                }
                let bytes = if asset.media_type == "application/vnd.mon.llama-completion" {
                    portable::portable_completion(&bytes, &mut references)?
                } else {
                    portable::portable_json_with_metadata(&bytes, capture, &mut references, &known)?
                };
                let path = stage.path().join(&item.key);
                let mut out = private_file(&path)?;
                out.write_all(&bytes)
                    .and_then(|_| out.sync_all())
                    .map_err(|e| e.to_string())?;
                (path, Transform::PortableJson)
            } else {
                (item.path.clone(), Transform::Identity)
            };
            let (size, sha256) = digest_file(&source, cancelled)?;
            entries.push(Entry {
                path: key,
                size,
                sha256,
                asset_id: Some(asset.id.clone()),
                meeting_id: (!asset.meeting_id.is_empty()).then(||asset.meeting_id.clone()),
                source_size: Some(asset.size_bytes as u64),
                source_sha256: Some(asset.content_hash.clone()),
                transform,
            });
            sources.push(source);
        }
        let data = stage.path().join("database.jsonl");
        let tables = portable::dump(self, &data, &lookup, &mut references, cancelled)?;
        let deleted_audio = if self.deleted_audio.is_empty() {
            Vec::new()
        } else {
            let logical = portable::load(
                &data,
                &stage.path().join("availability.sqlite3"),
                &tables,
                MIGRATIONS.len(),
                cancelled,
            )?;
            let descriptors = self.deleted_audio.bind(&logical, cancelled)?;
            deleted_audio::verify(&logical, &descriptors, &omitted, cancelled)?;
            descriptors
        };
        if digest_file(&self.database, cancelled)?
            != (self.database_size, self.database_hash.clone())
        {
            return Err("論理データ取得中にスナップショットDBが変更されています".into());
        }
        let (size, sha256) = digest_file(&data, cancelled)?;
        entries.insert(
            0,
            Entry {
                path: "database.jsonl".into(),
                size,
                sha256,
                asset_id: None,
                meeting_id: None,
                source_size: None,
                source_sha256: None,
                transform: Transform::Identity,
            },
        );
        sources.insert(0, data);
        let has_preferences: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM app_settings WHERE key='voice_matching')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let manifest = Manifest {
            speaker_completion_preferences: Some(
                crate::storage::completion_preferences::load(&db)?.payload,
            ),
            voice_matching_preferences: if has_preferences {
                Some(crate::storage::voice_preferences::load(&db)?.payload)
            } else {
                None
            },
            format_version: crate::current_formats::BACKUP,
            schema_version: MIGRATIONS.len(),
            archive_id: Uuid::new_v4().to_string(),
            created_at: now_ms()?,
            encryption: Encryption::None,
            hash_basis: HashBasis::SourceIdentity,
            meetings: self.meetings.clone(),
            tables,
            entries,
            omitted_assets: omitted,
            file_references: references.into_iter().collect(),
            model_catalog,
            deleted_audio,
        };
        validate_manifest(&manifest)?;
        let bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_MANIFEST {
            return Err("バックアップmanifestが大きすぎます".into());
        }
        let required =
            manifest
                .entries
                .iter()
                .try_fold(HEADER_BYTES + bytes.len() as u64, |sum, e| {
                    sum.checked_add(e.size)
                        .ok_or("バックアップ容量が範囲外です")
                })?;
        check_capacity(parent, required)?;
        // Do not publish a snapshot whose deliberately absent source has
        // reappeared while the portable data was being reconstructed.
        self.verify(cancelled)?;
        output
            .write_all(MAGIC)
            .and_then(|_| output.write_all(&(bytes.len() as u32).to_le_bytes()))
            .and_then(|_| output.write_all(&Sha256::digest(&bytes)))
            .and_then(|_| output.write_all(&bytes))
            .map_err(|e| e.to_string())?;
        for (entry, path) in manifest.entries.iter().zip(sources) {
            let (mut source, before) = open_source(&path)?;
            let (_, hash) = stream(&mut source, Some(output), entry.size, cancelled)?;
            if hash != entry.sha256
                || !same_file(
                    &before,
                    &crate::native_fs::metadata(&source).map_err(|e| e.to_string())?,
                )
            {
                return Err("バックアップ作成中に入力が変更されています".into());
            }
        }
        output.sync_all().map_err(|e| e.to_string())?;
        output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let verified = read(output, stage.path(), cancelled)?;
        let size = crate::native_fs::metadata(&output)
            .map_err(|e| e.to_string())?
            .len();
        output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let (_, sha256) = stream(output, None, size, cancelled)?;
        self.verify(cancelled)?;
        Ok(BackupReceipt {
            archive_id: verified.manifest.archive_id.clone(),
            meeting_count: verified.manifest.meetings.len(),
            asset_count: verified.assets.len(),
            omitted_pcm_count: verified
                .manifest
                .omitted_assets
                .iter()
                .filter(|a| a.reason == OmittedReason::RecreatablePcm)
                .count(),
            omitted_log_count: verified
                .manifest
                .omitted_assets
                .iter()
                .filter(|a| a.reason == OmittedReason::NativeLog)
                .count(),
            deleted_audio_meeting_count: verified.manifest.deleted_audio.len(),
            deleted_audio_asset_count: verified
                .manifest
                .omitted_assets
                .iter()
                .filter(|a| a.reason == OmittedReason::SourceAudioDeleted)
                .count(),
            size,
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
    if let Some(p) = m.speaker_completion_preferences {
        p.validate()?;
    }
    if let Some(p) = m.voice_matching_preferences {
        p.validate()?;
    }
    if !((m.format_version == 7 && m.schema_version == 38)
        || (m.format_version == 6 && matches!(m.schema_version, 36 | 37))
        || (m.format_version == 5 && m.schema_version == 35))
    {
        return Err("現在の形式ではないバックアップは復元できません".into());
    }
    if !Uuid::parse_str(&m.archive_id).is_ok_and(|id| id.to_string() == m.archive_id)
        || m.created_at < 0
        || m.entries.is_empty()
        || m.entries.len() + m.omitted_assets.len() > MAX_ENTRIES
        || m.deleted_audio.len() > deleted_audio::MAX_MEETINGS
    {
        return Err("バックアップの形式版・件数が不正です".into());
    }
    m.model_catalog.validate()?;
    let references: BTreeSet<_> = m.file_references.iter().cloned().collect();
    if !m.model_catalog.file_references()?.is_subset(&references) {
        return Err("モデル構成履歴の全ファイル情報がmanifestにありません".into());
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
            if e.path != "database.jsonl"
                || e.asset_id.is_some()
                || e.meeting_id.is_some()
                || e.source_size.is_some()
                || e.source_sha256.is_some()
                || e.transform != Transform::Identity
            {
                return Err("バックアップDB記録が不正です".into());
            }
        } else if !key(&e.path, "assets/")
            || e.asset_id
                .as_deref()
                .is_none_or(|id| !identifier(id) || !ids.insert(id))
            || e.meeting_id.as_ref().map_or(m.schema_version < 38, |id| !meetings.contains(id))
            || e.source_size.is_none()
            || e.source_sha256
                .as_deref()
                .is_none_or(|s| !portable::valid_hash(s))
            || (e.transform == Transform::Identity
                && (e.source_size != Some(e.size) || e.source_sha256.as_deref() != Some(&e.sha256)))
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
            || (e.reason == OmittedReason::SourceAudioDeleted
                && !deleted_meetings.contains(&e.meeting_id))
            || (e.reason == OmittedReason::RecreatablePcm
                && deleted_meetings.contains(&e.meeting_id))
        {
            return Err("除外ファイルの由来が不正です".into());
        }
    }
    let mut references = BTreeSet::new();
    for r in &m.file_references {
        r.validate()?;
        if !portable::valid_hash(&r.sha256)
            || !identifier(&r.role)
            || r.name.is_empty()
            || r.name.len() > 4096
            || r.name.contains(['/', '\\', '\0'])
            || !references.insert(r)
        {
            return Err("モデル・実行ファイルの記録が不正です".into());
        }
    }
    let rows = m.tables.iter().try_fold(0_u64, |sum, t| {
        sum.checked_add(t.row_count)
            .ok_or("バックアップ行数が範囲外です")
    })?;
    if rows > m.entries[0].size {
        return Err("バックアップ行数と容量が一致しません".into());
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
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
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
    let database = portable::load(
        &stage.path().join("database.jsonl"),
        &stage.path().join("validation.sqlite3"),
        &manifest.tables,
        manifest.schema_version,
        cancelled,
    )?;
    let deleted_audio = deleted_audio::verify(
        &database,
        &manifest.deleted_audio,
        &manifest.omitted_assets,
        cancelled,
    )?;
    validate_inventory(&database, &manifest, &deleted_audio)?;
    validate_portability(&database, &manifest, &assets, cancelled)?;
    database
        .execute_batch("PRAGMA query_only=ON")
        .map_err(|e| e.to_string())?;
    let database_proof = digest_file(&stage.path().join("validation.sqlite3"), cancelled)?;
    let database_identity =
        crate::native_fs::symlink_metadata(stage.path().join("validation.sqlite3"))
            .map_err(|e| e.to_string())?;
    Ok(VerifiedArchive {
        database,
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
            let referenced: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM audio_chunks WHERE file_asset_id=?1) OR EXISTS(SELECT 1 FROM audio_imports WHERE original_asset_id=?1 OR converted_asset_id=?1) OR EXISTS(SELECT 1 FROM audio_import_commits WHERE manifest_asset_id=?1)",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if e.meeting_id != meeting
                || e.path != path
                || e.source_size != size as u64
                || e.source_sha256 != hash
                || state != "missing"
                || (referenced
                    && !matches!(
                        e.reason,
                        OmittedReason::SourceAudioDeleted | OmittedReason::SupersededAudio
                    ))
                || match e.reason {
                    OmittedReason::RecreatablePcm => media != "audio/wav",
                    OmittedReason::NativeLog => media != "text/plain",
                    OmittedReason::SupersededAudio => {
                        !super::super::meeting_audio::superseded_asset(db, &id)?
                    }
                    OmittedReason::SourceAudioDeleted => {
                        !deleted.contains(&meeting) || !deleted_audio::audio_media(&media)
                    }
                }
            {
                return Err("除外ファイルの参照が不正です".into());
            }
        } else {
            return Err("manifestにない管理ファイルがあります".into());
        }
        count += 1;
    }
    if m.schema_version >= 38 {
        let mut q=db.prepare("SELECT id,relative_path,size_bytes,content_hash,frame_count,state FROM voice_sample_audio").map_err(|e|e.to_string())?;
        let mut rows=q.query([]).map_err(|e|e.to_string())?;
        while let Some(r)=rows.next().map_err(|e|e.to_string())? {
            let state:String=r.get(5).map_err(|e|e.to_string())?;
            if state == "unavailable" { continue; }
            if state != "ready" { return Err("未保存の登録音声がバックアップにあります".into()); }
            let id:String=r.get(0).map_err(|e|e.to_string())?;
            let e=entries.get(&id).ok_or("登録音声がバックアップにありません")?;
            let path:String=r.get(1).map_err(|e|e.to_string())?;
            let size:i64=r.get(2).map_err(|e|e.to_string())?;
            let hash:String=r.get(3).map_err(|e|e.to_string())?;
            let frames:i64=r.get(4).map_err(|e|e.to_string())?;
            if e.meeting_id.is_some() || e.path!=path || e.source_size!=Some(size as u64) || e.source_sha256.as_deref()!=Some(&hash) || frames<=0 || Some(size)!=frames.checked_mul(2).and_then(|n|n.checked_add(44)) {
                return Err("登録音声のバックアップ情報が一致しません".into());
            }
            count+=1;
        }
    }
    if count != entries.len() + omitted.len() {
        return Err("管理ファイルの件数が一致しません".into());
    }
    Ok(())
}

fn validate_portability(
    db: &Connection,
    manifest: &Manifest,
    assets: &BTreeMap<String, PathBuf>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let kept_log:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM job_checkpoints c JOIN processing_jobs j ON j.id=c.job_id, json_each(c.checkpoint_json,'$.outputs') o JOIN file_assets f ON f.id=json_extract(o.value,'$.stdoutAssetId') AND f.meeting_id=j.meeting_id WHERE j.kind='diarization' AND f.state!='missing')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if kept_log {
        return Err("バックアップに話者分離ログ本体が残っています".into());
    }
    let invalid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM audio_tracks WHERE device_label IS NOT NULL) OR EXISTS(SELECT 1 FROM export_records WHERE error_message IS NOT NULL) OR EXISTS(SELECT 1 FROM audio_import_commits WHERE error_message IS NOT NULL) OR EXISTS(SELECT 1 FROM analysis_flows WHERE error_message IS NOT NULL)",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if invalid {
        return Err("バックアップに端末固有の診断情報が残っています".into());
    }
    if manifest.schema_version>=36 && db.query_row("SELECT EXISTS(SELECT 1 FROM post_analysis_intents WHERE error_message IS NOT NULL)",[],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
        return Err("バックアップに後続処理の端末固有診断が残っています".into());
    }
    let mut statement = db
        .prepare("SELECT destination_path FROM export_records")
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let name: String = row.get(0).map_err(|e| e.to_string())?;
        if name.contains(['/', '\\']) {
            return Err("バックアップに元の出力先が残っています".into());
        }
    }
    let check_json = |bytes: &[u8], capture: bool| -> Result<(), String> {
        check_cancelled(cancelled)?;
        let original: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let mut references = BTreeSet::new();
        let normalized = portable::portable_json(bytes, capture, &mut references)?;
        let normalized: serde_json::Value =
            serde_json::from_slice(&normalized).map_err(|e| e.to_string())?;
        if original != normalized {
            return Err(
                "バックアップに端末固有のパス・機器指定・利用可能状態が残っています".into(),
            );
        }
        for reference in references {
            if !manifest
                .file_references
                .iter()
                .any(|r| r.sha256 == reference.sha256 && r.size == reference.size)
            {
                return Err("固定ファイルのメタデータがmanifestにありません".into());
            }
        }
        Ok(())
    };
    for sql in [
        "SELECT parameters_json FROM processing_jobs UNION ALL SELECT profile_snapshot_json FROM processing_jobs",
        "SELECT config_json FROM analysis_revisions UNION ALL SELECT profile_snapshot_json FROM analysis_revisions",
        "SELECT model_snapshot_json FROM minutes_revisions",
        "SELECT processing_config_json FROM audio_tracks WHERE processing_config_json IS NOT NULL",
        "SELECT capture_config_json FROM recording_sessions",
        "SELECT plan_json FROM analysis_flows WHERE plan_json IS NOT NULL",
        "SELECT checkpoint_json FROM job_checkpoints",
    ].into_iter().chain((manifest.schema_version>=36).then_some("SELECT spec_json FROM post_analysis_intents")) {
        let mut statement = db.prepare(sql).map_err(|e|e.to_string())?;
        let mut rows = statement.query([]).map_err(|e|e.to_string())?;
        while let Some(row) = rows.next().map_err(|e|e.to_string())? { check_json(row.get::<_,String>(0).map_err(|e|e.to_string())?.as_bytes(),false)?; }
    }
    for entry in &manifest.entries[1..] {
        let id = entry
            .asset_id
            .as_ref()
            .ok_or("管理ファイルのIDがありません")?;
        if entry.meeting_id.is_none() {
            if manifest.schema_version < 38 || entry.transform != Transform::Identity {
                return Err("登録音声の可搬化指定が不正です".into());
            }
            let frames:i64=db.query_row("SELECT frame_count FROM voice_sample_audio WHERE id=?1 AND state='ready'",[id],|r|r.get(0)).map_err(|e|e.to_string())?;
            let (file,_)=open_source(assets.get(id).ok_or("登録音声がありません")?)?;
            let audio=crate::audio::VerifiedAudio::open_cancellable(file,entry.size,&entry.sha256,cancelled)?;
            if !audio.format.is_voice_clip() || audio.format.frames!=frames as u64 {
                return Err("バックアップの登録音声の形式・長さが不正です".into());
            }
            continue;
        }
        let media_type: String = db
            .query_row(
                "SELECT media_type FROM file_assets WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let json = matches!(
            media_type.as_str(),
            "application/json" | "application/vnd.mon.llama-completion"
        );
        if json != (entry.transform == Transform::PortableJson) {
            return Err("管理JSONの可搬化指定が不正です".into());
        }
        if json {
            let capture: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM recording_sessions WHERE meeting_id=?1 AND audio_manifest_hash=?2)",params![entry.meeting_id,entry.source_sha256],|r|r.get(0)).map_err(|e|e.to_string())?;
            let (file, before) = open_source(assets.get(id).ok_or("管理JSONがありません")?)?;
            if before.len() > portable::MAX_ROW_BYTES as u64 {
                return Err("管理JSONが大きすぎます".into());
            }
            let mut bytes = Vec::new();
            file.take(portable::MAX_ROW_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            check_json(&bytes, capture)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "archive_tests.rs"]
pub(super) mod tests;

#[cfg(all(test, unix))]
#[path = "deleted_audio_tests.rs"]
mod deleted_audio_tests;
