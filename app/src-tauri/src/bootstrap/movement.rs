//! Server-owned folder selection. No IPC path, serialized progress, or receipt
//! can stand in for the live snapshot and placement used at publication.
use super::*;
use crate::data_controller::{MovePreview, MoveReceipt};

pub(crate) enum Prepared {
    New(NewMove),
    Pending(Box<super::move_recovery::Review>),
}
impl Prepared {
    pub(crate) fn preview(&self) -> MovePreview {
        match self {
            Self::New(p) => p.preview(),
            Self::Pending(p) => p.preview(),
        }
    }
    pub(crate) fn refresh(
        &mut self,
        b: &Bootstrap,
        s: &Storage,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<(), String> {
        match self {
            Self::New(p) => p.refresh(b, s, cancelled, progress),
            Self::Pending(p) => {
                *p = Box::new(b.prepare_pending_move(s, cancelled, progress)?);
                Ok(())
            }
        }
    }
    pub(crate) fn publish(
        &mut self,
        b: &Bootstrap,
        s: &Storage,
        confirmation: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<MoveReceipt, String> {
        match self {
            Self::New(p) => p.publish(b, s, confirmation, cancelled, progress),
            Self::Pending(p) => p.resume(b, s, confirmation, cancelled, progress),
        }
    }
    pub(crate) fn keep(
        &mut self,
        b: &Bootstrap,
        s: &Storage,
        confirmation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<MoveReceipt, String> {
        match self {
            Self::Pending(p) => p.keep(b, s, confirmation, cancelled),
            Self::New(_) => Err("未完了移動の保持確認ではありません".into()),
        }
    }
}
pub(crate) struct NewMove {
    preview: MovePreview,
    parent: PathBuf,
    parent_file: File,
    parent_identity: Identity,
    current: Root,
    revision: u64,
    manifest_hash: String,
}

fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("保存先変更を取り消しました。旧保存先を保持します".into())
    } else {
        Ok(())
    }
}

fn capacity(parent: &Path) -> Result<u64, String> {
    let path = std::ffi::CString::new(parent.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let stat = unsafe { stat.assume_init() };
    (stat.f_bavail as u64)
        .checked_mul(stat.f_frsize as u64)
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or_else(|| "保存先容量が範囲外です".into())
}

impl Bootstrap {
    pub(crate) fn prepare_move(
        &self,
        storage: &Storage,
        parent: &Path,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Prepared, String> {
        check(cancelled)?;
        // Resolve system aliases once, but bind the resolved parent thereafter.
        let parent = parent.canonicalize().map_err(|e| e.to_string())?;
        let metadata = directory(&parent)?;
        local(&parent)?;
        let parent_file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&parent)
            .map_err(|e| e.to_string())?;
        let parent_identity = Identity::read(&metadata)?;
        if !parent_identity.matches(&parent_file.metadata().map_err(|e| e.to_string())?) {
            return Err("選択した親フォルダーが変更されています".into());
        }
        let operation_id = Uuid::new_v4().to_string();
        let target = parent.join(format!("MON-meetings-{operation_id}"));
        let mut prepared = NewMove {
            preview: MovePreview {
                confirmation_id: String::new(),
                operation_id,
                source: storage.root().into(),
                destination: target,
                meeting_count: 0,
                file_count: None,
                required_bytes: 0,
                available_bytes: 0,
                pending: false,
                resume_allowed: true,
                resume_reason: None,
                copied_paths: None,
                expected_paths: None,
            },
            parent,
            parent_file,
            parent_identity,
            current: Root::bound(storage)?,
            revision: 0,
            manifest_hash: String::new(),
        };
        prepared.refresh(self, storage, cancelled, progress)?;
        Ok(Prepared::New(prepared))
    }
}

impl NewMove {
    pub(crate) fn preview(&self) -> MovePreview {
        self.preview.clone()
    }

    fn guard(&self, bootstrap: &Bootstrap, storage: &Storage) -> Result<u64, String> {
        let state = bootstrap
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        bootstrap.guard(&state)?;
        let record = &state.loaded.as_ref().ok_or("起動設定が未確定です")?.record;
        record.current.verify()?;
        if record.current != self.current
            || storage.root() != self.current.path
            || bootstrap.selected != self.current.path
            || storage.restart_required()
            || bootstrap
                .restart_required
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("保存先の確認が失効しています。再起動後に確認してください".into());
        }
        if record.pending_move.is_some() {
            return Err("未完了の保存先変更があります。旧保存先と途中コピーを保持します".into());
        }
        let target = &self.preview.destination;
        if target.starts_with(&self.current.path)
            || self.current.path.starts_with(target)
            || target.starts_with(&bootstrap.config)
            || bootstrap.config.starts_with(target)
            || target.starts_with(&bootstrap.models_root)
            || bootstrap.models_root.starts_with(target)
        {
            return Err("会議・起動設定・モデルの領域と重なる保存先は選べません".into());
        }
        if !self.parent_identity.matches(&directory(&self.parent)?)
            || !self
                .parent_identity
                .matches(&self.parent_file.metadata().map_err(|e| e.to_string())?)
        {
            return Err("確認した親フォルダーの実体が変更されています".into());
        }
        local(&self.parent)?;
        match std::fs::symlink_metadata(target) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
            Ok(_) => return Err("確認した新規フォルダー名は既に存在します。上書きしません".into()),
        }
        Ok(record.revision)
    }

    pub(crate) fn refresh(
        &mut self,
        bootstrap: &Bootstrap,
        storage: &Storage,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<(), String> {
        let revision = self.guard(bootstrap, storage)?;
        progress("verifying");
        let snapshot = storage.prepare_relocation_snapshot(cancelled)?;
        let manifest = snapshot.manifest();
        let size = manifest
            .entries
            .iter()
            .try_fold(manifest.database_size, |n, entry| {
                n.checked_add(entry.size).ok_or("移動容量が範囲外です")
            })?;
        let required = size
            .checked_mul(2)
            .and_then(|n| n.checked_add(64 * 1024 * 1024))
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or("移動容量が範囲外です")?;
        let available = capacity(&self.parent)?;
        check(cancelled)?;
        if self.guard(bootstrap, storage)? != revision {
            return Err("起動設定が確認中に変更されています".into());
        }
        let mut preview = self.preview.clone();
        preview.confirmation_id = Uuid::new_v4().to_string();
        preview.meeting_count = manifest
            .tables
            .iter()
            .find(|t| t.name == "meetings")
            .ok_or("会議の件数がありません")?
            .rows;
        preview.file_count = Some(manifest.entries.iter().filter(|e| !e.directory).count() + 1);
        preview.required_bytes = required;
        preview.available_bytes = available;
        self.manifest_hash = snapshot.manifest_hash().into();
        self.revision = revision;
        self.preview = preview;
        Ok(())
    }

    pub(crate) fn publish(
        &mut self,
        bootstrap: &Bootstrap,
        storage: &Storage,
        confirmation: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<MoveReceipt, String> {
        check(cancelled)?;
        if confirmation != self.preview.confirmation_id
            || self.guard(bootstrap, storage)? != self.revision
        {
            return Err("保存先変更の確認が古くなっています。再確認してください".into());
        }
        if capacity(&self.parent)? < self.preview.required_bytes {
            return Err("移動先の容量が不足しています。容量を確保して再確認してください".into());
        }
        progress("verifying");
        let snapshot = storage.prepare_relocation_snapshot(cancelled)?;
        if snapshot.manifest_hash() != self.manifest_hash {
            return Err("確認後に会議・編集・音声が変更されています。再確認してください".into());
        }
        if self.guard(bootstrap, storage)? != self.revision {
            return Err("起動設定が確認後に変更されています".into());
        }
        check(cancelled)?;
        // Only this fresh private child is created. Never chmod, adopt, overwrite,
        // or remove the folder selected in the system panel or any old source.
        let name = std::ffi::CString::new(
            self.preview
                .destination
                .file_name()
                .ok_or("新規フォルダー名がありません")?
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        if unsafe { libc::mkdirat(self.parent_file.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if !self.parent_identity.matches(&directory(&self.parent)?) {
            return Err(
                "新規フォルダーの作成中に親が変更されました。途中コピーを保持します".into(),
            );
        }
        File::open(&self.preview.destination)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        self.parent_file.sync_all().map_err(|e| e.to_string())?;
        let intent = bootstrap.begin_move(
            &self.preview.operation_id,
            confirmation,
            &self.preview.destination,
            snapshot.manifest_hash(),
            self.revision,
        )?;
        progress("copying");
        let mut placement = bootstrap.pending_placement(&self.preview.operation_id, &snapshot)?;
        placement.copy(&snapshot, cancelled)?;
        progress("handoff");
        let handoff = snapshot.prepare_control_handoff(cancelled)?;
        placement.install_history(&handoff, cancelled)?;
        progress("switching");
        let selected =
            placement.activate_for_restart(&handoff, confirmation, intent.revision, cancelled)?;
        Ok(MoveReceipt {
            operation_id: self.preview.operation_id.clone(),
            source: self.preview.source.clone(),
            destination: selected.meeting_root,
            restart_required: selected.restart_required,
            retained: false,
        })
    }
}

#[cfg(test)]
mod tests;
