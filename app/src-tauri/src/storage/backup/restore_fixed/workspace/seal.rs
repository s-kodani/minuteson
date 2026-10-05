//! Complete reconstruction, verify it, then close and reopen read-only.
//! The resulting capability is not live publication or scheduler permission.
use super::*;
use crate::storage::vocabulary::GlobalMerge;
use std::collections::BTreeSet;

pub(in crate::storage::backup) struct SealedWorkspace {
    database: Connection,
    stage: tempfile::TempDir,
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

fn root(stage: &tempfile::TempDir) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(stage.path()).map_err(|e| e.to_string())?;
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
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
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
        if !std::fs::symlink_metadata(&directory)
            .map_err(|e| e.to_string())?
            .is_dir()
            || directory.canonicalize().map_err(|e| e.to_string())? != directory
        {
            return Err("復元管理ファイルの親パスが変更されています".into());
        }
    }
    Ok(root.join(relative))
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
                match std::fs::symlink_metadata(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(e) => return Err(e.to_string()),
                    Ok(info) if info.is_dir() && path != root.join(&relative) => {}
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
            .ok_or("復元ファイル名が不正です")?;
        if kind.is_file() {
            if relative != "fixed.sqlite3" {
                let known:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM file_assets WHERE relative_path=?1 AND state='ready')",params![relative],|r|r.get(0)).map_err(|e|e.to_string())?;
                if !known {
                    return Err("復元専用領域に未登録ファイルがあります".into());
                }
            }
        } else if kind.is_dir() {
            let allowed = match depth {
                0 => relative == "meetings",
                1 => {
                    let meeting = item
                        .file_name()
                        .to_str()
                        .map(str::to_owned)
                        .ok_or("復元会議フォルダー名が不正です")?;
                    uuid(&meeting)
                        && db
                            .query_row(
                                "SELECT EXISTS(SELECT 1 FROM meetings WHERE id=?1)",
                                params![meeting],
                                |r| r.get::<_, bool>(0),
                            )
                            .map_err(|e| e.to_string())?
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

impl FixedWorkspace {
    fn seal(
        mut self,
        fixed: &FixedData<'_, '_>,
        check: &impl Fn() -> bool,
    ) -> Result<SealedWorkspace, String> {
        check_cancelled(check)?;
        if self.completed != [true; 4] {
            return Err("復元の全構築段階が完了していません".into());
        }
        let owned = root(&self.stage)?;
        let image = owned.join("fixed.sqlite3");
        regular(&image)?;
        if self.final_database_proof.as_ref() != Some(&digest_file(&image, check)?) {
            return Err("構築完了後の復元DBが変更されています".into());
        }
        if !self.database.is_autocommit() {
            return Err("復元DBに未完了のトランザクションがあります".into());
        }
        portable::validate_scopes(&self.database)?;
        validate_database(&mut self.database, check)?;
        for source in &fixed.ids.source().manifest.tables {
            if source.name == "restore_provenance" {
                continue;
            }
            let count: i64 = self
                .database
                .query_row(
                    &format!("SELECT count(*) FROM \"{}\"", source.name),
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if u64::try_from(count).ok() != Some(source.row_count) {
                return Err("復元DBの構築件数が元アーカイブと一致しません".into());
            }
        }
        for table in [
            "app_settings",
            "restore_batches",
            "restore_identity_maps",
            "model_packages",
            "restore_omitted_assets",
            "model_files",
            "model_profiles",
            "profile_components",
            "model_catalog_history",
            "maintenance_operations",
            "maintenance_paths",
        ] {
            if self
                .database
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(|e| e.to_string())?
                != 0
            {
                return Err("復元専用DBに端末設定または端末内バッチがあります".into());
            }
        }
        let active:bool=self.database.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('queued','running','pausing','paused')) OR EXISTS(SELECT 1 FROM analysis_flows WHERE stage IN ('pending','transcription','diarization')) OR EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping')) OR EXISTS(SELECT 1 FROM audio_imports WHERE state IN ('copying','converting')) OR EXISTS(SELECT 1 FROM export_records WHERE state='writing') OR EXISTS(SELECT 1 FROM audio_tracks WHERE state='writing') OR EXISTS(SELECT 1 FROM analysis_revisions WHERE publication_state='building') OR EXISTS(SELECT 1 FROM minutes_revisions WHERE publication_state='building')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        if active {
            return Err("復元データに自動開始可能・書込み中の処理があります".into());
        }
        let expected: BTreeSet<_> = fixed
            .ids
            .source()
            .manifest
            .omitted_assets
            .iter()
            .map(|asset| {
                fixed
                    .ids
                    .get("file_assets", &asset.meeting_id, &asset.asset_id)
            })
            .collect::<Result<_, _>>()?;
        let actual: BTreeSet<String> = self
            .database
            .prepare("SELECT id FROM file_assets WHERE state='missing'")
            .map_err(|e| e.to_string())?
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        if actual != expected || self.missing_assets != expected {
            return Err("復元データの欠損成果物が除外記録と一致しません".into());
        }
        let bytes = verify_assets(&self.database, &owned, check)?;
        inventory(&self.database, &owned, &owned, 0, check)?;
        if self
            .database
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='trigger'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())?
            != 0
        {
            return Err("未検証の復元DBに追加トリガーがあります".into());
        }
        let tx = self.database.transaction().map_err(|e| e.to_string())?;
        for trigger in &self.triggers {
            check_cancelled(check)?;
            tx.execute_batch(trigger).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        let trusted = portable::schema(MIGRATIONS.len())?;
        if schema(&self.database)? != schema(&trusted)? {
            return Err("復元DBの表・索引・トリガーが信頼済み定義と一致しません".into());
        }
        check_capacity(&owned, 0)?;
        let batch_id = fixed.ids.batch_id.clone();
        let archive_id = fixed.ids.archive_id.clone();
        let identity_proof = super::super::super::ledger::BatchPlan::new(fixed.ids, check)?.proof();
        let FixedWorkspace {
            database, stage, ..
        } = self;
        database.close().map_err(|(_, e)| e.to_string())?;
        let proof = digest_file(&image, check)?;
        let database = read_only(&image)?;
        let required_bytes = proof
            .0
            .checked_mul(2)
            .and_then(|n| n.checked_add(bytes))
            .ok_or("復元容量が範囲外です")?;
        let sealed = SealedWorkspace {
            database,
            stage,
            database_proof: proof,
            identity_proof,
            batch_id,
            archive_id,
            required_bytes,
        };
        sealed.verify(check)?;
        Ok(sealed)
    }
}

impl SealedWorkspace {
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

    pub(in crate::storage::backup) fn prepare(
        fixed: &FixedData<'_, '_>,
        parent: &Path,
        check: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        FixedWorkspace::prepare(fixed, parent, check)?
            .with_speech_artifacts(fixed, check)?
            .with_generation_artifacts(fixed, check)?
            .with_processing_history(fixed, check)?
            .with_provenance(fixed, check)?
            .seal(fixed, check)
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

#[cfg(test)]
#[path = "seal_tests.rs"]
mod tests;
