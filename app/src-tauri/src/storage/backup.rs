//! Private consistent staging, NOT a distributable backup format. The temporary
//! SQLite image still contains machine-local settings. A portable exporter must
//! explicitly omit/redact those fields before publishing an external archive.
use super::*;
use rusqlite::backup::{Backup, StepResult};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::OpenFlags;
use std::fs::{File, Metadata, OpenOptions};
use std::io::Write;
use std::sync::atomic::Ordering;

pub(crate) mod archive;
pub(in crate::storage) mod ledger;
mod portable;
pub(in crate::storage::backup) mod publication;
pub(crate) mod restore_fixed;
pub(crate) mod restore_ids;

const BUFFER_BYTES: usize = 64 * 1024;
const CAPACITY_RESERVE: u64 = 16 * 1024 * 1024;

pub(in crate::storage) fn recover_restore_operations(
    root: &Path,
    db: &Connection,
) -> Result<(), String> {
    publication::journal::recover_all(root, db)
}

struct Maintenance<'a> {
    storage: &'a Storage,
    unresolved: std::cell::Cell<bool>,
}

impl Maintenance<'_> {
    fn fail_closed(&self) {
        self.unresolved.set(true);
    }
}

impl Drop for Maintenance<'_> {
    fn drop(&mut self) {
        // Even unwinding must remove the authorizer and release the operation.
        let db = self
            .storage
            .connection
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !self.unresolved.get()
            && db.is_autocommit()
            && db
                .set_db_config(
                    rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
                    true,
                )
                .is_ok()
            && db
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .is_ok()
        {
            self.storage.maintenance.store(false, Ordering::Release);
        } else {
            let _ = db.authorizer(Some(|_: AuthContext<'_>| Authorization::Deny));
        }
        // An unexpected hook failure stays fail-closed until Storage is reopened.
    }
}

pub(crate) struct Snapshot<'a> {
    // Drop stage before unlocking; only this operation's unpublished files go away.
    stage: tempfile::TempDir,
    pub(super) database: PathBuf,
    pub(super) database_hash: String,
    pub(super) database_size: u64,
    pub(super) assets: Vec<StagedAsset>,
    pub(super) meetings: Vec<Meeting>,
    _maintenance: Maintenance<'a>,
}

pub(super) struct StagedAsset {
    pub asset: FileAsset,
    // Opaque generated key, not a source relative path or a title.
    pub key: String,
    pub path: PathBuf,
}

impl Storage {
    fn backup_maintenance(&self) -> Result<Maintenance<'_>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        if self.maintenance.load(Ordering::Acquire) {
            return Err("保存領域の操作が既に進行中です".into());
        }
        let active: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM processing_jobs WHERE state IN ('running','pausing'))
             OR EXISTS(SELECT 1 FROM recording_sessions WHERE state IN ('preparing','recording','stopping'))
             OR EXISTS(SELECT 1 FROM audio_imports WHERE state IN ('copying','converting'))
             OR EXISTS(SELECT 1 FROM export_records WHERE state='writing')
             OR EXISTS(SELECT 1 FROM audio_tracks WHERE state='writing')
             OR EXISTS(SELECT 1 FROM analysis_revisions WHERE publication_state='building')
             OR EXISTS(SELECT 1 FROM minutes_revisions WHERE publication_state='building')",
            [], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if active {
            return Err(
                "録音・AI処理・取り込み・書き出しが停止してからバックアップしてください".into(),
            );
        }
        // All Storage statements are scoped to this mutex. Replacing the SQLite
        // authorizer invalidates prepared VMs too; flushing cached statements is
        // an additional safeguard for future prepare_cached call sites.
        db.flush_prepared_statement_cache();
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. }
            | AuthAction::Recursive => Authorization::Allow,
            AuthAction::Pragma {
                pragma_name,
                pragma_value: None,
            } if matches!(
                pragma_name,
                "foreign_keys"
                    | "user_version"
                    | "schema_version"
                    | "page_count"
                    | "page_size"
                    | "query_only"
            ) =>
            {
                Authorization::Allow
            }
            _ => Authorization::Deny,
        }))
        .map_err(|e| e.to_string())?;
        self.maintenance.store(true, Ordering::Release);
        Ok(Maintenance {
            storage: self,
            unresolved: std::cell::Cell::new(false),
        })
    }

    pub(crate) fn prepare_backup_snapshot(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Snapshot<'_>, String> {
        check_cancelled(cancelled)?;
        let maintenance = self.backup_maintenance()?;
        let mut builder = tempfile::Builder::new();
        builder.prefix("mon-backup-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let stage = builder.tempdir().map_err(|e| e.to_string())?;
        let database = stage.path().join("snapshot.sqlite3");
        // create_new/private permissions before SQLite opens the image.
        private_file(&database)?
            .sync_all()
            .map_err(|e| e.to_string())?;
        {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let required: i64 = db
                .query_row(
                    "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            check_capacity(
                stage.path(),
                u64::try_from(required).map_err(|_| "DBサイズが範囲外です")?,
            )?;
            let mut target = Connection::open(&database).map_err(|e| e.to_string())?;
            {
                // Online Backup reads committed WAL frames; never copy the raw
                // live DB file, checkpoint it, or close/reopen the original.
                let backup = Backup::new(&db, &mut target).map_err(|e| e.to_string())?;
                let mut busy = 0;
                loop {
                    check_cancelled(cancelled)?;
                    match backup.step(128).map_err(|e| e.to_string())? {
                        StepResult::Done => break,
                        StepResult::More => busy = 0,
                        StepResult::Busy | StepResult::Locked if busy < 20 => {
                            busy += 1;
                            std::thread::sleep(std::time::Duration::from_millis(25));
                        }
                        _ => return Err("DBスナップショットを取得できません".into()),
                    }
                }
            }
            // Consolidate the destination only, giving a single durable image.
            target
                .execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
                .map_err(|e| e.to_string())?;
            target.close().map_err(|(_, e)| e.to_string())?;
        }
        File::open(&database)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        let mut db = read_only(&database)?;
        validate_database(&mut db, cancelled)?;
        let meetings = read_meetings(&db)?;
        let assets = read_assets(&db)?;
        let required = assets.iter().try_fold(0_u64, |sum, a| {
            sum.checked_add(u64::try_from(a.size_bytes).map_err(|_| "ファイルサイズが不正です")?)
                .ok_or("バックアップ容量が範囲外です")
        })?;
        check_capacity(stage.path(), required)?;
        let asset_directory = stage.path().join("assets");
        std::fs::create_dir(&asset_directory).map_err(|e| e.to_string())?;
        let mut staged = Vec::with_capacity(assets.len());
        for asset in assets {
            check_cancelled(cancelled)?;
            let source = source_path(self, &asset)?;
            let key = Uuid::new_v4().to_string();
            let path = asset_directory.join(&key);
            copy_asset(&source, &path, &asset, cancelled)?;
            staged.push(StagedAsset { asset, key, path });
        }
        drop(db);
        let (database_size, database_hash) = digest_file(&database, cancelled)?;
        let snapshot = Snapshot {
            stage,
            database,
            database_hash,
            database_size,
            assets: staged,
            meetings,
            _maintenance: maintenance,
        };
        snapshot.verify(cancelled)?;
        Ok(snapshot)
    }
}

impl Snapshot<'_> {
    pub(super) fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        if digest_file(&self.database, cancelled)?
            != (self.database_size, self.database_hash.clone())
        {
            return Err("一時DBスナップショットが変更されています".into());
        }
        for staged in &self.assets {
            let (size, hash) = digest_file(&staged.path, cancelled)?;
            if size != staged.asset.size_bytes as u64 || hash != staged.asset.content_hash {
                return Err("一時バックアップの管理ファイルが変更されています".into());
            }
        }
        Ok(())
    }
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("バックアップを中断しました".into())
    } else {
        Ok(())
    }
}

fn check_capacity(path: &Path, bytes: u64) -> Result<(), String> {
    let required = bytes
        .checked_add(CAPACITY_RESERVE)
        .ok_or("バックアップ容量が範囲外です")?;
    if crate::audio_import::available_bytes(path)? < required {
        return Err("バックアップ一時領域の空き容量が不足しています".into());
    }
    Ok(())
}

fn read_only(path: &Path) -> Result<Connection, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA query_only=ON;")
        .map_err(|e| e.to_string())?;
    Ok(db)
}

fn validate_database(db: &mut Connection, cancelled: &impl Fn() -> bool) -> Result<(), String> {
    check_cancelled(cancelled)?;
    if verify_migration_history(db).map_err(|e| e.to_string())? != MIGRATIONS.len() {
        return Err("未対応のDBスナップショット形式です".into());
    }
    ledger::validate_provenance(db, cancelled)?;
    let result: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if result != "ok"
        || db
            .prepare("PRAGMA foreign_key_check")
            .map_err(|e| e.to_string())?
            .query([])
            .map_err(|e| e.to_string())?
            .next()
            .map_err(|e| e.to_string())?
            .is_some()
    {
        return Err("DBスナップショットの参照または構造が破損しています".into());
    }
    let tx = db.transaction().map_err(|e| e.to_string())?;
    for (table, kind) in [
        ("analysis_revisions", "解析"),
        ("minutes_revisions", "議事録"),
    ] {
        let mut statement = tx
            .prepare(&format!("SELECT id,content_hash FROM {table} ORDER BY id"))
            .map_err(|e| e.to_string())?;
        let mut rows = statement.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let id: String = row.get(0).map_err(|e| e.to_string())?;
            let expected: String = row.get(1).map_err(|e| e.to_string())?;
            let actual = if table == "analysis_revisions" {
                super::revisions::analysis_content_hash(&tx, &id)?
            } else {
                super::revisions::minutes_content_hash(&tx, &id)?
            };
            if expected != actual {
                return Err(format!("{kind}版の内容ハッシュが一致しません"));
            }
        }
    }
    // Validate every saved draft (including inactive ones), not just UI heads.
    let mut statement = tx.prepare("SELECT meeting_id,target_kind,base_analysis_id,base_minutes_id,payload_json,content_hash,schema_version FROM working_drafts ORDER BY id").map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(cancelled)?;
        let meeting: String = row.get(0).map_err(|e| e.to_string())?;
        let kind: String = row.get(1).map_err(|e| e.to_string())?;
        if kind == "transcript" {
            let base: String = row.get(2).map_err(|e| e.to_string())?;
            let draft = super::drafts::load_draft_for_base(&tx, &meeting, Some(&base))?
                .ok_or("保存済み下書きが見つかりません")?;
            super::drafts::validate_payload(&tx, &base, draft.payload)?;
        } else {
            let base: String = row.get(3).map_err(|e| e.to_string())?;
            let json: String = row.get(4).map_err(|e| e.to_string())?;
            let hash: String = row.get(5).map_err(|e| e.to_string())?;
            let schema: i64 = row.get(6).map_err(|e| e.to_string())?;
            if schema != 1
                || json.len() > 2 * 1024 * 1024
                || super::minutes_drafts::hash(&json) != hash
            {
                return Err("議事録下書きの形式・内容ハッシュが不正です".into());
            }
            // Archived inactive drafts are durable too. UI load intentionally
            // hides revisions outside the two heads, so read the fixed base here.
            let (analysis, hash, document): (String,String,String) = tx.query_row(
                "SELECT input_analysis_id,input_content_hash,effective_document_json FROM minutes_revisions WHERE id=?1 AND meeting_id=?2 AND publication_state='committed'",
                params![base,meeting], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).map_err(|e| e.to_string())?;
            let input = super::minutes_input::read_snapshot(&tx, &meeting, &analysis, &hash)?;
            let payload = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            let base = serde_json::from_str(&document).map_err(|e| e.to_string())?;
            super::minutes_drafts::validate(&payload, &base, &input)?;
        }
    }
    let mut statement = tx
        .prepare("SELECT id FROM processing_jobs ORDER BY id")
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(cancelled)?;
        super::jobs::read_fixed_input(&tx, &row.get::<_, String>(0).map_err(|e| e.to_string())?)?;
    }
    super::vocabulary::verify_saved_books(&tx)?;
    for sql in [
        "SELECT plan_json,plan_hash FROM analysis_flows WHERE plan_json IS NOT NULL",
        "SELECT options_json,options_hash FROM audio_import_options",
    ] {
        let mut statement = tx.prepare(sql).map_err(|e| e.to_string())?;
        let mut rows = statement.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check_cancelled(cancelled)?;
            let json: String = row.get(0).map_err(|e| e.to_string())?;
            let expected: String = row.get(1).map_err(|e| e.to_string())?;
            if format!("{:x}", Sha256::digest(json.as_bytes())) != expected {
                return Err("保存済み設定・処理記録・編集履歴が変更されています".into());
            }
        }
    }
    let mut statement = tx.prepare("SELECT e.draft_id,d.target_kind,e.sequence,e.payload_json,e.inverse_json,e.before_hash,e.after_hash,d.content_hash FROM edit_operations e JOIN working_drafts d ON d.id=e.draft_id ORDER BY e.draft_id,e.sequence").map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut previous: Option<(String, i64, String, String)> = None;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(cancelled)?;
        let draft: String = row.get(0).map_err(|e| e.to_string())?;
        let kind: String = row.get(1).map_err(|e| e.to_string())?;
        let sequence: i64 = row.get(2).map_err(|e| e.to_string())?;
        let payload: String = row.get(3).map_err(|e| e.to_string())?;
        let inverse: String = row.get(4).map_err(|e| e.to_string())?;
        let before: String = row.get(5).map_err(|e| e.to_string())?;
        let after: String = row.get(6).map_err(|e| e.to_string())?;
        let current: String = row.get(7).map_err(|e| e.to_string())?;
        let hash = if kind == "transcript" {
            super::drafts::draft_hash
        } else {
            super::minutes_drafts::hash
        };
        if hash(&payload) != after || hash(&inverse) != before {
            return Err("保存済み編集履歴の内容が変更されています".into());
        }
        if let Some((id, seq, prev_after, prev_current)) = &previous {
            if *id == draft {
                if sequence != seq + 1 || before != *prev_after {
                    return Err("保存済み編集履歴のつながりが不正です".into());
                }
            } else if prev_after != prev_current || sequence != 1 {
                return Err("保存済み編集履歴の最終内容が不正です".into());
            }
        } else if sequence != 1 {
            return Err("保存済み編集履歴の開始位置が不正です".into());
        }
        previous = Some((draft, sequence, after, current));
    }
    if previous.is_some_and(|(_, _, after, current)| after != current) {
        return Err("保存済み編集履歴と下書きが一致しません".into());
    }
    let mut statement = tx.prepare("SELECT c.checkpoint_json,c.checkpoint_hash,c.completed_units,j.spec_hash FROM job_checkpoints c JOIN processing_jobs j ON j.id=c.job_id").map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check_cancelled(cancelled)?;
        let json: String = row.get(0).map_err(|e| e.to_string())?;
        let expected: String = row.get(1).map_err(|e| e.to_string())?;
        let units: i64 = row.get(2).map_err(|e| e.to_string())?;
        let spec: String = row.get(3).map_err(|e| e.to_string())?;
        if super::jobs::checkpoint_hash(&spec, units, &json) != expected {
            return Err("処理の保存済みチェックポイントが変更されています".into());
        }
    }
    Ok(())
}

fn read_meetings(db: &Connection) -> Result<Vec<Meeting>, String> {
    let mut statement = db.prepare("SELECT id,title,lifecycle,source_type,created_at,updated_at,lock_version FROM meetings ORDER BY id").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok(Meeting {
                id: r.get(0)?,
                title: r.get(1)?,
                lifecycle: r.get(2)?,
                source_type: r.get(3)?,
                created_at: r.get(4)?,
                updated_at: r.get(5)?,
                lock_version: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn read_assets(db: &Connection) -> Result<Vec<FileAsset>, String> {
    let mut statement = db.prepare("SELECT id,meeting_id,relative_path,media_type,size_bytes,content_hash,state FROM file_assets ORDER BY id").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok(FileAsset {
                id: r.get(0)?,
                meeting_id: r.get(1)?,
                relative_path: r.get(2)?,
                media_type: r.get(3)?,
                size_bytes: r.get(4)?,
                content_hash: r.get(5)?,
                state: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let assets: Vec<FileAsset> = rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    if assets.iter().any(|a| a.state != "ready") {
        return Err(
            "未準備・破損・削除待ちの管理ファイルがあります。修復後にバックアップしてください"
                .into(),
        );
    }
    Ok(assets)
}

fn source_path(storage: &Storage, asset: &FileAsset) -> Result<PathBuf, String> {
    if asset.relative_path.contains('\\') {
        return Err("管理パスが不正です".into());
    }
    let relative = Path::new(&asset.relative_path);
    let path = storage.managed_path(relative)?;
    let mut parts = relative.components();
    if !matches!(parts.next(), Some(Component::Normal(p)) if p == "meetings")
        || !matches!(parts.next(), Some(Component::Normal(p)) if p == asset.meeting_id.as_str())
        || parts.next().is_none()
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("管理ファイルの会議・パスが不正です".into());
    }
    Ok(path)
}

fn private_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| e.to_string())
}

fn same_file(before: &Metadata, after: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return false;
        }
    }
    before.is_file()
        && after.is_file()
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}

fn open_source(path: &Path) -> Result<(File, Metadata), String> {
    let before = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("管理対象は通常ファイルに限ります".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !same_file(&before, &file.metadata().map_err(|e| e.to_string())?) {
        return Err("管理ファイルが変更されています".into());
    }
    Ok((file, before))
}

fn stream(
    file: &mut File,
    mut output: Option<&mut File>,
    expected: u64,
    cancelled: &impl Fn() -> bool,
) -> Result<(u64, String), String> {
    let mut buffer = [0_u8; BUFFER_BYTES];
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    loop {
        check_cancelled(cancelled)?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or("管理ファイルが大きすぎます")?;
        // Never fill disk or read indefinitely if an external actor grows a file.
        if size > expected {
            return Err("管理ファイルのサイズが変更されています".into());
        }
        hash.update(&buffer[..count]);
        if let Some(destination) = output.as_deref_mut() {
            destination
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
        }
    }
    if size != expected {
        return Err("管理ファイルのサイズが変更されています".into());
    }
    Ok((size, format!("{:x}", hash.finalize())))
}

fn copy_asset(
    source: &Path,
    target: &Path,
    asset: &FileAsset,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let (mut input, before) = open_source(source)?;
    let expected = u64::try_from(asset.size_bytes).map_err(|_| "管理ファイルサイズが不正です")?;
    if before.len() != expected {
        return Err("管理ファイルサイズが登録値と一致しません".into());
    }
    let mut output = private_file(target)?;
    let (_, hash) = stream(&mut input, Some(&mut output), expected, cancelled)?;
    if hash != asset.content_hash
        || !same_file(&before, &input.metadata().map_err(|e| e.to_string())?)
        || !same_file(
            &before,
            &std::fs::symlink_metadata(source).map_err(|e| e.to_string())?,
        )
        || source.canonicalize().map_err(|e| e.to_string())? != source
    {
        return Err("管理ファイルの内容・パスが変更されています".into());
    }
    output.sync_all().map_err(|e| e.to_string())
}

fn digest_file(path: &Path, cancelled: &impl Fn() -> bool) -> Result<(u64, String), String> {
    let (mut file, before) = open_source(path)?;
    let result = stream(&mut file, None, before.len(), cancelled)?;
    if !same_file(&before, &file.metadata().map_err(|e| e.to_string())?)
        || !same_file(
            &before,
            &std::fs::symlink_metadata(path).map_err(|e| e.to_string())?,
        )
    {
        return Err("バックアップ検証中にファイルが変更されています".into());
    }
    Ok(result)
}

#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
