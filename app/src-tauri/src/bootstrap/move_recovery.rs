//! Pending move review and non-destructive retention. Retention is a durable
//! choice, never a capability to adopt, activate, overwrite, or delete a copy.
use super::*;
use crate::data_controller::{MovePreview, MoveReceipt};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeptMove {
    pub operation_id: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub decision_pending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Decision {
    pending: PendingMove,
    revision: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct KeptDocument {
    version: u32,
    decision: Decision,
    sha256: String,
}

pub(crate) struct Review {
    pending: PendingMove,
    revision: u64,
    preview: MovePreview,
    observation: String,
}
fn check(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("未完了移動の確認を取り消しました。データを保持します".into())
    } else {
        Ok(())
    }
}
fn marker(operation: &str) -> String {
    format!(".relocation-kept-{operation}.json")
}
fn missing(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

// Metadata observation does not confer ownership. Symlinks are never followed;
// unreadable/unknown objects may be kept, but cannot authorize resumed writes.
fn observe(
    bootstrap: &Bootstrap,
    pending: &PendingMove,
    cancelled: &impl Fn() -> bool,
) -> Result<String, String> {
    let mut objects = Vec::new();
    let mut paths = vec![
        pending.target.clone(),
        bootstrap
            .config
            .join(format!(".relocation-{}.sqlite3", pending.operation_id)),
        bootstrap
            .config
            .join(format!(".relocation-{}.lock", pending.operation_id)),
    ];
    while let Some(path) = paths.pop() {
        check(cancelled)?;
        if objects.len() > 200_004 {
            return Err("保持対象の確認が安全件数を超えました。ファイルは変更しません".into());
        }
        match std::fs::symlink_metadata(&path) {
            Ok(m) => {
                objects.push(serde_json::json!([
                    path,
                    Identity::read(&m)?,
                    m.mode(),
                    m.nlink(),
                    m.len(),
                    m.mtime(),
                    m.mtime_nsec(),
                    m.ctime(),
                    m.ctime_nsec()
                ]));
                if m.is_dir() && path.starts_with(&pending.target) {
                    match std::fs::read_dir(&path) {
                        Ok(children) => {
                            let mut children_paths = Vec::new();
                            for child in children {
                                if objects.len() + paths.len() + children_paths.len() >= 200_004 {
                                    return Err("保持対象の件数が範囲外です".into());
                                }
                                children_paths.push(child.map_err(|e| e.to_string())?.path());
                            }
                            let mut children = children_paths;
                            children.sort();
                            paths.extend(children.into_iter().rev());
                        }
                        Err(e) => objects.push(serde_json::json!([
                            path,
                            "unreadable",
                            format!("{:?}", e.kind())
                        ])),
                    }
                }
            }
            Err(e) => objects.push(serde_json::json!([
                path,
                "unavailable",
                format!("{:?}", e.kind())
            ])),
        }
    }
    Ok(hash(
        &serde_json::to_vec(&objects).map_err(|e| e.to_string())?,
    ))
}

impl Bootstrap {
    fn current_pending(&self, storage: &Storage) -> Result<(PendingMove, u64), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        self.guard(&state)?;
        let r = &state.loaded.as_ref().ok_or("起動設定がありません")?.record;
        r.current.verify()?;
        if storage.restart_required()
            || self
                .restart_required
                .load(std::sync::atomic::Ordering::Acquire)
            || r.current.path != self.selected
            || storage.root() != r.current.path
        {
            return Err("再起動してから未完了移動を確認してください".into());
        }
        Ok((
            r.pending_move.clone().ok_or("未完了移動はありません")?,
            r.revision,
        ))
    }

    fn read_kept(&self, operation: &str) -> Result<Option<Decision>, String> {
        if !uuid(operation) {
            return Err("保持操作IDが不正です".into());
        }
        self.guard_directory()?;
        let name = std::ffi::CString::new(marker(operation)).map_err(|e| e.to_string())?;
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            return if e.kind() == std::io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(e.to_string())
            };
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !regular(&before, true) || before.len() > LIMIT {
            return Err("保持記録の形式・権限・容量が不正です".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("保持記録の容量が範囲外です".into());
        }
        let doc: KeptDocument = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let p = &doc.decision.pending;
        absolute(&p.source.path)?;
        absolute(&p.target)?;
        if doc.version != 1
            || doc.decision.revision > i64::MAX as u64
            || !uuid(&p.operation_id)
            || p.operation_id != operation
            || !uuid(&p.confirmation_id)
            || !sha(&p.manifest_hash)
            || !sha(&doc.sha256)
            || p.target.starts_with(&p.source.path)
            || p.source.path.starts_with(&p.target)
            || hash(&serde_json::to_vec(&doc.decision).map_err(|e| e.to_string())?) != doc.sha256
        {
            return Err("保持記録の確認対象が不正です".into());
        }
        let after = file.metadata().map_err(|e| e.to_string())?;
        if !Identity::read(&before)?.matches(
            &std::fs::symlink_metadata(self.config.join(marker(operation)))
                .map_err(|e| e.to_string())?,
        ) || !Identity::read(&before)?.matches(&after)
            || before.len() != after.len()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err("保持記録が読込中に変更されています".into());
        }
        self.guard_directory()?;
        Ok(Some(doc.decision))
    }

    pub(super) fn kept_moves(&self, record: &Record) -> Result<Vec<KeptMove>, String> {
        self.guard_directory()?;
        let mut ids = Vec::new();
        for child in std::fs::read_dir(&self.config).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            let name = child
                .file_name()
                .into_string()
                .map_err(|_| "設定領域の名前が不正です")?;
            if let Some(id) = name
                .strip_prefix(".relocation-kept-")
                .and_then(|n| n.strip_suffix(".json"))
            {
                if ids.len() >= 1024 {
                    return Err("保持記録の一覧が安全件数を超えました".into());
                }
                ids.push(id.to_owned());
            }
        }
        ids.sort();
        ids.into_iter()
            .map(|id| {
                let d = self
                    .read_kept(&id)?
                    .ok_or("保持記録が読込中に消失しています")?;
                Ok(KeptMove {
                    operation_id: id.clone(),
                    source: d.pending.source.path,
                    destination: d.pending.target,
                    decision_pending: record
                        .pending_move
                        .as_ref()
                        .is_some_and(|p| p.operation_id == id),
                })
            })
            .collect()
    }

    pub(crate) fn prepare_pending_move(
        &self,
        storage: &Storage,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<Review, String> {
        check(cancelled)?;
        let (pending, revision) = self.current_pending(storage)?;
        let decision = self.read_kept(&pending.operation_id)?;
        if decision
            .as_ref()
            .is_some_and(|d| d.pending != pending || d.revision != revision)
        {
            return Err("保持記録と未完了移動が一致しません。自動修復しません".into());
        }
        progress("verifying");
        let mut preview = MovePreview {
            confirmation_id: Uuid::new_v4().to_string(),
            operation_id: pending.operation_id.clone(),
            source: pending.source.path.clone(),
            destination: pending.target.clone(),
            meeting_count: storage.list_meetings().map_err(|e| e.to_string())?.len(),
            file_count: None,
            required_bytes: 0,
            available_bytes: 0,
            pending: true,
            resume_allowed: false,
            resume_reason: None,
            copied_paths: None,
            expected_paths: None,
        };
        let can_resume = (|| -> Result<(), String> {
            if decision.is_some() {
                return Err(
                    "保持する判断を保存済みです。保持を確認して未完了記録の解決を完了してください"
                        .into(),
                );
            }
            let snapshot = storage.prepare_relocation_snapshot(cancelled)?;
            if snapshot.manifest_hash() != pending.manifest_hash {
                return Err("コピー開始後に旧保存先の会議・編集・音声が変わっています。古いコピーへ切り替えません".into());
            }
            preview.file_count = Some(
                snapshot
                    .manifest()
                    .entries
                    .iter()
                    .filter(|e| !e.directory)
                    .count()
                    + 1,
            );
            let bytes = snapshot
                .manifest()
                .entries
                .iter()
                .try_fold(snapshot.manifest().database_size, |n, e| {
                    n.checked_add(e.size).ok_or("移動容量が範囲外です")
                })?;
            preview.required_bytes = bytes
                .checked_mul(2)
                .and_then(|n| n.checked_add(64 * 1024 * 1024))
                .filter(|n| *n <= 9_007_199_254_740_991)
                .ok_or("移動容量が範囲外です")?;
            preview.available_bytes =
                crate::audio_import::available_bytes(&pending.target)?.min(9_007_199_254_740_991);
            if missing(
                &self
                    .config
                    .join(format!(".relocation-{}.sqlite3", pending.operation_id)),
            ) {
                let m = directory(&pending.target)?;
                local(&pending.target)?;
                if !pending.target_directory.matches(&m)
                    || m.mode() & 0o077 != 0
                    || !missing(
                        &self
                            .config
                            .join(format!(".relocation-{}.lock", pending.operation_id)),
                    )
                    || std::fs::read_dir(&pending.target)
                        .map_err(|e| e.to_string())?
                        .next()
                        .is_some()
                {
                    return Err("所有台帳がなく、移動先が新規の空領域と確認できません。既存物を採用しません".into());
                }
                preview.copied_paths = Some(0);
                preview.expected_paths = Some(snapshot.manifest().entries.len() + 1);
            } else {
                let p = self.open_placement(pending.clone(), None)?;
                let result = p.inspect_partial(cancelled)?;
                preview.copied_paths = Some(result.verified_paths);
                preview.expected_paths = Some(result.expected_paths);
            }
            if preview.available_bytes < preview.required_bytes {
                return Err(
                    "移動先の空き容量が不足しています。容量を確保して再確認してください".into(),
                );
            }
            Ok(())
        })();
        check(cancelled)?;
        match can_resume {
            Ok(()) => preview.resume_allowed = true,
            Err(e) => preview.resume_reason = Some(e),
        }
        let observation = observe(self, &pending, cancelled)?;
        if self.current_pending(storage)? != (pending.clone(), revision) {
            return Err("未完了移動が確認中に変更されています".into());
        }
        Ok(Review {
            pending,
            revision,
            preview,
            observation,
        })
    }

    fn write_kept(&self, decision: &Decision) -> Result<(), String> {
        let id = &decision.pending.operation_id;
        if let Some(old) = self.read_kept(id)? {
            if old != *decision {
                return Err("別の保持判断があります。上書きしません".into());
            }
        } else {
            let bytes = serde_json::to_vec(&KeptDocument {
                version: 1,
                decision: decision.clone(),
                sha256: hash(&serde_json::to_vec(decision).map_err(|e| e.to_string())?),
            })
            .map_err(|e| e.to_string())?;
            if bytes.len() as u64 > LIMIT {
                return Err("保持記録が容量上限を超えています".into());
            }
            let temp =
                std::ffi::CString::new(format!(".move-keep-{}.tmp", Uuid::new_v4())).unwrap();
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    temp.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
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
            self.guard_directory()?;
            let target = std::ffi::CString::new(marker(id)).unwrap();
            if unsafe {
                libc::renameatx_np(
                    self.directory.as_raw_fd(),
                    temp.as_ptr(),
                    self.directory.as_raw_fd(),
                    target.as_ptr(),
                    libc::RENAME_EXCL,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        self.directory.sync_all().map_err(|e| e.to_string())?;
        if self.read_kept(id)?.as_ref() != Some(decision) {
            return Err("保持記録の確定結果が一致しません".into());
        }
        keep_checkpoint();
        Ok(())
    }
}

impl Review {
    pub(crate) fn preview(&self) -> MovePreview {
        self.preview.clone()
    }
    fn guard(
        &self,
        b: &Bootstrap,
        s: &Storage,
        confirmation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        check(cancelled)?;
        if confirmation != self.preview.confirmation_id
            || b.current_pending(s)? != (self.pending.clone(), self.revision)
            || observe(b, &self.pending, cancelled)? != self.observation
        {
            return Err("未完了移動の確認後に対象が変更されています。再確認してください".into());
        }
        Ok(())
    }
    pub(crate) fn resume(
        &self,
        b: &Bootstrap,
        s: &Storage,
        confirmation: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<MoveReceipt, String> {
        self.guard(b, s, confirmation, cancelled)?;
        if !self.preview.resume_allowed || b.read_kept(&self.pending.operation_id)?.is_some() {
            return Err("この途中コピーは続行できません。保持または再確認してください".into());
        }
        let snapshot = s.prepare_relocation_snapshot(cancelled)?;
        if snapshot.manifest_hash() != self.pending.manifest_hash {
            return Err("旧保存先が確認後に変わっています。再確認してください".into());
        }
        self.guard(b, s, confirmation, cancelled)?;
        progress("copying");
        let mut p = b.pending_placement(&self.pending.operation_id, &snapshot)?;
        p.inspect_partial(cancelled)?;
        p.copy(&snapshot, cancelled)?;
        progress("handoff");
        let handoff = snapshot.prepare_control_handoff(cancelled)?;
        p.install_history(&handoff, cancelled)?;
        progress("switching");
        let selected = p.activate_for_restart(
            &handoff,
            &self.pending.confirmation_id,
            self.revision,
            cancelled,
        )?;
        Ok(MoveReceipt {
            operation_id: self.pending.operation_id.clone(),
            source: self.pending.source.path.clone(),
            destination: self.pending.target.clone(),
            restart_required: selected.restart_required,
            retained: false,
        })
    }
    pub(crate) fn keep(
        &self,
        b: &Bootstrap,
        s: &Storage,
        confirmation: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<MoveReceipt, String> {
        let decision = Decision {
            pending: self.pending.clone(),
            revision: self.revision,
        };
        if confirmation != self.preview.confirmation_id {
            return Err("保持の確認内容が一致しません".into());
        }
        // A lost acknowledgement may be replayed only from the exact durable
        // decision and its immediately following config revision, not a receipt.
        if b.read_kept(&self.pending.operation_id)?.as_ref() == Some(&decision) {
            let state = b
                .state
                .lock()
                .map_err(|_| "起動設定ロックを取得できません")?;
            b.guard(&state)?;
            let r = &state.loaded.as_ref().ok_or("起動設定がありません")?.record;
            if r.pending_move.is_none()
                && Some(r.revision) == self.revision.checked_add(1)
                && r.current == self.pending.source
                && s.root() == r.current.path
                && !s.restart_required()
            {
                r.current.verify()?;
                return Ok(self.kept_receipt());
            }
        }
        self.guard(b, s, confirmation, cancelled)?;
        b.write_kept(&decision)?;
        // After the durable choice, finish its config transaction even on late
        // cancellation. A crash is explicitly resumed from the saved choice.
        self.guard(b, s, confirmation, &|| false)?;
        let mut state = b
            .state
            .lock()
            .map_err(|_| "起動設定ロックを取得できません")?;
        b.guard(&state)?;
        let mut record = state
            .loaded
            .as_ref()
            .ok_or("起動設定がありません")?
            .record
            .clone();
        if record.pending_move.as_ref() != Some(&self.pending) || record.revision != self.revision {
            return Err("保持対象が変更されています".into());
        }
        record.pending_move = None;
        record.revision = record
            .revision
            .checked_add(1)
            .ok_or("起動設定の版が上限です")?;
        b.write(&mut state, record)?;
        Ok(self.kept_receipt())
    }
    fn kept_receipt(&self) -> MoveReceipt {
        MoveReceipt {
            operation_id: self.pending.operation_id.clone(),
            source: self.pending.source.path.clone(),
            destination: self.pending.target.clone(),
            restart_required: false,
            retained: true,
        }
    }
}
fn keep_checkpoint() {
    #[cfg(test)]
    if std::env::var("MON_MOVE_KEEP_CHECKPOINT").is_ok_and(|v| v == "saved") {
        if let Ok(path) = std::env::var("MON_MOVE_KEEP_READY") {
            std::fs::write(path, b"ready").unwrap();
        }
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests;
