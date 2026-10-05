//! macOS meeting-root selection precedes opening SQLite. No silent fallback,
//! copy, deletion, or activation of an unverified relocation destination.
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

const NAME: &str = "bootstrap.json";
const LOCK: &str = ".bootstrap.lock";
const STAMP: &[u8] = b"MON bootstrap 1\n";
const LIMIT: u64 = 32 * 1024;

#[allow(dead_code)] // Native coordinator/UI and cutover integration follow.
pub(crate) mod placement;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Identity {
    device: u64,
    inode: u64,
    birth_ns: u64,
}
impl Identity {
    fn read(m: &Metadata) -> Result<Self, String> {
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
            birth_ns: m
                .created()
                .map_err(|e| e.to_string())?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
                .try_into()
                .map_err(|_| "保存先の実体時刻が範囲外です")?,
        })
    }
    fn matches(&self, m: &Metadata) -> bool {
        Self::read(m).is_ok_and(|i| i == *self)
    }
}
fn absolute(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .to_str()
            .is_none_or(|p| p.len() > 4096 || p.contains('\0'))
        || path
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err("起動設定の保存先には正規化した絶対パスが必要です".into());
    }
    Ok(())
}
fn directory(path: &Path) -> Result<Metadata, String> {
    absolute(path)?;
    let m = std::fs::symlink_metadata(path)
        .map_err(|e| format!("保存先を確認できません。空の保存先へ切り替えません: {e}"))?;
    if !m.is_dir() || path.canonicalize().map_err(|e| e.to_string())? != path {
        return Err("保存先の実体または親が変更されています。データを保持します".into());
    }
    Ok(m)
}
fn regular(m: &Metadata, private: bool) -> bool {
    m.is_file() && m.nlink() == 1 && (!private || m.permissions().mode() & 0o077 == 0)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn uuid(s: &str) -> bool {
    Uuid::parse_str(s).is_ok_and(|u| u.to_string() == s)
}
fn sha(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Only local writable filesystems are candidates. This is not a throughput
/// qualification or proof of power-loss behavior. Network roots are rejected.
fn local(path: &Path) -> Result<(), String> {
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(c.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let info = unsafe { info.assume_init() };
    let kind = info
        .f_fstypename
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as u8)
        .collect::<Vec<_>>();
    if !mount_supported(info.f_flags, &kind) {
        return Err("会議保存先は検証対象の書込可能なローカルAPFS領域に限定します".into());
    }
    Ok(())
}
fn mount_supported(flags: u32, kind: &[u8]) -> bool {
    flags & libc::MNT_LOCAL as u32 != 0 && flags & libc::MNT_RDONLY as u32 == 0 && kind == b"apfs"
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Root {
    path: PathBuf,
    directory: Identity,
    database: Identity,
}
impl Root {
    fn inspect(path: &Path) -> Result<Self, String> {
        let d = directory(path)?;
        local(path)?;
        let m = std::fs::symlink_metadata(path.join("meetings.sqlite3"))
            .map_err(|e| format!("登録済みDBがありません。新しいDBは作成しません: {e}"))?;
        if !regular(&m, false) {
            return Err("会議DBは単独の通常ファイルである必要があります".into());
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path.join("meetings.sqlite3"))
            .map_err(|e| e.to_string())?;
        if !Identity::read(&m)?.matches(&file.metadata().map_err(|e| e.to_string())?) {
            return Err("会議DBが確認中に差し替えられています".into());
        }
        // SQLite treats a zero-byte file as a new database. Reject it BEFORE
        // Storage opens it, not after migrations have created empty tables.
        // Header checks are not a replacement for SQLite integrity/migrations.
        let mut header = [0u8; 100];
        file.read_exact(&mut header)
            .map_err(|e| format!("登録済みDBのヘッダーを読めません。新しいDBにしません: {e}"))?;
        if &header[..16] != b"SQLite format 3\0"
            || !matches!(header[18], 1 | 2)
            || !matches!(header[19], 1 | 2)
        {
            return Err("登録済みDBのヘッダーが不正です。自動初期化しません".into());
        }
        Ok(Self {
            path: path.into(),
            directory: Identity::read(&d)?,
            database: Identity::read(&m)?,
        })
    }
    fn verify(&self) -> Result<(), String> {
        if Self::inspect(&self.path)? != *self {
            return Err(
                "起動設定に登録した保存先またはDBの実体が変更されています。自動修復しません".into(),
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Origin {
    Legacy,
    Default,
    Relocated,
}

/// Durable intent only. Startup always opens `current`, never this target.
/// Copy verification and destination activation are supplied by relocation,
/// not by a user-editable path or a claim of progress in this record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PendingMove {
    operation_id: String,
    confirmation_id: String,
    source: Root,
    target: PathBuf,
    target_directory: Identity,
    manifest_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Record {
    revision: u64,
    origin: Origin,
    current: Root,
    pending_move: Option<PendingMove>,
}
impl Record {
    fn validate(&self) -> Result<(), String> {
        absolute(&self.current.path)?;
        if self.revision > i64::MAX as u64 {
            return Err("起動設定の版が範囲外です".into());
        }
        if let Some(p) = &self.pending_move {
            absolute(&p.target)?;
            if !uuid(&p.operation_id)
                || !uuid(&p.confirmation_id)
                || !sha(&p.manifest_hash)
                || p.source != self.current
                || p.target.starts_with(&self.current.path)
                || self.current.path.starts_with(&p.target)
            {
                return Err("移動の起動設定が不正です。旧保存先を変更しません".into());
            }
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Document {
    schema_version: u32,
    record: Record,
    sha256: String,
}
struct Loaded {
    record: Record,
    identity: Identity,
    bytes_hash: String,
}
struct State {
    loaded: Option<Loaded>,
    uncertain: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    pub meeting_root: PathBuf,
    pub models_root: PathBuf,
    pub bootstrap_path: PathBuf,
    pub revision: u64,
    pub origin: String,
    pub pending_operation_id: Option<String>,
    pub pending_target: Option<PathBuf>,
}
pub(crate) struct Bootstrap {
    directory: File,
    config: PathBuf,
    identity: Identity,
    lock: File,
    lock_identity: Identity,
    selected: PathBuf,
    selected_identity: Identity,
    initial_origin: Origin,
    models_root: PathBuf,
    state: Mutex<State>,
}

impl Bootstrap {
    pub(crate) fn open(config: &Path, legacy: &Path) -> Result<Self, String> {
        absolute(config)?;
        absolute(legacy)?;
        if let Ok(m) = std::fs::symlink_metadata(config) {
            if !m.is_dir() {
                return Err("起動設定の親が通常のフォルダーではありません".into());
            }
        }
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(config)
            .map_err(|e| e.to_string())?;
        let config = config.canonicalize().map_err(|e| e.to_string())?;
        let identity = Identity::read(&directory(&config)?)?;
        local(&config)?;
        let c = std::ffi::CString::new(config.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let dir = unsafe { File::from_raw_fd(fd) };
        let lock_name = std::ffi::CString::new(LOCK).unwrap();
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                lock_name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err("起動設定のロックを開けません。設定を保持します".into());
        }
        let mut lock = unsafe { File::from_raw_fd(fd) };
        if !regular(&lock.metadata().map_err(|e| e.to_string())?, true) {
            return Err("起動設定のロックの実体・権限が不正です".into());
        }
        lock.try_lock().map_err(|e| {
            format!("同じ起動設定のアプリが実行中、または排他を確認できません: {e}")
        })?;
        let lock_identity = Identity::read(&lock.metadata().map_err(|e| e.to_string())?)?;
        let mut stamp = vec![];
        (&mut lock)
            .take(128)
            .read_to_end(&mut stamp)
            .map_err(|e| e.to_string())?;
        if !stamp.is_empty() && stamp != STAMP {
            return Err("起動設定の初期化記録が不正です".into());
        }
        let mut result = Self {
            directory: dir,
            config,
            identity,
            lock,
            lock_identity,
            selected: PathBuf::new(),
            selected_identity: Identity {
                device: 0,
                inode: 0,
                birth_ns: 0,
            },
            initial_origin: Origin::Default,
            models_root: legacy.join("models"),
            state: Mutex::new(State {
                loaded: None,
                uncertain: false,
            }),
        };
        result.guard_directory()?;
        let loaded = result.read()?;
        if loaded.is_some() != (stamp == STAMP) {
            return Err("起動設定と初期化記録が一致しません。既定の空DBへ切り替えません".into());
        }
        let (selected, origin) = match &loaded {
            Some(l) => {
                l.record.current.verify()?;
                (l.record.current.path.clone(), l.record.origin.clone())
            }
            None => {
                // Preserve the pre-bootstrap database. Only fresh installs use
                // app-data/data, keeping models and control files outside it.
                let old = std::fs::symlink_metadata(legacy.join("meetings.sqlite3"));
                let fresh = legacy.join("data");
                if old.is_ok() && std::fs::symlink_metadata(fresh.join("meetings.sqlite3")).is_ok()
                {
                    return Err(
                        "旧保存先と新しい既定保存先の両方にDBがあります。自動選択しません".into(),
                    );
                }
                match old {
                    Ok(m) if regular(&m, false) => (legacy.to_path_buf(), Origin::Legacy),
                    Ok(_) => {
                        return Err("旧会議DBの実体が不正です。新しいDBへ切り替えません".into())
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (fresh, Origin::Default),
                    Err(e) => return Err(e.to_string()),
                }
            }
        };
        if std::fs::symlink_metadata(&selected).is_ok_and(|m| !m.is_dir()) {
            return Err("会議保存先が通常のフォルダーではありません".into());
        }
        // Only native default/legacy paths may be created before initialization.
        // Configured roots were already verified, so a missing one never reaches here.
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&selected)
            .map_err(|e| e.to_string())?;
        let selected = selected.canonicalize().map_err(|e| e.to_string())?;
        local(&selected)?;
        if let Ok(m) = std::fs::symlink_metadata(selected.join("meetings.sqlite3")) {
            if !regular(&m, false) {
                return Err("会議DBのリンク・別実体を採用しません".into());
            }
            Root::inspect(&selected)?;
        }
        result.selected_identity = Identity::read(&directory(&selected)?)?;
        result.selected = selected;
        result.initial_origin = origin;
        result
            .state
            .get_mut()
            .map_err(|_| "起動設定ロックを取得できません")?
            .loaded = loaded;
        Ok(result)
    }
    pub(crate) fn root(&self) -> &Path {
        &self.selected
    }
    fn guard_directory(&self) -> Result<(), String> {
        if !self.identity.matches(&directory(&self.config)?)
            || !self
                .identity
                .matches(&self.directory.metadata().map_err(|e| e.to_string())?)
            || !self.lock_identity.matches(
                &std::fs::symlink_metadata(self.config.join(LOCK)).map_err(|e| e.to_string())?,
            )
            || !regular(&self.lock.metadata().map_err(|e| e.to_string())?, true)
        {
            return Err("起動設定の親またはロックが差し替えられています。書き込みません".into());
        }
        Ok(())
    }
    fn read(&self) -> Result<Option<Loaded>, String> {
        self.guard_directory()?;
        let c = std::ffi::CString::new(NAME).unwrap();
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            return if e.kind() == std::io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(format!("起動設定を読めません。保存先を変更しません: {e}"))
            };
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !regular(&metadata, true) || metadata.len() > LIMIT {
            return Err("起動設定の形式・権限・容量が不正です".into());
        }
        let identity = Identity::read(&metadata)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("起動設定が読込中に容量上限を超えました".into());
        }
        let doc: Document = serde_json::from_slice(&bytes)
            .map_err(|e| format!("起動設定が不正です。空DBへ切り替えません: {e}"))?;
        if doc.schema_version != 1
            || !sha(&doc.sha256)
            || hash(&serde_json::to_vec(&doc.record).map_err(|e| e.to_string())?) != doc.sha256
        {
            return Err("起動設定の形式版またはSHAが一致しません".into());
        }
        doc.record.validate()?;
        if !identity
            .matches(&std::fs::symlink_metadata(self.config.join(NAME)).map_err(|e| e.to_string())?)
        {
            return Err("起動設定が読込中に差し替えられています".into());
        }
        self.guard_directory()?;
        Ok(Some(Loaded {
            record: doc.record,
            identity,
            bytes_hash: hash(&bytes),
        }))
    }
    fn guard(&self, state: &State) -> Result<(), String> {
        if state.uncertain {
            return Err(
                "起動設定の永続化が未確定です。操作を停止し再起動後に確認してください".into(),
            );
        }
        let now = self.read()?;
        match (&state.loaded, now) {
            (None, None) => Ok(()),
            (Some(old), Some(now))
                if old.identity == now.identity
                    && old.bytes_hash == now.bytes_hash
                    && old.record == now.record =>
            {
                Ok(())
            }
            _ => Err("起動設定が変更されています。古い確認では更新しません".into()),
        }
    }
    fn write(&self, state: &mut State, record: Record) -> Result<(), String> {
        self.guard(state)?;
        record.validate()?;
        let digest = hash(&serde_json::to_vec(&record).map_err(|e| e.to_string())?);
        let bytes = serde_json::to_vec(&Document {
            schema_version: 1,
            record,
            sha256: digest,
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("起動設定が容量上限を超えています".into());
        }
        let name = std::ffi::CString::new(format!(".bootstrap-{}.tmp", Uuid::new_v4())).unwrap();
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        self.guard(state)?;
        let target = std::ffi::CString::new(NAME).unwrap();
        // First creation cannot replace another file that appeared after guard.
        let flags = if state.loaded.is_none() {
            libc::RENAME_EXCL
        } else {
            0
        };
        if unsafe {
            libc::renameatx_np(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                self.directory.as_raw_fd(),
                target.as_ptr(),
                flags,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // Rename may already have committed. Never retry against stale state
        // after a sync/read failure; next launch reads the durable selection.
        state.uncertain = true;
        self.directory.sync_all().map_err(|e| e.to_string())?;
        state.loaded = self.read()?;
        if state
            .loaded
            .as_ref()
            .is_none_or(|l| l.bytes_hash != hash(&bytes))
        {
            return Err("起動設定の確定結果が一致しません".into());
        }
        state.uncertain = false;
        Ok(())
    }
    pub(crate) fn initialize(&self, storage: &Storage) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        if storage.root() != self.selected
            || !self.selected_identity.matches(&directory(&self.selected)?)
        {
            return Err("開いた会議保存先が起動設定と一致しません".into());
        }
        if let Some(l) = &state.loaded {
            return l.record.current.verify();
        }
        let root = Root::inspect(&self.selected)?;
        // Stamp BEFORE publication, so an initialization crash or deletion of
        // bootstrap.json can never trigger automatic empty-root fallback.
        let mut lock = self.lock.try_clone().map_err(|e| e.to_string())?;
        lock.seek(SeekFrom::Start(0))
            .and_then(|_| lock.write_all(STAMP))
            .and_then(|_| lock.sync_all())
            .map_err(|e| e.to_string())?;
        self.write(
            &mut state,
            Record {
                revision: 0,
                origin: self.initial_origin.clone(),
                current: root,
                pending_move: None,
            },
        )
    }
    pub(crate) fn view(&self) -> Result<View, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        let r = &state.loaded.as_ref().ok_or("起動設定が未確定です")?.record;
        r.current.verify()?;
        Ok(View {
            meeting_root: r.current.path.clone(),
            models_root: self.models_root.clone(),
            bootstrap_path: self.config.join(NAME),
            revision: r.revision,
            origin: match r.origin {
                Origin::Legacy => "legacy",
                Origin::Default => "default",
                Origin::Relocated => "relocated",
            }
            .into(),
            pending_operation_id: r.pending_move.as_ref().map(|p| p.operation_id.clone()),
            pending_target: r.pending_move.as_ref().map(|p| p.target.clone()),
        })
    }
    // Internal intent lifecycle for relocation. No command accepts these paths;
    // only a later verified-copy capability may activate a new current root.
    #[allow(dead_code)]
    pub(crate) fn begin_move(
        &self,
        operation: &str,
        confirmation: &str,
        target: &Path,
        manifest: &str,
        expected_revision: u64,
    ) -> Result<View, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        let mut r = state
            .loaded
            .as_ref()
            .ok_or("起動設定が未確定です")?
            .record
            .clone();
        r.current.verify()?;
        if let Some(p) = &r.pending_move {
            if p.operation_id == operation
                && p.confirmation_id == confirmation
                && p.target == target
                && p.manifest_hash == manifest
            {
                drop(state);
                return self.view();
            }
            return Err("別の保存先移動が未完了です。旧保存先を維持します".into());
        }
        if r.revision != expected_revision {
            return Err("起動設定の確認が古くなっています".into());
        }
        if target.starts_with(&self.config)
            || self.config.starts_with(target)
            || target.starts_with(&self.models_root)
            || self.models_root.starts_with(target)
        {
            return Err("起動設定またはモデルの領域を会議保存先にできません".into());
        }
        local(target)?;
        let metadata = directory(target)?;
        if metadata.permissions().mode() & 0o077 != 0
            || std::fs::read_dir(target)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err(
                "移動先は新しい非公開の空フォルダーに限定します。既存物を採用しません".into(),
            );
        }
        let p = PendingMove {
            operation_id: operation.into(),
            confirmation_id: confirmation.into(),
            source: r.current.clone(),
            target: target.into(),
            target_directory: Identity::read(&metadata)?,
            manifest_hash: manifest.into(),
        };
        r.pending_move = Some(p);
        r.revision = r.revision.checked_add(1).ok_or("起動設定の版が上限です")?;
        self.write(&mut state, r)?;
        drop(state);
        self.view()
    }
    #[allow(dead_code)]
    pub(crate) fn cancel_move(&self, operation: &str) -> Result<View, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        let mut r = state
            .loaded
            .as_ref()
            .ok_or("起動設定が未確定です")?
            .record
            .clone();
        r.current.verify()?;
        if r.pending_move
            .as_ref()
            .is_none_or(|p| p.operation_id != operation)
        {
            return Err("取消対象の保存先移動が一致しません".into());
        }
        match std::fs::symlink_metadata(
            self.config.join(format!(".relocation-{operation}.sqlite3")),
        ) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
            Ok(_) => {
                return Err(
                    "配置台帳がある移動は、途中コピーの保持・解決を確認するまで記録を消せません"
                        .into(),
                )
            }
        }
        r.pending_move = None;
        r.revision = r.revision.checked_add(1).ok_or("起動設定の版が上限です")?;
        self.write(&mut state, r)?;
        drop(state);
        self.view()
    }
}

#[cfg(test)]
pub(crate) fn set_test_root(config: &Path, data: &Path, root: &Path) {
    let b = Bootstrap::open(config, data).unwrap();
    let mut state = b.state.lock().unwrap();
    let mut record = state.loaded.as_ref().unwrap().record.clone();
    record.current = Root::inspect(root).unwrap();
    b.write(&mut state, record).unwrap();
}

#[cfg(test)]
mod tests;
