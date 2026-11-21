//! Cold salvage owns the startup lock and an exclusive lease on the original
//! meeting directory. SQLite is only opened against the private copied image.
use super::*;
use crate::native_fs;
use crate::storage::salvage::{self, Plan, Preview, ResultSummary};
use std::sync::Arc;

mod source_cleanup;

const JOURNAL: &str = ".salvage-operation.json";
const CONTROL: [&str; 4] = [
    "meetings.sqlite3",
    "meetings.sqlite3-wal",
    "meetings.sqlite3-shm",
    "meetings.sqlite3-journal",
];
const OP_LIMIT: u64 = 128 * 1024;
const PLAN_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SelectionRecord {
    operation_id: String,
    models: PathBuf,
    pub(super) models_identity: Identity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_cleanup_sha256: Option<String>,
}

impl SelectionRecord {
    pub(super) fn validate(&self, meeting_root: &Path) -> Result<(), String> {
        absolute(&self.models)?;
        if !uuid(&self.operation_id)
            || self.models.starts_with(meeting_root)
            || meeting_root.starts_with(&self.models)
            || self
                .source_cleanup_sha256
                .as_deref()
                .is_some_and(|digest| !sha(digest))
        {
            return Err("復元したモデル保存先の登録情報が不正です".into());
        }
        Ok(())
    }

    pub(crate) fn models_root(&self) -> &Path {
        &self.models
    }

    pub(crate) fn verify_models(&self) -> Result<(), String> {
        if !self.models_identity.matches(&directory(&self.models)?) {
            return Err(
                "復元したモデルのフォルダーが変わりました。モデルの保存先を確認してください".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SelectionFile {
    identity: Identity,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct OwnedDirectory {
    path: PathBuf,
    identity: Identity,
}

impl OwnedDirectory {
    fn bind(path: PathBuf) -> Result<Self, String> {
        let identity = Identity::read(&directory(&path)?)?;
        Ok(Self { path, identity })
    }

    fn create(path: PathBuf) -> Result<Self, String> {
        reset::create_directory(&path)?;
        Self::bind(path)
    }

    fn verify(&self) -> Result<(), String> {
        if !self.identity.matches(&directory(&self.path)?) {
            return Err("復元に使うフォルダーが変わりました。復元先を確認してください".into());
        }
        Ok(())
    }

    fn remove_private_workspace(&self) -> Result<(), String> {
        self.verify()?;
        let m = native_fs::symlink_metadata(&self.path).map_err(err)?;
        if m.uid() != native_fs::current_owner()? || m.mode() & 0o077 != 0 {
            return Err("復元用フォルダーの所有を確認できません。内容を保持します".into());
        }
        // The root was created exclusively, identity-bound in the private journal,
        // and used only for this operation. remove_dir_all does not follow links.
        std::fs::remove_dir_all(&self.path).map_err(err)?;
        native_fs::sync_directory(self.path.parent().ok_or("復元先の親がありません")?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    operation_id: String,
    previous: Option<SelectionFile>,
    source: Option<OwnedDirectory>,
    parent: Option<OwnedDirectory>,
    workspace: Option<OwnedDirectory>,
    scratch: Option<OwnedDirectory>,
    build: Option<OwnedDirectory>,
    plan_sha256: Option<String>,
    source_snapshot: Option<Vec<Option<Observed>>>,
}

impl Journal {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || !uuid(&self.operation_id) {
            return Err("復元操作記録の形式が不正です".into());
        }
        if let Some(source) = &self.source {
            absolute(&source.path)?;
        }
        if let Some(parent) = &self.parent {
            absolute(&parent.path)?;
        }
        if let Some(workspace) = &self.workspace {
            if self.source.is_none()
                || self.parent.as_ref().is_none_or(|parent| {
                    workspace.path
                        != parent
                            .path
                            .join(format!("MinutesOn-salvage-{}", self.operation_id))
                })
            {
                return Err("復元先の所有記録が一致しません".into());
            }
        }
        for (child, leaf) in [(&self.scratch, "work"), (&self.build, "output")] {
            if let Some(child) = child {
                if self
                    .workspace
                    .as_ref()
                    .is_none_or(|root| child.path != root.path.join(leaf))
                {
                    return Err("復元用フォルダーの所有記録が一致しません".into());
                }
            }
        }
        if self.plan_sha256.as_ref().is_some_and(|digest| {
            !sha(digest)
                || self.scratch.is_none()
                || self
                    .source_snapshot
                    .as_ref()
                    .is_none_or(|s| s.len() != CONTROL.len())
        }) {
            return Err("復元計画の確認記録が不正です".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Observed {
    identity: Identity,
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Observed {
    fn new(m: &Metadata, device: u64) -> Result<Self, String> {
        if !regular(m, false) || m.dev() != device {
            return Err("元の会議データのファイルを確認できませんでした".into());
        }
        Ok(Self {
            identity: Identity::read(m)?,
            len: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn exists(path: &Path) -> Result<bool, String> {
    match native_fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(err(error)),
    }
}

fn check_cancel(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        return Err("復元を取り消しました".into());
    }
    Ok(())
}

fn controls(source: &OwnedDirectory) -> Result<Vec<Option<Observed>>, String> {
    source.verify()?;
    CONTROL
        .iter()
        .map(
            |name| match native_fs::symlink_metadata(source.path.join(name)) {
                Ok(m) => Observed::new(&m, source.identity.device).map(Some),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(err(error)),
            },
        )
        .collect()
}

fn copy_controls(
    source: &OwnedDirectory,
    scratch: &OwnedDirectory,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<Option<Observed>>, String> {
    let before = controls(source)?;
    if before.first().is_none_or(Option::is_none) {
        return Err("選んだフォルダーに会議データがありません".into());
    }
    let required = before
        .iter()
        .enumerate()
        .filter_map(|(index, file)| (index != 2).then_some(file.as_ref()).flatten())
        .try_fold(0u64, |n, file| {
            n.checked_add(file.len)
                .ok_or("元の会議データが大きすぎてコピーできません")
        })?;
    if movement::capacity(&scratch.path)? < required {
        return Err("元の会議データをコピーする空き容量が不足しています。空き容量の多い復元先を選んでください".into());
    }
    crate::operation_observation::stage("copying_salvage_database", Some(required), "bytes");
    let mut block = vec![0; 1024 * 1024];
    for (index, (name, expected)) in CONTROL.iter().zip(&before).enumerate() {
        // The -shm file is an ephemeral WAL index. Its identity is checked, but
        // SQLite must rebuild the index for this independent copied DB/WAL.
        if index == 2 {
            continue;
        }
        let Some(expected) = expected else {
            continue;
        };
        check_cancel(cancelled)?;
        let mut input = reset::open_file(&source.path.join(name), libc::O_RDONLY)?;
        let observed = |file: &File| {
            Observed::new(
                &native_fs::metadata(file).map_err(err)?,
                source.identity.device,
            )
        };
        if observed(&input)? != *expected {
            return Err(
                "コピーを始める前に元の会議データが変わりました。復元元を確認してください".into(),
            );
        }
        scratch.verify()?;
        let mut output = reset::open_file(
            &scratch.path.join(name),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        let mut remaining = expected.len;
        while remaining > 0 {
            check_cancel(cancelled)?;
            let amount = remaining.min(block.len() as u64) as usize;
            input.read_exact(&mut block[..amount]).map_err(err)?;
            output.write_all(&block[..amount]).map_err(err)?;
            crate::operation_observation::read(amount);
            remaining -= amount as u64;
        }
        if input.read(&mut [0; 1]).map_err(err)? != 0 || observed(&input)? != *expected {
            return Err("コピー中に元の会議データが変わりました。復元元を確認してください".into());
        }
        native_fs::sync_handle(&output).map_err(err)?;
    }
    if controls(source)? != before {
        return Err("コピー中に元の会議データが変わりました。復元元を確認してください".into());
    }
    native_fs::sync_directory(&scratch.path)?;
    Ok(before)
}

fn read_selection(
    bootstrap: &Bootstrap,
) -> Result<(Option<SelectionFile>, Option<Vec<u8>>), String> {
    bootstrap.guard_directory()?;
    let path = bootstrap.config.join(NAME);
    if !exists(&path)? {
        return Ok((None, None));
    }
    let identity = Identity::read(&native_fs::symlink_metadata(&path).map_err(err)?)?;
    let bytes = reset::read_private(&path, OP_LIMIT)?;
    if !identity.matches(&native_fs::symlink_metadata(&path).map_err(err)?) {
        return Err("起動設定が読み取り中に変更されています".into());
    }
    bootstrap.guard_directory()?;
    Ok((
        Some(SelectionFile {
            identity,
            digest: hash(&bytes),
        }),
        Some(bytes),
    ))
}

#[cfg(test)]
fn requested(config: &Path) -> Result<bool, String> {
    exists(&config.join(JOURNAL))
}

pub(super) fn startup_intent(b: &Bootstrap) -> Result<bool, String> {
    let Some(journal) = read_journal(b)? else {
        return Ok(false);
    };
    if read_selection(b)?.0 == journal.previous {
        if let Some(workspace) = &journal.workspace {
            // A replaced workspace would make even a safe cancel impossible.
            // Source/destination choices can still be corrected in the session.
            workspace.verify()?;
        }
        return Ok(true);
    }
    let record = b
        .read()?
        .ok_or("復元操作に対応する起動設定がありません")?
        .record;
    let build = journal
        .build
        .as_ref()
        .ok_or("確定済み復元先の所有記録がありません")?;
    if record.current.path != build.path.join("data")
        || record.salvage.as_ref().is_none_or(|s| {
            s.operation_id != journal.operation_id || s.models != build.path.join("models")
        })
    {
        return Err("起動設定と復元操作の記録が一致しません".into());
    }
    let lease = reset::RootLease::acquire_for_salvage(&record.current.path)?;
    build.verify()?;
    record.current.verify()?;
    let result = salvage::read_result_from_path(&record.current.path.join(CONTROL[0]))?;
    lease.verify(&record.current.path)?;
    if result.is_none_or(|r| r.operation_id != journal.operation_id) {
        return Err("確定した復元結果の操作IDを確認できません".into());
    }
    Ok(true)
}

pub(super) fn journal_observation(b: &Bootstrap) -> Result<Observed, String> {
    b.guard_directory()?;
    let metadata = native_fs::symlink_metadata(b.config.join(JOURNAL)).map_err(err)?;
    if !regular(&metadata, true) || metadata.uid() != native_fs::current_owner()? {
        return Err("復元操作記録の所有を確認できません。記録を保持します".into());
    }
    Observed::new(&metadata, b.identity.device)
}

pub(super) fn retain_invalid_journal(b: &Bootstrap, observed: &Observed) -> Result<(), String> {
    if journal_observation(b)? != *observed {
        return Err("復元操作記録の実体が変わりました。記録を保持します".into());
    }
    let retained = b.config.join(format!(
        ".retained-salvage-operation-{}.json",
        Uuid::new_v4()
    ));
    reset::rename(&b.config.join(JOURNAL), &retained, false)?;
    crate::internal_logs::report_error(
        "salvage",
        &format!(
            "再開できない復元操作記録を保持しました: {}",
            retained.display()
        ),
    );
    Ok(())
}

fn read_journal(b: &Bootstrap) -> Result<Option<Journal>, String> {
    b.guard_directory()?;
    let path = b.config.join(JOURNAL);
    if !exists(&path)? {
        return Ok(None);
    }
    let stored: Journal =
        serde_json::from_slice(&reset::read_private(&path, OP_LIMIT)?).map_err(err)?;
    stored.validate()?;
    Ok(Some(stored))
}

fn save_journal(b: &Bootstrap, j: &Journal, previous: Option<&Journal>) -> Result<(), String> {
    j.validate()?;
    if read_journal(b)?.as_ref() != previous {
        return Err("復元操作記録が変更されています".into());
    }
    let temporary = b.config.join(format!(".salvage-{}.tmp", Uuid::new_v4()));
    reset::write_new(&temporary, &serde_json::to_vec(j).map_err(err)?)?;
    b.guard_directory()?;
    if read_journal(b)?.as_ref() != previous {
        return Err("復元操作記録が更新されています".into());
    }
    reset::rename(&temporary, &b.config.join(JOURNAL), previous.is_some())
}

fn initial(b: &Bootstrap, registered: Option<&Loaded>) -> Result<Journal, String> {
    let (previous, _) = read_selection(b)?;
    let source = registered.and_then(|loaded| {
        let current = &loaded.record.current;
        directory(&current.path)
            .ok()
            .filter(|m| current.directory.matches(m))
            .map(|_| OwnedDirectory {
                path: current.path.clone(),
                identity: current.directory.clone(),
            })
    });
    Ok(Journal {
        version: 1,
        operation_id: Uuid::new_v4().to_string(),
        previous,
        source,
        parent: None,
        workspace: None,
        scratch: None,
        build: None,
        plan_sha256: None,
        source_snapshot: None,
    })
}

impl Bootstrap {
    fn require_ready_salvage_root(&self, state: &State, storage: &Storage) -> Result<(), String> {
        self.guard(state)?;
        let record = &state.loaded.as_ref().ok_or("起動設定が未確定です")?.record;
        if record.pending_move.is_some()
            || self
                .restart_required
                .load(std::sync::atomic::Ordering::Acquire)
            || self.selected != record.current.path
            || storage.root() != record.current.path
        {
            return Err("現在の保存先からデータ復元へ切り替えられません".into());
        }
        record.current.verify()?;
        storage.verify_bootstrap_database()
    }

    pub(crate) fn require_ready_salvage(&self, storage: &Storage) -> Result<(), String> {
        if !storage.ready_salvage_available() {
            return Err("現在の書込み制限ではデータ復元を開始できません".into());
        }
        let state = self.state.lock().map_err(err)?;
        self.require_ready_salvage_root(&state, storage)?;
        if exists(&self.config.join(JOURNAL))? {
            return Err("先に復元操作の記録を確認してください".into());
        }
        Ok(())
    }

    /// Called after the ordinary close lifecycle has sealed work admission. The
    /// successor takes the mode before opening DBs.
    pub(crate) fn request_ready_salvage(&self, storage: &Storage) -> Result<(), String> {
        if !storage.ready_salvage_available() {
            return Err("現在の書込み制限ではデータ復元を開始できません".into());
        }
        let state = self.state.lock().map_err(err)?;
        self.require_ready_salvage_root(&state, storage)?;
        self.request_salvage_locked(&state)
    }

    #[cfg(test)]
    pub(crate) fn request_salvage(&self) -> Result<(), String> {
        let state = self.state.lock().map_err(err)?;
        self.guard(&state)?;
        self.request_salvage_locked(&state)
    }

    fn request_salvage_locked(&self, state: &State) -> Result<(), String> {
        if read_journal(self)?.is_some() {
            return Err(
                "前回のデータ復元が残っています。アプリを起動し直して復元画面を開いてください"
                    .into(),
            );
        }
        save_journal(self, &initial(self, state.loaded.as_ref())?, None)
    }

    pub(crate) fn undo_salvage_request(&self) -> Result<(), String> {
        if let Some(journal) = read_journal(self)? {
            if journal.workspace.is_none() {
                return remove_journal(self, &journal);
            }
        }
        Err("復元の受付状況を再起動後に確認してください".into())
    }
}

fn remove_journal(b: &Bootstrap, expected: &Journal) -> Result<(), String> {
    let path = b.config.join(JOURNAL);
    let metadata = native_fs::symlink_metadata(&path).map_err(err)?;
    if read_journal(b)?.as_ref() != Some(expected) {
        return Err("復元操作記録が変更されています".into());
    }
    native_fs::remove_owned(&path, &metadata).map_err(err)?;
    native_fs::sync_handle(&b.directory).map_err(err)
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    operation_id: Option<String>,
    source_root: Option<PathBuf>,
    destination_root: Option<PathBuf>,
    available_bytes: Option<u64>,
    source_selectable: bool,
    preview: Option<Preview>,
    result: Option<ResultSummary>,
}

pub(crate) struct Session {
    b: Bootstrap,
    source_models: Option<PathBuf>,
    registered_source: Option<PathBuf>,
    journal: Journal,
    lease: Option<Arc<reset::RootLease>>,
}

impl Session {
    pub(crate) fn open(config: &Path, app_data: &Path) -> Result<Self, String> {
        let (b, stamp) = Bootstrap::lock_configuration(config, app_data)?;
        let registered = if stamp == STAMP {
            b.read().ok().flatten()
        } else {
            None
        };
        let mut source_models = Some(b.models_root.clone());
        if let Some(model) = registered.as_ref().and_then(|l| l.record.salvage.as_ref()) {
            source_models = model.verify_models().ok().map(|_| model.models.clone());
        }
        if let Some(path) = &mut source_models {
            if let Ok(resolved) = path.canonicalize() {
                *path = resolved;
            }
        }
        let registered_source = registered
            .as_ref()
            .map(|loaded| loaded.record.current.path.clone());
        let journal = match read_journal(&b)? {
            Some(journal) => journal,
            None => {
                let journal = initial(&b, registered.as_ref())?;
                save_journal(&b, &journal, None)?;
                journal
            }
        };
        Ok(Self {
            b,
            source_models,
            registered_source,
            journal,
            lease: None,
        })
    }

    fn update(&mut self, change: impl FnOnce(&mut Journal)) -> Result<(), String> {
        let mut next = self.journal.clone();
        change(&mut next);
        save_journal(&self.b, &next, Some(&self.journal))?;
        self.journal = next;
        Ok(())
    }

    fn committed(&self) -> Result<Option<Record>, String> {
        let Some(current) = self.b.read().ok().flatten().map(|l| l.record) else {
            return Ok(None);
        };
        if current
            .salvage
            .as_ref()
            .is_none_or(|s| s.operation_id != self.journal.operation_id)
        {
            return Ok(None);
        }
        let build = self
            .journal
            .build
            .as_ref()
            .ok_or("確定済み復元先の所有記録がありません")?;
        if current.current.path != build.path.join("data")
            || current
                .salvage
                .as_ref()
                .is_none_or(|s| s.models != build.path.join("models"))
        {
            return Err("起動設定と確定済み復元先が一致しません".into());
        }
        current.current.verify()?;
        Ok(Some(current))
    }

    fn result(&self) -> Result<Option<ResultSummary>, String> {
        let Some(record) = self.committed()? else {
            return Ok(None);
        };
        let result = salvage::read_result_from_path(&record.current.path.join(CONTROL[0]))?
            .ok_or("確定した復元結果が新DBにありません")?;
        if result.operation_id != self.journal.operation_id {
            return Err("確定した復元結果の操作IDが一致しません".into());
        }
        Ok(Some(result))
    }

    fn plan(&self) -> Result<Option<Plan>, String> {
        let Some(digest) = self.journal.plan_sha256.as_ref() else {
            return Ok(None);
        };
        let scratch = self
            .journal
            .scratch
            .as_ref()
            .ok_or("復元計画がありません")?;
        scratch.verify()?;
        let bytes = reset::read_private(&scratch.path.join("plan.json"), PLAN_LIMIT)?;
        if hash(&bytes) != *digest {
            return Err("確認済みの復元計画が変更されています".into());
        }
        Ok(Some(serde_json::from_slice(&bytes).map_err(err)?))
    }

    pub(crate) fn view(&self) -> Result<View, String> {
        let result = self.result()?;
        Ok(View {
            operation_id: Some(self.journal.operation_id.clone()),
            source_root: self.journal.source.as_ref().map(|s| s.path.clone()),
            destination_root: self.journal.parent.as_ref().map(|parent| {
                parent
                    .path
                    .join(format!("MinutesOn-salvage-{}", self.journal.operation_id))
                    .join("output")
                    .join("data")
            }),
            available_bytes: self
                .journal
                .parent
                .as_ref()
                .and_then(|p| movement::capacity(&p.path).ok()),
            source_selectable: self.registered_source.is_none() && self.journal.workspace.is_none(),
            preview: if result.is_none() {
                self.plan()?.map(|p| p.preview)
            } else {
                None
            },
            result,
        })
    }

    pub(crate) fn select(&mut self, path: PathBuf, source: bool) -> Result<View, String> {
        if self.committed()?.is_some() {
            return Err("切替済みの復元先は変更できません".into());
        }
        if source && self.journal.workspace.is_some() {
            return Err("確認済みの復元元を変更するには復元を取り消してください".into());
        }
        if source && self.registered_source.is_some() {
            return Err("確認できた起動設定の保存先を使用してください".into());
        }
        let metadata = native_fs::symlink_metadata(&path).map_err(err)?;
        if !metadata.is_dir() {
            return Err("通常のローカルフォルダーを選択してください".into());
        }
        let path = path.canonicalize().map_err(err)?;
        local(&path)?;
        let selected = OwnedDirectory::bind(path)?;
        if source {
            if !exists(&selected.path.join(CONTROL[0]))? {
                return Err("選んだフォルダーに会議データがありません".into());
            }
            self.update(|j| j.source = Some(selected))?;
        } else if self.journal.parent.as_ref() != Some(&selected) {
            if let Some(original) = self.journal.source.as_ref() {
                self.require_separate_destination(&selected, original)?;
            }
            if let Some(workspace) = &self.journal.workspace {
                if selected.path.starts_with(&workspace.path) {
                    return Err("今回の復元用フォルダーの外にある保存先を選択してください".into());
                }
                self.require_unpublished_selection()?;
                // Invalidate consent before removing its workspace. A restart
                // during cleanup leaves an open operation without a preview.
                self.update(|j| {
                    j.plan_sha256 = None;
                    j.source_snapshot = None;
                })?;
                self.discard_unpublished_workspace()?;
                self.update(|j| {
                    j.operation_id = Uuid::new_v4().to_string();
                    j.parent = Some(selected);
                    j.workspace = None;
                    j.scratch = None;
                    j.build = None;
                })?;
            } else {
                self.update(|j| j.parent = Some(selected))?;
            }
        }
        self.view()
    }

    fn require_separate_destination(
        &self,
        parent: &OwnedDirectory,
        source: &OwnedDirectory,
    ) -> Result<(), String> {
        let destination = parent
            .path
            .join(format!("MinutesOn-salvage-{}", self.journal.operation_id));
        if destination.starts_with(&source.path)
            || destination.starts_with(&self.b.config)
            || self
                .source_models
                .as_ref()
                .is_some_and(|models| destination.starts_with(models))
        {
            return Err("元データ・モデル・起動設定の外にある保存先を選択してください".into());
        }
        Ok(())
    }

    fn guard_inputs(&mut self) -> Result<(OwnedDirectory, OwnedDirectory), String> {
        if read_selection(&self.b)?.0 != self.journal.previous {
            return Err("起動設定が確認後に変更されています。元データを保持します".into());
        }
        let source = self
            .journal
            .source
            .clone()
            .ok_or("元の会議保存先を確認してください")?;
        let parent = self
            .journal
            .parent
            .clone()
            .ok_or("新しい保存先の親フォルダーを選択してください")?;
        source.verify()?;
        parent.verify()?;
        self.require_separate_destination(&parent, &source)?;
        if self.lease.is_none() {
            self.lease = Some(reset::RootLease::acquire_for_salvage(&source.path)?);
        }
        self.lease
            .as_ref()
            .ok_or("復元元の排他がありません")?
            .verify(&source.path)?;
        if self
            .journal
            .source_snapshot
            .as_ref()
            .is_some_and(|snapshot| controls(&source).as_ref() != Ok(snapshot))
        {
            return Err("事前確認後に元の会議データが変更されています。復元計画を取り消して再確認してください".into());
        }
        Ok((source, parent))
    }

    pub(crate) fn inspect(&mut self, cancelled: &impl Fn() -> bool) -> Result<View, String> {
        if self.committed()?.is_some() || self.journal.plan_sha256.is_some() {
            return self.view();
        }
        let (source, parent) = self.guard_inputs()?;
        if let Some(workspace) = &self.journal.workspace {
            if !exists(&workspace.path)? {
                // A previous change of destination finished workspace cleanup
                // but stopped before recording the new selection.
                self.update(|j| {
                    j.workspace = None;
                    j.scratch = None;
                    j.build = None;
                })?;
            }
        }
        if self.journal.workspace.is_none() {
            let workspace = OwnedDirectory::create(
                parent
                    .path
                    .join(format!("MinutesOn-salvage-{}", self.journal.operation_id)),
            )?;
            self.update(|j| j.workspace = Some(workspace))?;
        }
        let workspace = self
            .journal
            .workspace
            .as_ref()
            .ok_or("復元先がありません")?
            .clone();
        workspace.verify()?;
        if let Some(scratch) = self.journal.scratch.as_ref() {
            remove_if_present(scratch)?;
            self.update(|j| j.scratch = None)?;
        }
        let scratch = OwnedDirectory::create(workspace.path.join("work"))?;
        self.update(|j| j.scratch = Some(scratch.clone()))?;
        let source_snapshot = copy_controls(&source, &scratch, cancelled)?;
        let source_db = scratch.path.join(CONTROL[0]);
        let plan = salvage::inspect(
            &self.journal.operation_id,
            &source.path,
            self.source_models.as_deref(),
            &source_db,
            &scratch.path,
            cancelled,
        )?;
        check_cancel(cancelled)?;
        self.guard_inputs()?;
        scratch.verify()?;
        let bytes = serde_json::to_vec(&plan).map_err(err)?;
        if bytes.len() as u64 > PLAN_LIMIT {
            return Err("復元計画が保存できる容量を超えています".into());
        }
        reset::write_new(&scratch.path.join("plan.json"), &bytes)?;
        if controls(&source)? != source_snapshot {
            return Err("事前確認中に元の会議データが変更されています".into());
        }
        self.update(|j| {
            j.plan_sha256 = Some(hash(&bytes));
            j.source_snapshot = Some(source_snapshot);
        })?;
        self.view()
    }

    pub(crate) fn commit(
        &mut self,
        operation_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<View, String> {
        if self.journal.operation_id != operation_id {
            return Err("復元の確認対象が一致しません".into());
        }
        if self.committed()?.is_some() {
            self.confirm_publication()?;
            return self.view();
        }
        let plan = self.plan()?.ok_or("復元できる内容を確認してください")?;
        let (source, parent) = self.guard_inputs()?;
        if movement::capacity(&parent.path)? < plan.preview.required_bytes {
            return Err("復元するデータを保存する空き容量が不足しています。空き容量の多い復元先を選んでください".into());
        }
        let workspace = self
            .journal
            .workspace
            .as_ref()
            .ok_or("復元先がありません")?
            .clone();
        workspace.verify()?;
        self.remove_owned_config_stage(&self.config_stage())?;
        if let Some(attempt) = self.journal.build.as_ref() {
            remove_if_present(attempt)?;
            self.update(|j| j.build = None)?;
        }
        let build = OwnedDirectory::create(workspace.path.join("output"))?;
        self.update(|j| j.build = Some(build.clone()))?;
        let scratch = self
            .journal
            .scratch
            .as_ref()
            .ok_or("復元計画がありません")?
            .clone();
        scratch.verify()?;
        let root = build.path.join("data");
        let models = build.path.join("models");
        reset::create_directory(&root)?;
        reset::create_directory(&models)?;
        let result = match salvage::build(&plan, &scratch.path, &root, &models, cancelled) {
            Ok(result) => result,
            Err(error) => {
                remove_if_present(&build).map_err(|cleanup| {
                    format!("{error}。復元用の途中データを確認してください: {cleanup}")
                })?;
                self.update(|j| j.build = None)?;
                return Err(error);
            }
        };
        check_cancel(cancelled)?;
        let current = Root::inspect(&root)?;
        let models_identity = Identity::read(&directory(&models)?)?;
        let (original, original_bytes) = read_selection(&self.b)?;
        if original != self.journal.previous {
            return Err("起動設定が確認後に変更されています。元データを保持します".into());
        }
        let revision = self.b.read().ok().flatten().map_or(Ok(0), |loaded| {
            loaded
                .record
                .revision
                .checked_add(1)
                .ok_or("起動設定の版が上限です")
        })?;
        let source_cleanup_sha256 = match source_cleanup::prepare(
            &self.journal,
            &source,
            &build,
            &current,
            &plan,
            cancelled,
        ) {
            Ok(digest) => digest,
            Err(error) => {
                check_cancel(cancelled)?;
                crate::internal_logs::report_error("salvage_source_cleanup", &error);
                None
            }
        };
        let record = Record {
            revision,
            origin: Origin::Salvaged,
            current,
            pending_move: None,
            activation: None,
            recovery: None,
            salvage: Some(SelectionRecord {
                operation_id: operation_id.into(),
                models,
                models_identity,
                source_cleanup_sha256,
            }),
        };
        record.validate()?;
        let result_bytes = serde_json::to_vec(&result).map_err(err)?;
        replace_owned_file(&workspace, "result.json", &result_bytes)?;
        if let Some(bytes) = original_bytes {
            replace_owned_file(&workspace, "previous-bootstrap.json", &bytes)?;
        }
        native_fs::sync_directory(&build.path)?;
        native_fs::sync_directory(&workspace.path)?;
        self.guard_inputs()?;
        build.verify()?;
        record.current.verify()?;
        record.salvage.as_ref().unwrap().verify_models()?;
        check_cancel(cancelled)?;
        crate::operation_observation::stage("switching_salvage", None, "items");
        self.publish(record, cancelled)?;
        self.view()
    }

    fn publish(&self, record: Record, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        let bytes = serde_json::to_vec(&Document {
            schema_version: 3,
            sha256: hash(&serde_json::to_vec(&record).map_err(err)?),
            record,
        })
        .map_err(err)?;
        if bytes.len() as u64 > LIMIT {
            return Err("新しい起動設定が容量上限を超えています".into());
        }
        let temp = self.config_stage();
        // A previous process may have staged the exact bytes without publishing.
        self.remove_owned_config_stage(&temp)?;
        reset::write_new(&temp, &bytes)?;
        if read_selection(&self.b)?.0 != self.journal.previous {
            return Err("切替直前に起動設定が変更されています".into());
        }
        // If a rename took effect but its sync/response was lost, read the
        // replacement and sync it. Never roll the configuration back.
        publication_checkpoint("before")?;
        check_cancel(cancelled)?;
        let replaced = reset::rename(
            &temp,
            &self.b.config.join(NAME),
            self.journal.previous.is_some(),
        );
        publication_checkpoint("after")?;
        if self.committed()?.is_some() {
            return self.confirm_publication();
        }
        replaced?;
        Err("新しい起動設定の確定を確認できません。復元記録を保持します".into())
    }

    fn config_stage(&self) -> PathBuf {
        self.b.config.join(format!(
            ".bootstrap-salvage-{}.tmp",
            self.journal.operation_id
        ))
    }

    fn remove_owned_config_stage(&self, temp: &Path) -> Result<(), String> {
        if exists(temp)? {
            let m = native_fs::symlink_metadata(temp).map_err(err)?;
            let previous: Document =
                serde_json::from_slice(&reset::read_private(temp, LIMIT)?).map_err(err)?;
            if previous
                .record
                .salvage
                .as_ref()
                .is_none_or(|s| s.operation_id != self.journal.operation_id)
                || self
                    .journal
                    .build
                    .as_ref()
                    .is_none_or(|b| previous.record.current.path != b.path.join("data"))
                || previous.sha256 != hash(&serde_json::to_vec(&previous.record).map_err(err)?)
            {
                return Err("未確定の起動設定を保持しています。復元操作を確認してください".into());
            }
            native_fs::remove_owned(temp, &m).map_err(err)?;
        }
        Ok(())
    }

    fn confirm_publication(&self) -> Result<(), String> {
        self.b.guard_directory()?;
        let mut lock = self.b.lock.try_clone().map_err(err)?;
        lock.seek(SeekFrom::Start(0))
            .and_then(|_| lock.write_all(STAMP))
            .and_then(|_| native_fs::sync_handle(&lock))
            .map_err(err)?;
        native_fs::sync_handle(&self.b.directory).map_err(err)
    }

    pub(crate) fn leave(&self) -> Result<(), String> {
        if self.committed()?.is_some() {
            self.confirm_publication()?;
            if let Some(scratch) = &self.journal.scratch {
                remove_if_present(scratch)?;
            }
        } else {
            self.discard_unpublished_workspace()?;
        }
        remove_journal(&self.b, &self.journal)
    }

    fn require_unpublished_selection(&self) -> Result<(), String> {
        if read_selection(&self.b)?.0 != self.journal.previous {
            return Err("切替結果を確認できません。元データと復元先を保持します".into());
        }
        Ok(())
    }

    fn discard_unpublished_workspace(&self) -> Result<(), String> {
        if let Some(workspace) = &self.journal.workspace {
            self.require_unpublished_selection()?;
            let temp = self.config_stage();
            self.remove_owned_config_stage(&temp)?;
            if exists(&workspace.path)? {
                workspace.verify()?;
                for owned in [self.journal.scratch.as_ref(), self.journal.build.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    remove_if_present(owned)?;
                }
                let report = workspace.path.join("result.json");
                remove_matching_file(&report, |bytes| {
                    serde_json::from_slice::<ResultSummary>(bytes).is_ok_and(|r| {
                        r.operation_id == self.journal.operation_id
                            && self.journal.build.as_ref().is_some_and(|b| {
                                Path::new(&r.destination_root) == b.path.join("data")
                            })
                    })
                })?;
                if let Some(previous) = &self.journal.previous {
                    remove_matching_file(
                        &workspace.path.join("previous-bootstrap.json"),
                        |bytes| hash(bytes) == previous.digest,
                    )?;
                }
                workspace.verify()?;
                match std::fs::remove_dir(&workspace.path) {
                    Ok(()) => native_fs::sync_directory(
                        workspace.path.parent().ok_or("復元先の親がありません")?,
                    )?,
                    Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                    Err(error) => return Err(err(error)),
                }
            }
        }
        Ok(())
    }
}

fn remove_if_present(owned: &OwnedDirectory) -> Result<(), String> {
    if exists(&owned.path)? {
        owned.remove_private_workspace()?;
    }
    Ok(())
}

fn remove_matching_file(
    path: &Path,
    observed_bytes: impl FnOnce(&[u8]) -> bool,
) -> Result<(), String> {
    if exists(path)? {
        let metadata = native_fs::symlink_metadata(path).map_err(err)?;
        if reset::read_private(path, PLAN_LIMIT).is_ok_and(|bytes| observed_bytes(&bytes)) {
            native_fs::remove_owned(path, &metadata).map_err(err)?;
        }
    }
    Ok(())
}

fn replace_owned_file(workspace: &OwnedDirectory, name: &str, bytes: &[u8]) -> Result<(), String> {
    workspace.verify()?;
    let path = workspace.path.join(name);
    if exists(&path)? {
        let previous = reset::read_private(&path, PLAN_LIMIT)?;
        if previous == bytes {
            return Ok(());
        }
        return Err("確定前に作成した復元記録と一致しません。以前の記録を保持します".into());
    }
    reset::write_new(&path, bytes)
}

#[cfg(not(test))]
fn publication_checkpoint(_: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static INTERRUPT_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn publication_checkpoint(phase: &str) -> Result<(), String> {
    if INTERRUPT_AT.get() == Some(phase) {
        Err("simulated interruption at atomic selection".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn source(base: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let config = base.join("config");
        let app_data = base.join("application");
        let b = Bootstrap::open(&config, &app_data).unwrap();
        let storage = b.open_storage().unwrap();
        storage.create_meeting("読み取れる会議").unwrap();
        storage.keep_sqlite_controls_on_close(true).unwrap();
        let root = storage.root().to_owned();
        b.request_salvage().unwrap();
        drop(storage);
        drop(b);
        (config, app_data, root)
    }

    #[test]
    fn ready_request_requires_shared_damage_and_the_current_stationary_storage() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let config = base.join("config");
        let data = base.join("application");
        let bootstrap = Bootstrap::open(&config, &data).unwrap();
        let storage = bootstrap.open_storage().unwrap();
        assert!(bootstrap.request_ready_salvage(&storage).is_err());
        assert!(!requested(&config).unwrap());
        let a = storage.create_meeting("会議A").unwrap();
        let b = storage.create_meeting("会議B").unwrap();
        crate::storage::with_test_database(&storage, |db| {
            let claim = serde_json::json!({"scope": {"meetingId": b.id}});
            db.execute("INSERT INTO maintenance_operations(id,meeting_id,kind,state,target_manifest_json,target_hash,progress_json,created_at,updated_at)
                        VALUES('conflict',?1,'delete_meeting','attention',?2,?3,'{}',0,0)",
                rusqlite::params![a.id, claim.to_string(), "a".repeat(64)]).unwrap();
        });
        drop(storage);
        drop(bootstrap);

        let bootstrap = Bootstrap::open(&config, &data).unwrap();
        let storage = bootstrap.open_storage().unwrap();
        assert!(storage.ready_salvage_available());
        let unrelated = Storage::open(base.join("unrelated")).unwrap();
        {
            let state = bootstrap.state.lock().unwrap();
            assert!(bootstrap
                .require_ready_salvage_root(&state, &unrelated)
                .is_err());
        }
        bootstrap.request_ready_salvage(&storage).unwrap();
        assert!(requested(&config).unwrap());
        bootstrap.undo_salvage_request().unwrap();

        let move_target = base.join("move-target");
        reset::create_directory(&move_target).unwrap();
        bootstrap
            .begin_move(
                &Uuid::new_v4().to_string(),
                &Uuid::new_v4().to_string(),
                &move_target,
                &"a".repeat(64),
                bootstrap.view().unwrap().revision,
            )
            .unwrap();
        assert!(bootstrap.request_ready_salvage(&storage).is_err());
        assert!(!requested(&config).unwrap());
    }

    #[test]
    fn interruption_on_each_side_of_cutover_reuses_the_plan_and_preserves_the_original() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let (config, app_data, source) = source(&base);
        let original = std::fs::read(config.join(NAME)).unwrap();
        let files: Vec<_> = CONTROL
            .iter()
            .map(|name| std::fs::read(source.join(name)).ok())
            .collect();
        let parent = base.join("destination");
        reset::create_directory(&parent).unwrap();

        let mut session = Session::open(&config, &app_data).unwrap();
        session.select(parent, false).unwrap();
        let preview = session.inspect(&|| false).unwrap().preview.unwrap();
        assert!(preview.recoverable_count > 0);
        let id = preview.operation_id;
        let mut plan = session.plan().unwrap().unwrap();
        let scratch = session.journal.scratch.as_ref().unwrap().path.clone();
        let plan_file = scratch.join("plan.json");
        let original_plan = std::fs::read(&plan_file).unwrap();
        plan.preview.required_bytes = u64::MAX;
        let too_large = serde_json::to_vec(&plan).unwrap();
        std::fs::write(&plan_file, &too_large).unwrap();
        session
            .update(|j| j.plan_sha256 = Some(hash(&too_large)))
            .unwrap();
        assert!(session
            .commit(&id, &|| false)
            .err()
            .is_some_and(|e| e.contains("空き容量")));
        assert_eq!(std::fs::read(config.join(NAME)).unwrap(), original);
        std::fs::write(&plan_file, &original_plan).unwrap();
        session
            .update(|j| j.plan_sha256 = Some(hash(&original_plan)))
            .unwrap();
        INTERRUPT_AT.set(Some("before"));
        assert!(session
            .commit(&id, &|| false)
            .err()
            .is_some_and(|e| e.contains("simulated")));
        assert_eq!(std::fs::read(config.join(NAME)).unwrap(), original);
        drop(session);

        let mut session = Session::open(&config, &app_data).unwrap();
        assert_eq!(session.view().unwrap().preview.unwrap().operation_id, id);
        INTERRUPT_AT.set(Some("after"));
        assert!(session
            .commit(&id, &|| false)
            .err()
            .is_some_and(|e| e.contains("simulated")));
        assert_ne!(std::fs::read(config.join(NAME)).unwrap(), original);
        let workspace = session.journal.workspace.as_ref().unwrap().path.clone();
        std::fs::remove_file(workspace.join("result.json")).unwrap();
        drop(session);

        INTERRUPT_AT.set(None);
        let mut session = Session::open(&config, &app_data).unwrap();
        let result = session.commit(&id, &|| false).unwrap().result.unwrap();
        assert_eq!(result.meeting_count, 1);
        assert_eq!(result.operation_id, id);
        let workspace = session.journal.workspace.as_ref().unwrap().path.clone();
        assert_eq!(
            std::fs::read(workspace.join("previous-bootstrap.json")).unwrap(),
            original
        );
        session.leave().unwrap();
        assert!(!workspace.join("work").exists());
        assert!(!requested(&config).unwrap());
        drop(session);
        let b = Bootstrap::open(&config, &app_data).unwrap();
        let storage = b.open_storage().unwrap();
        let meetings = storage.list_meetings().unwrap();
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].title, "読み取れる会議");
        assert_eq!(storage.salvage_result().unwrap().unwrap().operation_id, id);
        assert_eq!(
            files,
            CONTROL
                .iter()
                .map(|name| std::fs::read(source.join(name)).ok())
                .collect::<Vec<_>>()
        );
        let database = storage.root().join(CONTROL[0]);
        let model_root = b.models_root.clone();
        drop(storage);
        drop(b);
        let checked_database = std::fs::read(&database).unwrap();
        std::fs::rename(&model_root, model_root.with_file_name("retained-models")).unwrap();
        reset::create_directory(&model_root).unwrap();
        let b = Bootstrap::open_for_startup(&config, &app_data).unwrap();
        assert_eq!(std::fs::read(database).unwrap(), checked_database);
        let storage = b.open_storage().unwrap();
        assert_eq!(storage.list_meetings().unwrap()[0].title, "読み取れる会議");
        assert!(storage
            .create_meeting("モデル未接続でも保存できる会議")
            .is_ok());
        assert!(storage.native_models_root().is_err());
        assert!(storage.list_model_placements().is_err());
        let deletion = storage.list_model_deletion_items().unwrap();
        assert!(deletion.items.is_empty());
        assert!(!deletion.write_blocked);
    }

    #[test]
    fn corrupt_salvage_intent_keeps_normal_startup_and_preserves_the_record() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let (config, app_data, source) = source(&base);
        let pending = crate::startup_attention::admit_storage(&config, &app_data)
            .err()
            .expect("valid request resumes restoration");
        assert_eq!(
            serde_json::to_value(pending).unwrap()["stage"],
            "partial_salvage"
        );

        let corrupt = b"{ truncated";
        let journal = config.join(JOURNAL);
        std::fs::write(&journal, corrupt).unwrap();
        for _ in 0..2 {
            let (storage, _bootstrap) =
                crate::startup_attention::admit_storage(&config, &app_data).unwrap();
            assert_eq!(storage.root(), source);
            assert!(storage
                .list_meetings()
                .unwrap()
                .iter()
                .any(|m| m.title == "読み取れる会議"));
            assert!(storage.create_meeting("通常起動").is_ok());
            assert!(!journal.exists());
        }
        let retained = std::fs::read_dir(&config)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|file| {
                file.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with(".retained-salvage-operation-")
                })
            })
            .unwrap();
        assert_eq!(std::fs::read(retained).unwrap(), corrupt);
        let session = Session::open(&config, &app_data).unwrap();
        assert_eq!(session.view().unwrap().source_root.as_ref(), Some(&source));
    }

    #[test]
    fn invalid_salvage_intent_stays_active_if_the_registered_store_cannot_be_admitted() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let (config, app_data, source) = source(&base);
        let journal = config.join(JOURNAL);
        let corrupt = b"{ truncated";
        std::fs::write(&journal, corrupt).unwrap();
        let db = rusqlite::Connection::open(source.join(CONTROL[0])).unwrap();
        db.execute("UPDATE schema_migrations SET checksum='unrecognized'", [])
            .unwrap();
        drop(db);

        let admission = crate::startup_attention::admit_storage(&config, &app_data);
        let failure =
            serde_json::to_value(admission.err().expect("the schema is unsupported")).unwrap();
        assert_eq!(failure["stage"], "storage_admission");
        assert_eq!(failure["salvageAvailable"], false);
        assert_eq!(std::fs::read(journal).unwrap(), corrupt);
    }

    #[test]
    fn a_replaced_recovery_workspace_cannot_block_or_be_removed_by_normal_startup() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let (config, app_data, source) = source(&base);
        let parent = base.join("destination");
        reset::create_directory(&parent).unwrap();
        let mut session = Session::open(&config, &app_data).unwrap();
        session.select(parent, false).unwrap();
        let workspace = session.journal.parent.as_ref().unwrap().path.join(format!(
            "MinutesOn-salvage-{}",
            session.journal.operation_id
        ));
        let selected_workspace = OwnedDirectory::create(workspace.clone()).unwrap();
        session
            .update(|journal| journal.workspace = Some(selected_workspace))
            .unwrap();
        drop(session);

        let original = workspace.with_file_name("retained-workspace");
        std::fs::rename(&workspace, &original).unwrap();
        reset::create_directory(&workspace).unwrap();
        let foreign = workspace.join("unrelated.txt");
        std::fs::write(&foreign, b"not owned by recovery").unwrap();
        let (storage, _bootstrap) =
            crate::startup_attention::admit_storage(&config, &app_data).unwrap();
        assert_eq!(storage.root(), source);
        assert!(original.is_dir());
        assert_eq!(std::fs::read(foreign).unwrap(), b"not owned by recovery");
        assert!(!requested(&config).unwrap());
    }

    #[test]
    fn a_corrupt_selection_requires_an_explicit_source_and_cancel_leaves_it_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let (config, app_data, source) = source(&base);
        let previous = read_journal(&Bootstrap::lock_configuration(&config, &app_data).unwrap().0)
            .unwrap()
            .unwrap();
        let (b, _) = Bootstrap::lock_configuration(&config, &app_data).unwrap();
        remove_journal(&b, &previous).unwrap();
        drop(b);
        let invalid = b"{ invalid bootstrap document";
        std::fs::write(config.join(NAME), invalid).unwrap();
        let destination = base.join("destination");
        reset::create_directory(&destination).unwrap();

        let mut session = Session::open(&config, &app_data).unwrap();
        assert!(session.view().unwrap().source_selectable);
        session.select(source.clone(), true).unwrap();
        session.select(destination, false).unwrap();
        assert!(session.inspect(&|| true).is_err());
        let workspace = session.journal.workspace.as_ref().unwrap().path.clone();
        session.leave().unwrap();
        assert!(!workspace.exists());
        assert_eq!(std::fs::read(config.join(NAME)).unwrap(), invalid);
        assert!(source.join(CONTROL[0]).exists());
    }

    #[test]
    fn a_preexisting_destination_folder_is_never_claimed_or_removed() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let (config, app_data, _) = source(&base);
        let parent = base.join("destination");
        reset::create_directory(&parent).unwrap();
        let mut session = Session::open(&config, &app_data).unwrap();
        session.select(parent.clone(), false).unwrap();
        let unrelated = parent.join(format!(
            "MinutesOn-salvage-{}",
            session.journal.operation_id
        ));
        reset::create_directory(&unrelated).unwrap();
        let sentinel = unrelated.join("unrelated");
        std::fs::write(&sentinel, b"owned by somebody else").unwrap();

        assert!(session.inspect(&|| false).is_err());
        assert!(session.journal.workspace.is_none());
        session.leave().unwrap();
        assert_eq!(std::fs::read(sentinel).unwrap(), b"owned by somebody else");
    }

    #[test]
    fn a_checked_destination_can_be_changed_without_restarting() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let (config, app_data, source) = source(&base);
        let original = std::fs::read(source.join(CONTROL[0])).unwrap();
        let first = base.join("first");
        let second = base.join("second");
        reset::create_directory(&first).unwrap();
        reset::create_directory(&second).unwrap();
        let mut session = Session::open(&config, &app_data).unwrap();
        session.select(first, false).unwrap();
        let before = session.inspect(&|| false).unwrap().preview.unwrap();
        let workspace = session.journal.workspace.as_ref().unwrap().path.clone();
        assert!(session.select(source.clone(), false).is_err());
        assert!(session.select(workspace.clone(), false).is_err());
        assert_eq!(
            session.view().unwrap().preview.unwrap().operation_id,
            before.operation_id
        );

        let changed = session.select(second.clone(), false).unwrap();
        assert!(!workspace.exists());
        assert!(changed.destination_root.unwrap().starts_with(second));
        assert!(changed.preview.is_none());
        assert!(session.commit(&before.operation_id, &|| false).is_err());
        let after = session.inspect(&|| false).unwrap().preview.unwrap();
        assert_ne!(before.operation_id, after.operation_id);
        assert_eq!(before.recoverable_count, after.recoverable_count);
        assert_eq!(std::fs::read(source.join(CONTROL[0])).unwrap(), original);
        session.leave().unwrap();
    }
}
