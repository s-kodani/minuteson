//! Complete reconstruction, verify it, then close and reopen read-only.
//! The resulting capability is not live publication or scheduler permission.
use super::super::super::restore_ids::RestoreIds;
use super::super::super::*;
#[cfg(test)]
use super::super::FixedData;
use crate::storage::vocabulary::GlobalMerge;

pub(in crate::storage::backup) struct SealedWorkspace {
    database: Connection,
    stage: scratch::Stage,
    database_proof: (u64, String),
    identity_proof: (String, String, i64),
    batch_id: String,
    archive_id: String,
    required_bytes: u64,
}

fn schema(db: &Connection) -> Result<Vec<(String, String, String)>, String> {
    db.prepare("SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' AND sql IS NOT NULL ORDER BY type,name")
        .map_err(|e|e.to_string())?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?
        .collect::<Result<_,_>>().map_err(|e|e.to_string())
}

fn uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

fn root(stage: &scratch::Stage) -> Result<PathBuf, String> {
    let metadata = crate::native_fs::symlink_metadata(stage.path()).map_err(|e| e.to_string())?;
    if !metadata.is_dir() {
        return Err("復元専用領域が変更されています".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("復元専用領域の非公開権限が変更されています".into());
        }
    }
    stage.path().canonicalize().map_err(|e| e.to_string())
}

fn regular(path: &Path) -> Result<(), String> {
    let metadata = crate::native_fs::symlink_metadata(path)
        .map_err(|e| format!("復元ファイル確認 {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err("復元管理対象は通常ファイルに限ります".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 {
            return Err("復元管理対象に外部とのハードリンクがあります".into());
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("復元管理対象の非公開権限が変更されています".into());
        }
    }
    Ok(())
}

fn asset_path(root: &Path, id: &str, meeting: &str, relative: &str) -> Result<PathBuf, String> {
    if !uuid(id) || !uuid(meeting) || relative != format!("meetings/{meeting}/restored/{id}") {
        return Err("復元管理ファイルの会議・パスが不正です".into());
    }
    let mut directory = root.to_path_buf();
    for component in ["meetings", meeting, "restored"] {
        directory.push(component);
        if !crate::native_fs::symlink_metadata(&directory)
            .map_err(|e| format!("復元親フォルダー {}: {e}", directory.display()))?
            .is_dir()
            || directory.canonicalize().map_err(|e| e.to_string())? != directory
        {
            return Err("復元管理ファイルの親パスが変更されています".into());
        }
    }
    Ok(crate::native_fs::join(root, relative))
}

fn verify_assets(db: &Connection, root: &Path, check: &impl Fn() -> bool) -> Result<u64, String> {
    let mut total = 0_u64;
    let mut query=db.prepare("SELECT id,meeting_id,relative_path,state,size_bytes,content_hash FROM file_assets ORDER BY id").map_err(|e|e.to_string())?;
    let mut rows = query.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(check)?;
        let id: String = row.get(0).map_err(|e| e.to_string())?;
        let meeting: String = row.get(1).map_err(|e| e.to_string())?;
        let relative: String = row.get(2).map_err(|e| e.to_string())?;
        let state: String = row.get(3).map_err(|e| e.to_string())?;
        let size: i64 = row.get(4).map_err(|e| e.to_string())?;
        let hash: String = row.get(5).map_err(|e| e.to_string())?;
        if size < 0
            || !portable::valid_hash(&hash)
            || !uuid(&id)
            || !uuid(&meeting)
            || relative != format!("meetings/{meeting}/restored/{id}")
        {
            return Err("復元管理ファイルの登録情報が不正です".into());
        }
        if state == "missing" {
            // An omitted asset can have no directory at all. Never follow a
            // substituted parent/leaf even when testing its absence.
            let mut path = root.to_path_buf();
            for component in Path::new(&relative).components() {
                path.push(component);
                match crate::native_fs::symlink_metadata(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(e) => return Err(e.to_string()),
                    Ok(info)
                        if info.is_dir() && path != crate::native_fs::join(root, &relative) => {}
                    Ok(_) => return Err("除外されたPCM・ログの保存先に追加物があります".into()),
                }
            }
        } else if state == "ready" {
            let path = asset_path(root, &id, &meeting, &relative)?;
            regular(&path)?;
            if digest_file(&path, check)? != (size as u64, hash) {
                return Err("復元管理ファイルのSHA・容量が一致しません".into());
            }
            total = total
                .checked_add(size as u64)
                .ok_or("復元管理ファイル容量が範囲外です")?;
        } else {
            return Err("復元管理ファイルに未完了・破損・削除待ちの状態があります".into());
        }
    }
    for asset in crate::storage::voice::audio::assets(db)? {
        check_cancelled(check)?;
        let person: String = db
            .query_row(
                "SELECT person_id FROM voice_samples WHERE id=?1",
                [&asset.id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if crate::storage::voice::audio::relative(&person, &asset.id)? != asset.relative_path {
            return Err("復元登録音声の所属が不正です".into());
        }
        let (dir, leaf) = crate::storage::voice::audio::file(root, &asset.relative_path, false)?;
        regular(&dir.path.join(&leaf))?;
        if digest_file(&dir.path.join(&leaf), check)?
            != (asset.size_bytes as u64, asset.content_hash)
        {
            return Err("復元登録音声のSHA・容量が一致しません".into());
        }
        total = total
            .checked_add(asset.size_bytes as u64)
            .ok_or("登録音声容量が範囲外です")?;
    }
    Ok(total)
}

fn inventory(
    db: &Connection,
    root: &Path,
    dir: &Path,
    depth: usize,
    check: &impl Fn() -> bool,
) -> Result<(), String> {
    for item in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        check_cancelled(check)?;
        let item = item.map_err(|e| e.to_string())?;
        let path = item.path();
        let kind = item.file_type().map_err(|e| e.to_string())?;
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("復元ファイル名が不正です")?
            .replace('\\', "/");
        if kind.is_file() {
            if relative != "fixed.sqlite3" {
                let known:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE relative_path=?1 AND state='ready') OR EXISTS(SELECT 1 FROM voice_sample_audio WHERE relative_path=?1 AND state='ready')",params![relative],|r|r.get(0)).map_err(|e|e.to_string())?;
                if !known {
                    return Err("復元専用領域に未登録ファイルがあります".into());
                }
            }
        } else if kind.is_dir() {
            let allowed = match depth {
                0 => relative == "meetings" || relative == "voices",
                1 => {
                    let meeting = item
                        .file_name()
                        .to_str()
                        .map(str::to_owned)
                        .ok_or("復元会議フォルダー名が不正です")?;
                    if relative.starts_with("voices/") {
                        uuid(&meeting) && db.query_row("SELECT EXISTS(SELECT 1 FROM voice_samples s JOIN voice_sample_audio a ON a.id=s.id WHERE s.person_id=?1 AND a.state='ready')",[&meeting],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())?
                    } else {
                        uuid(&meeting)
                            && db
                                .query_row(
                                    "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                                    params![meeting],
                                    |r| r.get::<_, bool>(0),
                                )
                                .map_err(|e| e.to_string())?
                    }
                }
                2 => item.file_name() == "restored",
                _ => false,
            };
            if !allowed || path.canonicalize().map_err(|e| e.to_string())? != path {
                return Err("復元専用領域に未対応のフォルダーがあります".into());
            }
            inventory(db, root, &path, depth + 1, check)?;
        } else {
            return Err("復元専用領域にリンクまたは特殊ファイルがあります".into());
        }
    }
    Ok(())
}

impl SealedWorkspace {
    pub(in crate::storage::backup) fn prepare_current(
        ids: &RestoreIds<'_>,
        parent: &Path,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        ids.source().verify_validation_database(check)?;
        let stage = archive::private_stage(parent)?;
        let image = stage.path().join("fixed.sqlite3");
        let map = |domain: &str, meeting: &str, key: &str| ids.get(domain, meeting, key);
        let paths = |meeting: &str, asset: &str| -> Result<String, String> {
            if meeting.is_empty() {
                let person: String = ids
                    .source()
                    .database
                    .query_row(
                        "SELECT person_id FROM voice_samples WHERE id=?1",
                        [asset],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                crate::storage::voice::audio::relative(
                    &ids.get("voice_people", "", &person)?,
                    &ids.get("voice_samples", "", asset)?,
                )
            } else {
                Ok(format!(
                    "meetings/{}/restored/{}",
                    ids.get("meetings", "", meeting)?,
                    ids.get("file_assets", meeting, asset)?
                ))
            }
        };
        let database = ids
            .source()
            .contents
            .write_database(&image, &map, &paths, check)?;
        ids.visit_mappings(check, |m| {
            if !super::super::super::ledger::valid_domain(&m.domain) {return Ok(());}
            let meeting=if m.domain=="meetings" {Some(m.target.clone())} else if m.scope.is_empty() {None} else {Some(ids.get("meetings","",&m.scope)?)};
            database.execute("INSERT INTO restore_provenance(id,meeting_id,domain,entity_key,source_archive,source_scope,source_key,restore_batch,restored_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![Uuid::new_v4().to_string(),meeting,m.domain,m.target,ids.archive_id,m.scope,m.source,ids.batch_id,now_ms()?]).map_err(|e|e.to_string())?;
            Ok(())
        })?;
        let total = ids
            .source()
            .manifest
            .entries
            .iter()
            .filter(|e| e.asset_id.is_some())
            .try_fold(0u64, |n, e| {
                n.checked_add(e.size).ok_or("復元音声容量が範囲外です")
            })?;
        crate::operation_observation::stage("copying_restore", Some(total), "bytes");
        for entry in ids
            .source()
            .manifest
            .entries
            .iter()
            .filter(|e| e.asset_id.is_some())
        {
            check_cancelled(check)?;
            let old = entry
                .asset_id
                .as_deref()
                .ok_or("管理ファイルのIDがありません")?;
            let meeting = entry.meeting_id.as_deref().unwrap_or("");
            let path = paths(meeting, old)?;
            let target = crate::native_fs::join(stage.path(), &path);
            scratch::directories(target.parent().ok_or("復元先がありません")?)?;
            copy_asset_observed(
                &ids.source().owned_asset_path(old)?,
                &target,
                &FileAsset {
                    id: old.into(),
                    meeting_id: meeting.into(),
                    relative_path: path,
                    media_type: String::new(),
                    size_bytes: i64::try_from(entry.size).map_err(|_| "音声容量が範囲外です")?,
                    content_hash: entry.sha256.clone(),
                    state: "ready".into(),
                },
                check,
            )?;
        }
        crate::operation_observation::report("copying_restore", total, total, "bytes");
        crate::operation_observation::stage("checking_restore", None, "bytes");
        for entry in &ids.source().manifest.omitted_assets {
            let target =
                crate::native_fs::join(stage.path(), &paths(&entry.meeting_id, &entry.asset_id)?);
            scratch::directories(target.parent().ok_or("復元先がありません")?)?;
        }
        portable::validate_scopes(&database)?;
        super::super::super::ledger::validate_provenance(&database, check)?;
        let owned = stage.path().canonicalize().map_err(|e| e.to_string())?;
        let bytes = verify_assets(&database, &owned, check)?;
        inventory(&database, &owned, &owned, 0, check)?;
        database.close().map_err(|(_, e)| e.to_string())?;
        let proof = digest_file(&image, check)?;
        let database = read_only(&image)?;
        let required_bytes = proof
            .0
            .checked_mul(2)
            .and_then(|n| n.checked_add(bytes))
            .ok_or("復元容量が範囲外です")?;
        let result = Self {
            database,
            stage,
            database_proof: proof,
            identity_proof: super::super::super::ledger::BatchPlan::new(ids, check)?.proof(),
            batch_id: ids.batch_id.clone(),
            archive_id: ids.archive_id.clone(),
            required_bytes,
        };
        result.verify(check)?;
        Ok(result)
    }

    pub(in crate::storage::backup) fn matches_ids(
        &self,
        ids: &RestoreIds<'_>,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if self.batch_id != ids.batch_id
            || self.archive_id != ids.archive_id
            || self.identity_proof
                != super::super::super::ledger::BatchPlan::new(ids, check)?.proof()
        {
            return Err("復元専用領域と公開するID対応が一致しません".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::storage::backup) fn prepare(
        fixed: &FixedData<'_, '_>,
        parent: &Path,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::prepare_current(fixed.ids, parent, check)
    }

    pub(in crate::storage::backup) fn verify(
        &self,
        check: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check_cancelled(check)?;
        let owned = root(&self.stage)?;
        let image = owned.join("fixed.sqlite3");
        regular(&image)?;
        if digest_file(&image, check)? != self.database_proof {
            return Err("検証済み復元DBが変更されています".into());
        }
        verify_assets(&self.database, &owned, check)?;
        inventory(&self.database, &owned, &owned, 0, check)?;
        if digest_file(&image, check)? != self.database_proof {
            return Err("復元再検査中にDBが変更されています".into());
        }
        Ok(())
    }

    pub(in crate::storage::backup) fn global_merge(
        &self,
        destination: &Connection,
        check: &impl Fn() -> bool,
    ) -> Result<GlobalMerge, String> {
        self.verify(check)?;
        let plan = GlobalMerge::plan(&self.database, destination, check)?;
        self.verify(check)?;
        Ok(plan)
    }

    pub(in crate::storage::backup) fn required_bytes(&self) -> u64 {
        self.required_bytes
    }

    pub(in crate::storage::backup) fn connection(&self) -> &Connection {
        &self.database
    }

    pub(in crate::storage::backup) fn root_path(&self) -> Result<PathBuf, String> {
        root(&self.stage)
    }
}
