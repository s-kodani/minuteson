//! Startup-only rebinding at the registered path. Historical ownership and
//! consent are archived, never rewritten into authority over copied files.
use super::*;
use std::os::unix::fs::DirBuilderExt;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Recovery {
    id: String,
    kind: String,
    previous_hash: String,
    source_observation: String,
    workspace_identity: Identity,
    pending: Vec<String>,
}

pub(crate) use crate::startup_attention::Recovery as RecoveryView;

impl Recovery {
    pub(super) fn validate(&self) -> Result<(), String> {
        if !uuid(&self.id)
            || !sha(&self.previous_hash)
            || !sha(&self.source_observation)
            || !matches!(self.kind.as_str(), "device_changed" | "copy_restored")
            || self.pending.len() > 32
            || self
                .pending
                .iter()
                .any(|s| s.is_empty() || s.len() > 256 || s.contains('\0'))
        {
            return Err("起動時の復旧記録が不正です".into());
        }
        Ok(())
    }
    fn workspace(&self, config: &Path) -> PathBuf {
        config.join(format!(".bootstrap-recovery-{}", self.id))
    }
    fn verify(&self, config: &Path) -> Result<PathBuf, String> {
        self.validate()?;
        let path = self.workspace(config);
        let metadata = directory(&path)?;
        if !self.workspace_identity.matches(&metadata) || metadata.mode() & 0o077 != 0 {
            return Err("復旧用作業領域の実体または権限が変更されています".into());
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path.join("previous-bootstrap.json"))
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !regular(&metadata, true) || metadata.len() > LIMIT {
            return Err("復旧前の起動設定の保存記録が不正です".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if hash(&bytes) != self.previous_hash {
            return Err("復旧前の起動設定の保存記録が変更されています".into());
        }
        Ok(path)
    }
    pub(super) fn view(&self, config: &Path) -> RecoveryView {
        RecoveryView {
            kind: self.kind.clone(),
            review_required: !self.pending.is_empty(),
            reasons: self.pending.clone(),
            record_path: self.workspace(config).join("previous-bootstrap.json"),
        }
    }
}

// Only Bootstrap constructs this process-local admission, after inspection.
pub(crate) struct Admission {
    root: Root,
    previous: Root,
    workspace: PathBuf,
    pending: Vec<String>,
    expected_observation: Option<String>,
}
pub(crate) struct ContentGuard {
    files: Vec<(PathBuf, Option<String>)>,
}
impl ContentGuard {
    pub(crate) fn verify(&self) -> Result<(), String> {
        for (path, expected) in &self.files {
            let now = content_proof(path)?;
            let unchanged = match (expected, now) {
                (Some(expected), Some((now, _))) => *expected == now,
                (None, None) => true,
                // SQLite can create its empty WAL on the first read of an
                // existing WAL-mode DB. It has not changed any committed data.
                (None, Some((_, 0))) => path
                    .file_name()
                    .is_some_and(|n| n == "meetings.sqlite3-wal"),
                _ => false,
            };
            if !unchanged {
                return Err("SQLite接続中に復旧対象のDBまたはWALが変更されています".into());
            }
        }
        Ok(())
    }
}
fn content_proof(path: &Path) -> Result<Option<(String, u64)>, String> {
    let m = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !regular(&m, false) {
        return Err("復旧対象の制御ファイルの実体が不正です".into());
    }
    let bytes = serde_json::to_vec(&serde_json::json!([
        Identity::read(&m)?,
        m.len(),
        m.mode(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec()
    ]))
    .map_err(|e| e.to_string())?;
    Ok(Some((hash(&bytes), m.len())))
}
impl Admission {
    pub(crate) fn verify(&self) -> Result<(), String> {
        self.root.verify()
    }
    pub(crate) fn verify_source(&self) -> Result<(), String> {
        self.verify()?;
        if let Some(expected) = &self.expected_observation {
            verify_observation(&self.root.path, expected)?;
        }
        Ok(())
    }
    pub(crate) fn scratch_root(&self) -> PathBuf {
        self.workspace.join("scratch")
    }
    pub(crate) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(crate) fn capture_content(&self) -> Result<ContentGuard, String> {
        self.verify_source()?;
        let mut files = Vec::new();
        for name in [
            "meetings.sqlite3",
            "meetings.sqlite3-wal",
            "meetings.sqlite3-journal",
        ] {
            let path = self.root.path.join(name);
            let proof = content_proof(&path)?.map(|(proof, _)| proof);
            files.push((path, proof));
        }
        Ok(ContentGuard { files })
    }
    pub(crate) fn previous_identities(&self) -> ((u64, u64, u64), (u64, u64, u64)) {
        let tuple = |i: &Identity| (i.device, i.inode, i.birth_ns);
        (
            tuple(&self.previous.directory),
            tuple(&self.previous.database),
        )
    }
}

fn observation(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    for name in [
        "meetings.sqlite3",
        "meetings.sqlite3-wal",
        "meetings.sqlite3-shm",
        "meetings.sqlite3-journal",
    ] {
        let metadata = match std::fs::symlink_metadata(root.join(name)) {
            Ok(metadata) => Some(metadata),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if let Some(m) = metadata {
            if !regular(&m, false) {
                return Err("復旧対象DBの制御ファイルが不正です".into());
            }
            files.push(serde_json::json!([
                name,
                Identity::read(&m)?,
                m.len(),
                m.mode(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec()
            ]));
        } else {
            files.push(serde_json::json!([name, null]));
        }
    }
    Ok(hash(
        &serde_json::to_vec(&files).map_err(|e| e.to_string())?,
    ))
}
fn verify_observation(root: &Path, expected: &str) -> Result<(), String> {
    if observation(root)? != expected {
        return Err("復旧の検査中にDBまたはWALが変更されています。保存先を更新しません".into());
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

impl Bootstrap {
    pub(super) fn recover_selection(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        let Some(loaded) = &state.loaded else {
            return Ok(());
        };
        let previous = &loaded.record;
        let actual = Root::inspect(&previous.current.path)?;
        let mut workspace_copied = false;
        if actual == previous.current {
            let changed = if let Some(recovery) = &previous.recovery {
                let workspace = recovery.workspace(&self.config);
                let current = Identity::read(&directory(&workspace)?)?;
                workspace_copied = current.inode != recovery.workspace_identity.inode
                    || current.birth_ns != recovery.workspace_identity.birth_ns;
                if current != recovery.workspace_identity {
                    true
                } else {
                    recovery.verify(&self.config)?;
                    Storage::ancillary_binding_changed(
                        &actual.path,
                        &workspace.join("scratch"),
                        &workspace,
                    )?
                }
            } else {
                Storage::ancillary_binding_changed(&actual.path, &actual.path, &self.config)?
            };
            if !changed {
                return Ok(());
            }
        }
        let same_object =
            |a: &Identity, b: &Identity| a.inode == b.inode && a.birth_ns == b.birth_ns;
        let kind = if same_object(&actual.directory, &previous.current.directory)
            && same_object(&actual.database, &previous.current.database)
            && !workspace_copied
        {
            "device_changed"
        } else {
            "copy_restored"
        };
        let id = Uuid::new_v4().to_string();
        let workspace = self.config.join(format!(".bootstrap-recovery-{id}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&workspace)
            .map_err(|e| e.to_string())?;
        self.directory.sync_all().map_err(|e| e.to_string())?;
        // Copies are owned and journalled independently of any stale cache.
        let source_observation = observation(&actual.path)?;
        let mut pending = Storage::inspect_rebinding(&actual.path, &workspace)?;
        verify_observation(&actual.path, &source_observation)?;
        if previous.pending_move.is_some() {
            pending.push("保存先移動が未完了です".into());
        }
        if let Some(recovery) = &previous.recovery {
            pending.extend(recovery.pending.clone());
        }
        pending.sort();
        pending.dedup();
        actual.verify()?;
        let recovery = Recovery {
            id,
            kind: kind.into(),
            previous_hash: loaded.bytes_hash.clone(),
            source_observation: source_observation.clone(),
            workspace_identity: Identity::read(&directory(&workspace)?)?,
            pending,
        };
        // Publish neither ownership nor selection until the exact old bytes
        // and the new scratch journal are durable. Crashes leave retained data.
        write_new(&workspace.join("previous-bootstrap.json"), &loaded.bytes)?;
        let scratch = workspace.join("scratch");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&scratch)
            .map_err(|e| e.to_string())?;
        Storage::initialize_rebinding_scratch(&actual.path, &scratch)?;
        let record = Record {
            revision: previous
                .revision
                .checked_add(1)
                .ok_or("起動設定の版が上限です")?,
            origin: Origin::Recovered,
            current: actual.clone(),
            pending_move: None,
            activation: None,
            recovery: Some(recovery),
        };
        write_new(
            &workspace.join("recovery.json"),
            &serde_json::to_vec(&record).map_err(|e| e.to_string())?,
        )?;
        File::open(&workspace)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        recovery_checkpoint("before_publication");
        actual.verify()?;
        verify_observation(&actual.path, &source_observation)?;
        self.write(&mut state, record)?;
        self.recovered_this_launch
            .store(true, std::sync::atomic::Ordering::Release);
        recovery_checkpoint("after_publication");
        Ok(())
    }

    pub(crate) fn recovery_view(&self) -> Result<Option<RecoveryView>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        Ok(state
            .loaded
            .as_ref()
            .and_then(|l| l.record.recovery.as_ref())
            .map(|r| r.view(&self.config)))
    }

    pub(crate) fn recovery_view_for_storage(
        &self,
        storage: &Storage,
    ) -> Result<Option<RecoveryView>, String> {
        let mut view = self.recovery_view()?;
        if let Some(view) = &mut view {
            if storage.data_write_blocked() && !storage.restart_required() && !view.review_required
            {
                view.review_required = true;
                view.reasons.push("削除履歴の再確認が必要です".into());
            }
        }
        Ok(view)
    }

    pub(super) fn read_cache_root(&self) -> Result<PathBuf, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        match state
            .loaded
            .as_ref()
            .and_then(|l| l.record.recovery.as_ref())
        {
            Some(recovery) => recovery.verify(&self.config),
            None => Ok(self.config.clone()),
        }
    }

    pub(super) fn open_recovered_storage(&self, recovery: &Recovery) -> Result<Storage, String> {
        let admission = {
            let state = self
                .state
                .lock()
                .map_err(|_| "起動設定ロックを取得できません")?;
            self.guard(&state)?;
            let record = &state.loaded.as_ref().ok_or("起動設定がありません")?.record;
            if record.recovery.as_ref() != Some(recovery) {
                return Err("復旧設定が変更されています".into());
            }
            let workspace = recovery.verify(&self.config)?;
            let bytes = std::fs::read(workspace.join("previous-bootstrap.json"))
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > LIMIT || hash(&bytes) != recovery.previous_hash {
                return Err("復旧前の起動設定の保存記録が変更されています".into());
            }
            let previous: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if previous.record.current.path != record.current.path {
                return Err("復旧前後の保存先が一致しません".into());
            }
            Admission {
                root: record.current.clone(),
                previous: previous.record.current,
                workspace,
                pending: recovery.pending.clone(),
                expected_observation: self
                    .recovered_this_launch
                    .load(std::sync::atomic::Ordering::Acquire)
                    .then(|| recovery.source_observation.clone()),
            }
        };
        admission.verify()?;
        let storage =
            Storage::open_recovered(self.root().to_owned(), self.models_root.clone(), &admission)?;
        admission.verify()?;
        Ok(storage)
    }
}

fn recovery_checkpoint(_name: &str) {
    #[cfg(test)]
    if std::env::var("MON_REBINDING_CHECKPOINT").as_deref() == Ok(_name) {
        println!("MON_REBINDING_READY={_name}");
        std::io::stdout().flush().unwrap();
        loop {
            if std::env::var_os("MON_REBINDING_RESUME_FILE").is_some_and(|p| Path::new(&p).exists())
            {
                break;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(20));
        }
    }
}

#[cfg(test)]
mod tests;
