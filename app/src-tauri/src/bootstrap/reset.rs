//! Startup-only reset and typed recovery. A plain reset never queries the old
//! database; recovery reads only a private copy and reconstructs user content.
//! A private write-ahead record owns every rename and the publication hash is
//! the commit point. Plain-reset archives exclude model payloads; recovery
//! verifies and collects registered model files before moving any old data.
use super::*;
use std::{collections::BTreeSet, sync::Arc};
const JOURNAL: &str = ".startup-reset.json";
const PENDING: &str = ".mon-startup-reset.pending";
const ROOT_LOCK: &str = ".mon-startup-reset.lock";
const MAX_RECORD: u64 = 8 * 1024 * 1024;
#[cfg(unix)]
type Flags = i32;
#[cfg(windows)]
type Flags = u32;
const DB: &str = "meetings.sqlite3";

pub(crate) struct RootLease {
    file: File,
    root: PathBuf,
    identity: Identity,
    lock_identity: Identity,
}
impl RootLease {
    pub(crate) fn acquire(root: &Path, resetting: bool) -> Result<Arc<Self>, String> {
        let identity = Identity::read(&directory(root)?)?;
        // Reject aliases using metadata only. Opening/closing a live SQLite
        // inode before acquiring this lease can release POSIX process locks.
        for name in [
            DB,
            "meetings.sqlite3-wal",
            "meetings.sqlite3-shm",
            "meetings.sqlite3-journal",
        ] {
            match crate::native_fs::symlink_metadata(root.join(name)) {
                Ok(m) if regular(&m, false) && m.dev() == identity.device => {}
                Ok(_) => return Err("DB関連ファイルのリンク・実体を確認できません".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(err(e)),
            }
        }
        if !exists(&root.join(ROOT_LOCK))? {
            match write_new(&root.join(ROOT_LOCK), b"") {
                Ok(()) => {}
                Err(e) if exists(&root.join(ROOT_LOCK))? => {
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        }
        let file = open_file(&root.join(ROOT_LOCK), libc::O_RDWR)?;
        let m = crate::native_fs::metadata(&file).map_err(err)?;
        if !regular(&m, true) {
            return Err("保存先の排他ファイルが不正です".into());
        }
        file.try_lock()
            .map_err(|e| format!("他のアプリが保存先を使用中、または排他を取得できません: {e}"))?;
        let lease = Arc::new(Self {
            file,
            root: root.into(),
            identity,
            lock_identity: Identity::read(&m)?,
        });
        lease.verify(root)?;
        if !resetting && exists(&root.join(PENDING))? {
            return Err("未完了の初期化があります。元の起動設定でアプリを起動してください".into());
        }
        Ok(lease)
    }
    pub(crate) fn verify(&self, root: &Path) -> Result<(), String> {
        if root != self.root
            || !self.identity.matches(&directory(root)?)
            || !self
                .lock_identity
                .matches(&crate::native_fs::metadata(&self.file).map_err(err)?)
            || !self.lock_identity.matches(&meta(&root.join(ROOT_LOCK))?)
        {
            return Err("保存先または排他ファイルの実体が変わっています".into());
        }
        Ok(())
    }
}
impl Drop for RootLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn meta(p: &Path) -> Result<Metadata, String> {
    crate::native_fs::symlink_metadata(p).map_err(err)
}
fn exists(p: &Path) -> Result<bool, String> {
    match crate::native_fs::symlink_metadata(p) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(err(e)),
    }
}
fn open_directory(p: &Path) -> Result<File, String> {
    let before = Identity::read(&directory(p)?)?;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(err)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    #[cfg(unix)]
    let fd = unsafe { libc::open(c.as_ptr(), flags, 0o600) };
    #[cfg(windows)]
    let fd = unsafe { libc::open(c.as_ptr(), flags) };
    if fd < 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !before.matches(&crate::native_fs::metadata(&file).map_err(err)?)
        || !before.matches(&directory(p)?)
    {
        return Err("親フォルダーが変更されています".into());
    }
    Ok(file)
}
fn open_file(p: &Path, flags: Flags) -> Result<File, String> {
    if flags & libc::O_DIRECTORY != 0 {
        return open_directory(p);
    }
    let parent = p.parent().ok_or("親フォルダーがありません")?;
    let directory = open_directory(parent)?;
    let name = std::ffi::CString::new(p.file_name().ok_or("ファイル名がありません")?.as_bytes())
        .map_err(err)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        Err(err(std::io::Error::last_os_error()))
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn create_directory(p: &Path) -> Result<(), String> {
    let parent = p.parent().ok_or("親フォルダーがありません")?;
    let directory = open_directory(parent)?;
    let name = std::ffi::CString::new(p.file_name().ok_or("フォルダー名がありません")?.as_bytes())
        .map_err(err)?;
    if unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    crate::native_fs::sync_handle(&directory).map_err(err)
}
fn read_private(p: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut f = open_file(p, libc::O_RDONLY)?;
    let m = crate::native_fs::metadata(&f).map_err(err)?;
    if !regular(&m, true) || m.len() > limit {
        return Err("初期化記録の形式・権限が不正です".into());
    }
    let mut bytes = vec![];
    (&mut f)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > limit || !Identity::read(&m)?.matches(&meta(p)?) {
        return Err("初期化記録が変更されています".into());
    }
    Ok(bytes)
}
fn write_new(p: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut f = open_file(p, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
    f.write_all(bytes)
        .and_then(|_| crate::native_fs::sync_handle(&f))
        .map_err(err)?;
    crate::native_fs::sync_path(p.parent().ok_or("親フォルダーがありません")?)
}
fn rename(from: &Path, to: &Path, replace: bool) -> Result<(), String> {
    let a = from.parent().ok_or("移動元の親がありません")?;
    let b = to.parent().ok_or("移動先の親がありません")?;
    directory(a)?;
    directory(b)?;
    let af = open_file(a, libc::O_RDONLY | libc::O_DIRECTORY)?;
    let bf = open_file(b, libc::O_RDONLY | libc::O_DIRECTORY)?;
    let an = std::ffi::CString::new(from.file_name().ok_or("移動元が不正です")?.as_bytes())
        .map_err(err)?;
    let bn = std::ffi::CString::new(to.file_name().ok_or("移動先が不正です")?.as_bytes())
        .map_err(err)?;
    if unsafe {
        libc::renameatx_np(
            af.as_raw_fd(),
            an.as_ptr(),
            bf.as_raw_fd(),
            bn.as_ptr(),
            if replace { 0 } else { libc::RENAME_EXCL },
        )
    } != 0
    {
        return Err(err(std::io::Error::last_os_error()));
    }
    crate::native_fs::sync_handle(&af).map_err(err)?;
    crate::native_fs::sync_handle(&bf).map_err(err)
}
fn atomic(p: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    let tmp = p.with_file_name(format!(".reset-write-{}", Uuid::new_v4()));
    write_new(&tmp, bytes)?;
    rename(&tmp, p, replace)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Location {
    path: PathBuf,
    identity: Identity,
}
impl Location {
    fn new(p: &Path) -> Result<Self, String> {
        Ok(Self {
            path: p.into(),
            identity: Identity::read(&directory(p)?)?,
        })
    }
    fn verify(&self) -> Result<(), String> {
        if self.identity.matches(&directory(&self.path)?) {
            Ok(())
        } else {
            Err("初期化対象フォルダーが差し替えられています".into())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    config: Location,
    data: Location,
    meeting: Location,
    bootstrap: Option<String>,
    database: Option<Identity>,
}
impl Selection {
    pub(crate) fn root(&self) -> &Path {
        &self.meeting.path
    }
    pub(crate) fn app_data(&self) -> &Path {
        &self.data.path
    }
}
pub(crate) fn inspect(bootstrap: &Bootstrap, data: &Path) -> Result<Selection, String> {
    bootstrap.guard_directory()?;
    let state = bootstrap.state.lock().map_err(err)?;
    bootstrap.guard(&state)?;
    let selection = Selection {
        config: Location::new(&bootstrap.config)?,
        data: Location::new(&data.canonicalize().map_err(err)?)?,
        meeting: Location::new(bootstrap.root())?,
        bootstrap: state.loaded.as_ref().map(|l| l.bytes_hash.clone()),
        database: if exists(&bootstrap.root().join(DB))? {
            Some(Identity::read(&meta(&bootstrap.root().join(DB))?)?)
        } else {
            None
        },
    };
    if exists(&selection.config.path.join(JOURNAL))?
        || exists(&selection.meeting.path.join(PENDING))?
    {
        return Err("未完了の初期化があります。アプリを再起動して回復してください".into());
    }
    // Probe writable directories without reading any rejected DB/model content.
    for root in [&selection.config, &selection.data, &selection.meeting] {
        let probe = root.path.join(format!(".reset-probe-{}", Uuid::new_v4()));
        write_new(&probe, b"")?;
        std::fs::remove_file(&probe).map_err(err)?;
    }
    // Top-level managed objects are checked without reading model payloads.
    let _ = entries(&selection)?;
    Ok(selection)
}
pub(crate) fn inspect_paths(config: &Path, data: &Path) -> Result<Selection, String> {
    let bootstrap = Bootstrap::open_for_reset(config, data)?;
    let _lease = RootLease::acquire(bootstrap.root(), true)?;
    inspect(&bootstrap, data)
}
pub(crate) fn open(config: &Path, data: &Path) -> Result<Bootstrap, String> {
    Bootstrap::open_for_reset(config, data)
}

// Enumerate only native-owned data. Active logs/WebViews, external audio,
// exports and backup files are deliberately outside this namespace.
fn meeting_name(n: &str) -> bool {
    [
        DB,
        "meetings.sqlite3-wal",
        "meetings.sqlite3-shm",
        "meetings.sqlite3-journal",
        "meetings",
        "voices",
        "temp",
        "journal",
        ".mon-scratch.sqlite3",
        ".mon-scratch.sqlite3-wal",
        ".mon-scratch.sqlite3-shm",
        ".mon-scratch.sqlite3-journal",
        ".mon-handoff.sqlite3",
        ".mon-handoff.sqlite3-wal",
        ".mon-handoff.sqlite3-shm",
    ]
    .contains(&n)
        || n.starts_with(".mon-stage-")
        || n.starts_with(".mon-delete-") && !n.starts_with(".mon-delete-reset-")
}
fn data_name(n: &str) -> bool {
    [
        "models",
        "model-downloads",
        ".mon-capture-tests",
        ".mon-capture-tests-windows-v1",
    ]
    .contains(&n)
}
fn config_name(n: &str) -> bool {
    n == ".mon-read-cache" || n.starts_with(".bootstrap-recovery-") || n.starts_with(".relocation-")
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Entry {
    root: usize,
    name: String,
    identity: Identity,
    directory: bool,
    archive: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    root: usize,
    identity: Identity,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fresh {
    name: String,
    identity: Identity,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    app_data: bool,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Accepted,
    Archiving,
    Staging,
    Initializing,
    Publishing,
    Cleanup,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    phase: Phase,
    id: String,
    config: Location,
    data: Location,
    meeting: Location,
    roots: Vec<Location>,
    entries: Vec<Entry>,
    workspaces: Vec<Workspace>,
    lock_identity: Identity,
    old_bootstrap: Option<Vec<u8>>,
    new_bootstrap: Option<String>,
    fresh: Vec<Fresh>,
    staged: bool,
    committed: bool,
}
impl Journal {
    fn fresh_path(&self, fresh: &Fresh) -> PathBuf {
        (if fresh.app_data {
            &self.data.path
        } else {
            &self.meeting.path
        })
        .join(&fresh.name)
    }
    fn fresh_workspace(&self, fresh: &Fresh) -> Result<PathBuf, String> {
        let parent = self
            .fresh_path(fresh)
            .parent()
            .ok_or("復元先の親がありません")?
            .to_owned();
        let root = self
            .roots
            .iter()
            .position(|root| root.path == parent)
            .ok_or("復元先の保存領域がありません")?;
        Ok(self.workspace(root).join(format!("new-{}", fresh.name)))
    }
    fn matches_fresh(&self, fresh: &Fresh, metadata: &Metadata) -> bool {
        let Ok(actual) = Identity::read(metadata) else {
            return false;
        };
        if actual == fresh.identity {
            return true;
        }
        // NTFS can give a renamed file the creation time of the previous
        // destination. Its volume and file ID still belong to our reservation.
        // Accept only that exact, journaled previous creation time.
        #[cfg(windows)]
        if actual.device == fresh.identity.device && actual.inode == fresh.identity.inode {
            return self.entries.iter().any(|entry| {
                self.source(entry) == self.fresh_path(fresh)
                    && actual.birth_ns == entry.identity.birth_ns
            });
        }
        false
    }
    fn path(&self) -> PathBuf {
        self.config.path.join(JOURNAL)
    }
    fn workspace(&self, root: usize) -> PathBuf {
        self.roots[root]
            .path
            .join(format!(".mon-delete-reset-{}", self.id))
    }
    fn source(&self, e: &Entry) -> PathBuf {
        self.roots[e.root].path.join(&e.name)
    }
    fn target(&self, i: usize, e: &Entry) -> PathBuf {
        self.workspace(e.root).join(i.to_string())
    }
    fn save(&self) -> Result<(), String> {
        self.verify_roots()?;
        let payload = serde_json::to_vec(self).map_err(err)?;
        let envelope = serde_json::to_vec(&(hash(&payload), self)).map_err(err)?;
        if envelope.len() as u64 > MAX_RECORD {
            return Err("初期化記録の容量上限を超えました".into());
        }
        let replace = exists(&self.path())?;
        if replace {
            let (_, previous): (String, Journal) =
                serde_json::from_slice(&read_private(&self.path(), MAX_RECORD)?).map_err(err)?;
            if previous.id != self.id
                || previous.config != self.config
                || previous.meeting != self.meeting
            {
                return Err("別の初期化記録を上書きしません".into());
            }
        }
        atomic(&self.path(), &envelope, replace)
    }
    fn verify_roots(&self) -> Result<(), String> {
        self.config.verify()?;
        self.data.verify()?;
        self.meeting.verify()?;
        if !self
            .lock_identity
            .matches(&meta(&self.config.path.join(LOCK))?)
        {
            return Err("起動設定の排他ファイルが差し替えられています".into());
        }
        for r in &self.roots {
            r.verify()?;
        }
        for w in &self.workspaces {
            if w.root >= self.roots.len() {
                return Err("初期化作業領域の親が不正です".into());
            }
            let path = self.workspace(w.root);
            if exists(&path)? && !w.identity.matches(&directory(&path)?) {
                return Err("初期化作業領域が差し替えられています".into());
            }
        }
        Ok(())
    }
    fn validate(&self, config: &Path, data: &Path) -> Result<(), String> {
        if self.version != 1
            || !uuid(&self.id)
            || self.config.path != config
            || self.data.path != data
            || self.roots.len() > 3
            || self.entries.len() > 10000
            || self.fresh.len() > 20
        {
            return Err("初期化記録の対象が不正です".into());
        }
        self.verify_roots()?;
        let expected: BTreeSet<_> = [
            self.config.path.clone(),
            self.data.path.clone(),
            self.meeting.path.clone(),
        ]
        .into_iter()
        .collect();
        if self
            .roots
            .iter()
            .map(|r| r.path.clone())
            .collect::<BTreeSet<_>>()
            != expected
        {
            return Err("初期化記録の保存先が一致しません".into());
        }
        let mut names = BTreeSet::new();
        for e in &self.entries {
            let root = self.roots.get(e.root).ok_or("初期化記録の親が不正です")?;
            if e.name.contains(['/', '\\'])
                || !(root.path == self.meeting.path && meeting_name(&e.name)
                    || root.path == self.data.path && data_name(&e.name)
                    || root.path == self.config.path && config_name(&e.name))
                || !names.insert((e.root, e.name.clone()))
            {
                return Err("初期化記録の対象名が不正です".into());
            }
        }
        for e in &self.entries {
            let source = self.source(e);
            if [&self.config.path, &self.data.path, &self.meeting.path]
                .iter()
                .any(|root| root.starts_with(&source))
            {
                return Err("初期化記録が保存先の親を含んでいます".into());
            }
        }
        let mut roots = BTreeSet::new();
        for w in &self.workspaces {
            if w.root >= self.roots.len() || !roots.insert(w.root) {
                return Err("初期化作業領域が不正です".into());
            }
        }
        for f in &self.fresh {
            if if f.app_data {
                f.name != "models"
            } else {
                !fresh_names().contains(&f.name.as_str())
                    && !["meetings", "voices"].contains(&f.name.as_str())
            } {
                return Err("新DBの作業対象が不正です".into());
            }
        }
        if let Some(bytes) = &self.old_bootstrap {
            let doc: Document = serde_json::from_slice(bytes).map_err(err)?;
            doc.record.validate()?;
            if doc.record.current.path != self.meeting.path
                || doc.record.current.directory != self.meeting.identity
                || hash(&serde_json::to_vec(&doc.record).map_err(err)?) != doc.sha256
            {
                return Err("初期化の旧保存先情報が一致しません".into());
            }
        } else if self.meeting.path != self.data.path
            && self.meeting.path != self.data.path.join("data")
        {
            return Err("未登録の初期化対象を回復しません".into());
        }
        if self.new_bootstrap.as_ref().is_some_and(|h| !sha(h)) {
            return Err("切替記録のSHAが不正です".into());
        }
        Ok(())
    }
}
fn fresh_names() -> [&'static str; 8] {
    [
        DB,
        "meetings.sqlite3-wal",
        "meetings.sqlite3-shm",
        "meetings.sqlite3-journal",
        ".mon-scratch.sqlite3",
        ".mon-scratch.sqlite3-wal",
        ".mon-scratch.sqlite3-shm",
        ".mon-scratch.sqlite3-journal",
    ]
}
fn entries(s: &Selection) -> Result<(Vec<Location>, Vec<Entry>), String> {
    let mut roots = vec![s.meeting.clone(), s.data.clone(), s.config.clone()];
    roots.sort_by(|a, b| a.path.cmp(&b.path));
    roots.dedup_by(|a, b| a.path == b.path);
    let mut result = vec![];
    for (i, r) in roots.iter().enumerate() {
        for item in std::fs::read_dir(&r.path).map_err(err)? {
            let item = item.map_err(err)?;
            let Ok(name) = item.file_name().into_string() else {
                continue;
            };
            if !(r.path == s.meeting.path && meeting_name(&name)
                || r.path == s.data.path && data_name(&name)
                || r.path == s.config.path && config_name(&name))
            {
                continue;
            }
            let p = item.path();
            let m = meta(&p)?;
            if !(m.is_dir() || regular(&m, false)) || m.file_type().is_symlink() {
                return Err(format!(
                    "リンクまたは共有ファイルを初期化できません: {}",
                    p.display()
                ));
            }
            // Never move a parent containing the running configuration/runtime.
            if [&s.config.path, &s.data.path, &s.meeting.path]
                .iter()
                .any(|root| root.starts_with(&p))
            {
                return Err("初期化対象内に稼働中のアプリ保存領域があります".into());
            }
            result.push(Entry {
                root: i,
                name,
                identity: Identity::read(&m)?,
                directory: m.is_dir(),
                archive: !p.starts_with(s.data.path.join("models"))
                    && !p.starts_with(s.data.path.join("model-downloads")),
            });
        }
    }
    result.sort_by_key(|e| (e.root, e.name.clone()));
    // A child already belongs to the selected parent's staged tree.
    let paths: Vec<_> = result
        .iter()
        .map(|e| roots[e.root].path.join(&e.name))
        .collect();
    result = result
        .into_iter()
        .enumerate()
        .filter_map(|(i, e)| {
            (!paths
                .iter()
                .enumerate()
                .any(|(k, p)| k != i && paths[i].starts_with(p)))
            .then_some(e)
        })
        .collect();
    Ok((roots, result))
}
#[derive(Clone)]
pub(crate) struct Archive {
    parent: Location,
    path: PathBuf,
}
impl Archive {
    pub(crate) fn select(selection: &Selection, parent: &Path, id: &str) -> Result<Self, String> {
        let path = destination(selection, parent, id)?;
        Ok(Self {
            parent: Location::new(path.parent().ok_or("退避先の親がありません")?)?,
            path,
        })
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn verify(&self) -> Result<(), String> {
        self.parent.verify()
    }
}
pub(crate) fn destination(
    selection: &Selection,
    parent: &Path,
    id: &str,
) -> Result<PathBuf, String> {
    if !uuid(id) {
        return Err("操作IDが不正です".into());
    }
    let parent = parent.canonicalize().map_err(err)?;
    directory(&parent)?;
    local(&parent)?;
    for root in [
        &selection.meeting.path,
        &selection.data.path,
        &selection.config.path,
    ] {
        if parent.starts_with(root) {
            return Err("初期化対象の内側には退避できません".into());
        }
    }
    for ancestor in parent.ancestors() {
        for name in [DB, ROOT_LOCK, PENDING, ".mon-scratch.sqlite3"] {
            if exists(&ancestor.join(name))? {
                return Err("使用中または既存のアプリ保存先には退避できません".into());
            }
        }
    }
    let child = parent.join(format!(
        "MON-reset-{}-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        id
    ));
    if exists(&child)? {
        return Err("退避先はすでに存在します。上書きしません".into());
    }
    Ok(child)
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    path: PathBuf,
    id: Identity,
    len: u64,
    modified: std::time::SystemTime,
    directory: bool,
}
fn fingerprint(p: &Path) -> Result<Fingerprint, String> {
    let m = meta(p)?;
    if !(m.is_dir() || regular(&m, false)) || m.file_type().is_symlink() {
        return Err(format!(
            "リンク・共有ファイルは退避できません: {}",
            p.display()
        ));
    }
    Ok(Fingerprint {
        path: p.into(),
        id: Identity::read(&m)?,
        len: if m.is_dir() { 0 } else { m.len() },
        modified: m.modified().map_err(err)?,
        directory: m.is_dir(),
    })
}
fn copy_tree(
    from: &Path,
    to: &Path,
    excluded: &[PathBuf],
    inventory: &mut Vec<Fingerprint>,
    depth: usize,
) -> Result<(), String> {
    if excluded.iter().any(|p| from.starts_with(p)) {
        return Ok(());
    }
    if depth > 128 || inventory.len() > 1_000_000 {
        return Err("退避対象の階層・件数上限を超えました".into());
    }
    let before = fingerprint(from)?;
    if before.directory {
        create_directory(to)?;
        let mut children = std::fs::read_dir(from)
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        children.sort_by_key(|d| d.file_name());
        for child in children {
            copy_tree(
                &child.path(),
                &to.join(child.file_name()),
                excluded,
                inventory,
                depth + 1,
            )?;
        }
        crate::native_fs::sync_path(to)?;
    } else {
        let mut source = open_file(from, libc::O_RDONLY)?;
        if !before
            .id
            .matches(&crate::native_fs::metadata(&source).map_err(err)?)
        {
            return Err("退避元が変更されています".into());
        }
        let mut target = open_file(to, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        let mut digest = Sha256::new();
        let mut buf = [0u8; 128 * 1024];
        let mut length = 0u64;
        loop {
            let n = source.read(&mut buf).map_err(err)?;
            if n == 0 {
                break;
            }
            checkpoint("copy_write")?;
            target.write_all(&buf[..n]).map_err(err)?;
            digest.update(&buf[..n]);
            length += n as u64;
        }
        crate::native_fs::sync_handle(&target).map_err(err)?;
        drop(target);
        let mut verify = open_file(to, libc::O_RDONLY)?;
        let mut check = Sha256::new();
        loop {
            let n = verify.read(&mut buf).map_err(err)?;
            if n == 0 {
                break;
            }
            check.update(&buf[..n]);
        }
        if length != before.len || digest.finalize() != check.finalize() {
            return Err("退避コピーの内容が一致しません".into());
        }
    }
    if fingerprint(from)? != before {
        return Err("退避中に元データが変更されています".into());
    }
    inventory.push(before);
    Ok(())
}
fn verify_entry(j: &Journal, e: &Entry) -> Result<(), String> {
    let m = meta(&j.source(e))?;
    if !e.identity.matches(&m) || e.directory != m.is_dir() {
        Err("初期化対象の実体が変更されています".into())
    } else {
        Ok(())
    }
}
fn create_journal(b: &Bootstrap, s: &Selection, id: &str) -> Result<Journal, String> {
    if !uuid(id) || inspect(b, &s.data.path)? != *s {
        return Err("初期化の確認後に保存先が変更されています".into());
    }
    let (roots, entries) = entries(s)?;
    let old_bootstrap = b
        .state
        .lock()
        .map_err(err)?
        .loaded
        .as_ref()
        .map(|l| l.bytes.clone());
    Ok(Journal {
        version: 1,
        phase: Phase::Accepted,
        id: id.into(),
        config: s.config.clone(),
        data: s.data.clone(),
        meeting: s.meeting.clone(),
        roots,
        entries,
        workspaces: vec![],
        lock_identity: b.lock_identity.clone(),
        old_bootstrap,
        new_bootstrap: None,
        fresh: vec![],
        staged: false,
        committed: false,
    })
}
// Helpers can outlive their old parent. These native lease files, rather
// than a legacy ownership DB schema, prove no capture writer is still active.
fn capture_writer_leases(data: &Path) -> Result<Vec<File>, String> {
    let mut leases = Vec::new();
    for name in [".mon-capture-tests", ".mon-capture-tests-windows-v1"] {
        let root = data.join(name);
        if !exists(&root)? {
            continue;
        }
        directory(&root)?;
        for item in std::fs::read_dir(&root).map_err(err)? {
            let item = item.map_err(err)?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name != "owner.lock"
                && name != "registry.lock"
                && !name.strip_prefix("lease-").is_some_and(uuid)
            {
                continue;
            }
            let file = open_file(&item.path(), libc::O_RDWR)?;
            let m = crate::native_fs::metadata(&file).map_err(err)?;
            if !regular(&m, true)
                || m.len() != 0
                || !Identity::read(&m)?.matches(&meta(&item.path())?)
            {
                return Err("録音前テストの排他ファイルを確認できません".into());
            }
            file.try_lock()
                .map_err(|e| format!("以前の取得プロセスまたは他のアプリが使用中です: {e}"))?;
            leases.push(file);
        }
    }
    Ok(leases)
}
// Progress is separate from the immutable startup report.
pub(crate) fn execute(
    b: Bootstrap,
    s: &Selection,
    id: &str,
    archive: Option<&Path>,
    progress: &impl Fn(&str),
) -> Result<(), String> {
    execute_mode(b, s, id, archive, false, progress)
}
pub(crate) fn execute_restore(
    b: Bootstrap,
    s: &Selection,
    id: &str,
    progress: &impl Fn(&str),
) -> Result<(), String> {
    execute_mode(b, s, id, None, true, progress)
}
fn execute_mode(
    b: Bootstrap,
    s: &Selection,
    id: &str,
    archive: Option<&Path>,
    restore: bool,
    progress: &impl Fn(&str),
) -> Result<(), String> {
    let lease = RootLease::acquire(s.root(), true)?;
    let mut j = create_journal(&b, s, id)?;
    let mut writer_leases = Some(capture_writer_leases(s.app_data())?);
    // The accepted operation is durable even if archival is interrupted.
    // This control record does not alter any original payload or old DB.
    j.save()?;
    let result = (|| {
        let recovered = if restore {
            // Every source remains in place until reconstruction and validation
            // of all requested content have completed in owned private workspaces.
            for root in 0..j.roots.len() {
                let path = j.workspace(root);
                create_directory(&path)?;
                j.workspaces.push(Workspace {
                    root,
                    identity: Identity::read(&directory(&path)?)?,
                });
                j.save()?;
            }
            let meeting = j
                .roots
                .iter()
                .position(|root| root.path == s.meeting.path)
                .ok_or("会議保存先がありません")?;
            let data = j
                .roots
                .iter()
                .position(|root| root.path == s.data.path)
                .ok_or("モデル保存先がありません")?;
            let input = j.workspace(meeting).join("source");
            create_directory(&input)?;
            let mut inventory = vec![];
            for name in &fresh_names()[..4] {
                let path = s.root().join(name);
                if exists(&path)? {
                    copy_tree(&path, &input.join(name), &[], &mut inventory, 0)?;
                }
            }
            let result = crate::storage::backup::startup::prepare(
                &input.join(DB),
                s.root(),
                &s.app_data().join("models"),
                &j.workspace(meeting),
                &j.workspace(data),
                progress,
            )?;
            for source in inventory {
                if fingerprint(&source.path)? != source {
                    return Err("回収中に元DBが変更されています。元データを保持します".into());
                }
            }
            checkpoint("recovered")?;
            Some(result)
        } else {
            None
        };
        if let Some(to) = archive {
            j.phase = Phase::Archiving;
            j.save()?;
            checkpoint("archiving")?;
            progress("旧データを退避しています（モデル本体・取得途中データを除く）");
            let parent = to.parent().ok_or("退避先の親がありません")?;
            // Recheck destination using native roots, not a client supplied path.
            let checked = destination(s, parent, id)?;
            if checked.parent() != to.parent()
                || to
                    .file_name()
                    .and_then(|v| v.to_str())
                    .is_none_or(|n| !n.starts_with("MON-reset-") || !n.ends_with(id))
            {
                return Err("退避先の確認情報が変わっています".into());
            }
            create_directory(to)?;
            write_new(
                &to.join("MON-reset-incomplete.json"),
                &serde_json::to_vec(&serde_json::json!({"operationId": id, "complete": false}))
                    .map_err(err)?,
            )?;
            let excluded = [
                s.data.path.join("models"),
                s.data.path.join("model-downloads"),
            ];
            let mut inventory = vec![];
            for (i, e) in j.entries.iter().enumerate().filter(|(_, e)| e.archive) {
                verify_entry(&j, e)?;
                copy_tree(
                    &j.source(e),
                    &to.join(format!("{i}-{}", e.name)),
                    &excluded,
                    &mut inventory,
                    0,
                )?;
            }
            if let Some(bytes) = &j.old_bootstrap {
                write_new(&to.join("bootstrap.json"), bytes)?;
                if read_private(&to.join("bootstrap.json"), LIMIT)? != *bytes {
                    return Err("旧起動設定の退避内容が一致しません".into());
                }
            }
            for f in inventory {
                if fingerprint(&f.path)? != f {
                    return Err("退避完了前に元データが変更されています".into());
                }
            }
            let archived: Vec<_> = j.entries.iter().enumerate().filter(|(_,e)|e.archive).map(|(i,e)|serde_json::json!({"source":j.source(e),"archive":format!("{i}-{}",e.name)})).collect();
            write_new(&to.join("MON-reset-info.json"), &serde_json::to_vec(&serde_json::json!({"format":"MON raw reset archive", "operationId":id, "meetingRoot":s.root(), "modelsIncluded":false, "entries":archived})).map_err(err)?)?;
            std::fs::remove_file(to.join("MON-reset-incomplete.json")).map_err(err)?;
            crate::native_fs::sync_path(to)?;
            crate::native_fs::sync_path(parent)?;
        }
        progress("初期化の受付と作業領域を記録しています");
        lease.verify(s.root())?;
        for e in &j.entries {
            verify_entry(&j, e)?;
        }
        let (roots_now, entries_now) = entries(s)?;
        if roots_now != j.roots || entries_now != j.entries {
            return Err("退避中に初期化対象が変更されています。元データを保持します".into());
        }
        // Journal is durable before the marker or any source rename.
        j.phase = Phase::Staging;
        j.save()?;
        write_new(&s.root().join(PENDING), id.as_bytes())?;
        for root in 0..j.roots.len() {
            if j.workspaces.iter().any(|workspace| workspace.root == root) {
                continue;
            }
            let path = j.workspace(root);
            create_directory(&path)?;
            j.workspaces.push(Workspace {
                root,
                identity: Identity::read(&directory(&path)?)?,
            });
            j.save()?;
        }
        checkpoint("prepared")?;
        progress("旧データを初期化用の作業領域へ移しています");
        // Windows cannot rename a directory with an open child, even with
        // delete sharing. Startup IPC/controllers are already drained and the
        // exclusive root lease prevents another MON from starting a writer.
        // Existing helpers own pre-opened lease handles and were rejected above.
        #[cfg(windows)]
        drop(writer_leases.take());
        for (i, e) in j.entries.iter().enumerate() {
            b.guard_directory()?;
            lease.verify(s.root())?;
            verify_entry(&j, e)?;
            rename(&j.source(e), &j.target(i, e), false)?;
            checkpoint("move")?;
        }
        j.staged = true;
        j.phase = Phase::Initializing;
        j.save()?;
        // The writer-free directory is now inside the operation workspace.
        // Release handles before Windows removes its old directory entries.
        drop(writer_leases.take());
        checkpoint("staged")?;
        progress("新しいDBと初期設定を作成しています");
        // Reserve every SQLite-created file BEFORE opening it. Recovery owns
        // only these identities; it never erases a subsequently replaced file.
        for name in &fresh_names()[..4] {
            let root = j
                .roots
                .iter()
                .position(|r| r.path == s.root())
                .ok_or("会議保存先がありません")?;
            let reserved = j.workspace(root).join(format!("new-{name}"));
            write_new(&reserved, b"")?;
            j.fresh.push(Fresh {
                name: (*name).into(),
                identity: Identity::read(&meta(&reserved)?)?,
                app_data: false,
            });
            j.save()?;
            checkpoint("fresh_reserved")?;
            rename(&reserved, &s.root().join(name), false)?;
            checkpoint("fresh_installed")?;
        }
        let storage =
            Storage::open_for_reset(s.root().into(), s.app_data().join("models"), lease.clone())
                .map_err(err)?;
        storage.validate_startup_reset()?;
        if let Some(recovered) = &recovered {
            progress("回収したデータを初期化後の保存先へ再登録しています");
            for (name, source, app_data) in [
                ("meetings", recovered.root.join("meetings"), false),
                ("voices", recovered.root.join("voices"), false),
                ("models", recovered.models.clone(), true),
            ] {
                if !exists(&source)? {
                    continue;
                }
                let fresh = Fresh {
                    name: name.into(),
                    identity: Identity::read(&directory(&source)?)?,
                    app_data,
                };
                let destination = j.fresh_path(&fresh);
                j.fresh.push(fresh);
                j.save()?;
                checkpoint("restore_reserved")?;
                rename(&source, &destination, false)?;
                checkpoint("restore_installed")?;
            }
            recovered.install(&storage)?;
            checkpoint("restored")?;
        }
        let current = Root::bound(&storage)?;
        let revision = b
            .state
            .lock()
            .map_err(err)?
            .loaded
            .as_ref()
            .map_or(Ok(0), |l| {
                l.record
                    .revision
                    .checked_add(1)
                    .ok_or("起動設定の版番号上限です")
            })?;
        let record = Record {
            revision,
            origin: Origin::Default,
            current,
            pending_move: None,
            activation: None,
            recovery: None,
        };
        let bytes = serde_json::to_vec(&Document {
            schema_version: 3,
            sha256: hash(&serde_json::to_vec(&record).map_err(err)?),
            record: record.clone(),
        })
        .map_err(err)?;
        j.new_bootstrap = Some(hash(&bytes));
        j.phase = Phase::Publishing;
        j.save()?;
        checkpoint("database")?;
        progress("保存先の切替を確定しています");
        let mut state = b.state.lock().map_err(err)?;
        // Existing bootstrap guard and atomic writer remain authoritative.
        if state.loaded.is_none() {
            let mut lock = b.lock.try_clone().map_err(err)?;
            lock.seek(SeekFrom::Start(0))
                .and_then(|_| lock.write_all(STAMP))
                .and_then(|_| crate::native_fs::sync_handle(&lock))
                .map_err(err)?;
        }
        b.write(&mut state, record)?;
        drop(state);
        checkpoint("published")?;
        j.committed = true;
        j.phase = Phase::Cleanup;
        j.save()?;
        drop(storage);
        checkpoint("committed")?;
        progress(if restore {
            "データ復元を確定しています"
        } else {
            "旧データとモデル本体を削除しています"
        });
        drop(recovered);
        finish(&j)?;
        Ok(())
    })();
    if let Err(error) = result {
        // Simulated process termination must leave the record untouched.
        #[cfg(test)]
        if error == "reset test interruption" {
            return Err(error);
        }
        progress("初期化の状態を確認しています");
        let was_committed = committed(&j).unwrap_or(false);
        match recover_record(&j, &lease) {
            Ok(()) => {
                if was_committed {
                    Ok(())
                } else {
                    Err(error)
                }
            }
            Err(recovery) => Err(format!(
                "{error}\n初期化の回復が必要です。再起動後に再確認します: {recovery}"
            )),
        }
    } else {
        Ok(())
    }
}
fn committed(j: &Journal) -> Result<bool, String> {
    let path = j.config.path.join(NAME);
    let actual = if exists(&path)? {
        Some(hash(&read_private(&path, LIMIT)?))
    } else {
        None
    };
    let old = j.old_bootstrap.as_ref().map(|b| hash(b));
    if j.new_bootstrap.is_some() && actual == j.new_bootstrap {
        return Ok(true);
    }
    if actual != old || j.committed {
        return Err("起動設定が初期化記録と一致しません。新しいデータを保持します".into());
    }
    Ok(false)
}
fn remove_owned(p: &Path, id: &Identity) -> Result<(), String> {
    let m = meta(p)?;
    if !id.matches(&m) {
        return Err("初期化ファイルが差し替えられています。保持します".into());
    }
    crate::native_fs::remove_owned(p, &m).map_err(err)
}
fn clear_record(j: &Journal) -> Result<(), String> {
    let marker = j.meeting.path.join(PENDING);
    if exists(&marker)? {
        if read_private(&marker, 128)? != j.id.as_bytes() {
            return Err("初期化の受付記録が一致しません".into());
        }
        std::fs::remove_file(marker).map_err(err)?;
        crate::native_fs::sync_path(&j.meeting.path)?;
    }
    std::fs::remove_file(j.path()).map_err(err)?;
    crate::native_fs::sync_path(&j.config.path)
}
fn finish(j: &Journal) -> Result<(), String> {
    j.verify_roots()?;
    if !committed(j)? {
        return Err("新DBへの切替を確認できません".into());
    }
    // Bind to the committed DB identity without admitting an old schema.
    let doc: Document =
        serde_json::from_slice(&read_private(&j.config.path.join(NAME), LIMIT)?).map_err(err)?;
    doc.record.current.verify()?;
    for w in &j.workspaces {
        let p = j.workspace(w.root);
        if exists(&p)? {
            crate::storage::deletion::remove_reset_workspace(
                &p,
                (w.identity.device, w.identity.inode, w.identity.birth_ns),
            )?;
            checkpoint("cleanup")?;
        }
    }
    clear_record(j)
}
fn recover_record(j: &Journal, lease: &RootLease) -> Result<(), String> {
    j.verify_roots()?;
    lease.verify(&j.meeting.path)?;
    if committed(j)? {
        return finish(j);
    }
    // Unexpected SQLite files could belong to a newer database generation.
    // Leave the entire transaction intact rather than attach them to the old DB.
    let candidates = fresh_names()
        .into_iter()
        .map(|name| j.meeting.path.join(name))
        .chain([
            j.meeting.path.join("meetings"),
            j.meeting.path.join("voices"),
            j.data.path.join("models"),
        ]);
    for path in candidates {
        if exists(&path)?
            && !j.fresh.iter().any(|f| {
                j.fresh_path(f) == path && meta(&path).is_ok_and(|m| j.matches_fresh(f, &m))
            })
            && !j
                .entries
                .iter()
                .any(|e| j.source(e) == path && meta(&path).is_ok_and(|m| e.identity.matches(&m)))
        {
            return Err(
                "新しく作成されたDB関連ファイルを保持します。元データの切戻しは停止しました".into(),
            );
        }
    }
    // Preflight every source/target BEFORE rolling back any rename.
    for (i, e) in j.entries.iter().enumerate() {
        let old = j.source(e);
        let staged = j.target(i, e);
        if exists(&staged)? {
            if !e.identity.matches(&meta(&staged)?) {
                return Err("退避作業領域が変更されています".into());
            }
            if exists(&old)?
                && !j.fresh.iter().any(|f| {
                    old == j.fresh_path(f) && meta(&old).is_ok_and(|m| j.matches_fresh(f, &m))
                })
            {
                return Err("初期化後に別データが作られています。上書きしません".into());
            }
        } else if !exists(&old)? || !e.identity.matches(&meta(&old)?) {
            return Err("元データの所在を確認できません".into());
        }
    }
    for f in &j.fresh {
        let p = j.fresh_path(f);
        if exists(&p)? {
            let metadata = meta(&p)?;
            if j.matches_fresh(f, &metadata) {
                if metadata.is_dir() {
                    // Move only our recorded new tree out of the way. Rollback
                    // retains it with the private workspace rather than erasing
                    // any unconfirmed content while returning the originals.
                    rename(&p, &j.fresh_workspace(f)?, false)?;
                } else {
                    remove_owned(&p, &Identity::read(&metadata)?)?;
                }
            } else if !j
                .entries
                .iter()
                .any(|e| j.source(e) == p && meta(&p).is_ok_and(|m| e.identity.matches(&m)))
            {
                return Err("新DBのファイルが差し替えられています。保持します".into());
            }
        }
        for w in &j.workspaces {
            let reserved = j.workspace(w.root).join(format!("new-{}", f.name));
            if exists(&reserved)? {
                if !meta(&reserved)?.is_dir() {
                    remove_owned(&reserved, &f.identity)?;
                }
            }
        }
    }
    for (i, e) in j.entries.iter().enumerate().rev() {
        let p = j.target(i, e);
        if exists(&p)? {
            rename(&p, &j.source(e), false)?;
        }
    }
    // Stamp may have been written immediately before an interrupted first publish.
    if j.old_bootstrap.is_none() {
        let mut lock = open_file(&j.config.path.join(LOCK), libc::O_RDWR)?;
        lock.set_len(0).map_err(err)?;
        lock.flush().map_err(err)?;
        crate::native_fs::sync_handle(&lock).map_err(err)?;
    }
    for w in &j.workspaces {
        let p = j.workspace(w.root);
        if exists(&p)? {
            if !w.identity.matches(&directory(&p)?) {
                return Err("作業領域が差し替えられています".into());
            }
            if std::fs::read_dir(&p).map_err(err)?.next().is_some() {
                // An interrupted file allocation may precede its identity
                // receipt. Preserve such unconfirmed files, never infer ownership.
                let retained = p.with_file_name(format!("MON-reset-retained-{}", j.id));
                rename(&p, &retained, false)?;
            } else {
                std::fs::remove_dir(&p).map_err(err)?;
            }
            crate::native_fs::sync_path(p.parent().unwrap())?;
        }
    }
    clear_record(j)
}
pub(crate) fn recover(config: &Path, data: &Path) -> Result<(), String> {
    let path = config.join(JOURNAL);
    if !exists(&path)? {
        return Ok(());
    }
    // Acquire the bootstrap lock without normal root/schema admission.
    // A first publication can have an in-flight stamp/document mismatch.
    let config = config.canonicalize().map_err(err)?;
    let data = data.canonicalize().map_err(err)?;
    let lock = open_file(&config.join(LOCK), libc::O_RDWR)?;
    if !regular(&crate::native_fs::metadata(&lock).map_err(err)?, true) {
        return Err("初期化の排他ファイルが不正です".into());
    }
    lock.try_lock()
        .map_err(|e| format!("初期化の回復は他のアプリが終了してから行います: {e}"))?;
    let lock = BootstrapLock(lock);
    let (digest, j): (String, Journal) =
        serde_json::from_slice(&read_private(&path, MAX_RECORD)?).map_err(err)?;
    if digest != hash(&serde_json::to_vec(&j).map_err(err)?) {
        return Err("初期化記録のSHAが一致しません".into());
    }
    j.validate(&config, &data)?;
    let lease = RootLease::acquire(&j.meeting.path, true)?;
    let result = recover_record(&j, &lease);
    drop(lock);
    result
}
#[cfg(not(test))]
fn checkpoint(_: &str) -> Result<(), String> {
    Ok(())
}
#[cfg(test)]
thread_local! {static FAIL:std::cell::RefCell<Option<String>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
fn checkpoint(stage: &str) -> Result<(), String> {
    FAIL.with(|f| {
        if stage == "copy_write" && f.borrow().as_deref() == Some("disk_full") {
            Err("退避先への書込に失敗しました: 容量不足（試験）".into())
        } else if stage == "copy_write" && f.borrow().as_deref() == Some("copy_failure") {
            Err("退避先へのコピーに失敗しました（試験）".into())
        } else if f.borrow().as_deref() == Some(stage) {
            Err("reset test interruption".into())
        } else {
            Ok(())
        }
    })
}
#[cfg(test)]
mod tests;
