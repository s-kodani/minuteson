//! Owned UI capabilities. Preview is read-only; confirmation adds, never replaces.
//! Keep verified source copies, not client paths/SQL or self-referential borrows.
use super::*;
use publication::{required_bytes, Publication};
use restore_fixed::{workspace::seal::SealedWorkspace, FixedData};
use restore_ids::{BatchPreparation, RestoreIds};
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackupPreview {
    pub confirmation_id: String,
    pub destination: String,
    pub replaces_existing: bool,
}

pub(crate) struct PreparedBackup {
    destination: crate::exports::Destination,
    preview: BackupPreview,
    terminal: Option<Result<archive::BackupReceipt, crate::exports::PublishError>>,
}

impl PreparedBackup {
    pub(crate) fn preview(&self) -> BackupPreview {
        self.preview.clone()
    }
    pub(crate) fn publish(
        &mut self,
        storage: &Storage,
        confirmation_id: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<archive::BackupReceipt, crate::exports::PublishError> {
        if confirmation_id != self.preview.confirmation_id {
            return Err(crate::exports::PublishError {
                published: false,
                message: "バックアップの保存先確認が変更されています".into(),
            });
        }
        self.destination
            .validate_backup_owner(&storage.root)
            .map_err(|message| crate::exports::PublishError {
                published: false,
                message,
            })?;
        if let Some(outcome) = &self.terminal {
            return outcome.clone();
        }
        let outcome = storage.create_backup_selected(&self.destination, cancelled);
        // Success and uncertain post-rename errors are terminal. A lost response
        // cannot repeat an overwrite or replace a subsequently modified file.
        if outcome.is_ok() || outcome.as_ref().err().is_some_and(|e| e.published) {
            self.terminal = Some(outcome.clone());
        }
        outcome
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RestorePreview {
    pub confirmation_id: String,
    pub batch_id: String,
    pub archive_id: String,
    pub meeting_titles: Vec<String>,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub vocabulary_added: usize,
    pub vocabulary_deduplicated: usize,
    pub vocabulary_conflicts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RestoreReceipt {
    pub batch_id: String,
    pub archive_id: String,
    pub meeting_ids: Vec<String>,
    pub already_published: bool,
    // A terminal replay must not invent the previous merge's counts.
    pub vocabulary_added: Option<usize>,
    pub vocabulary_deduplicated: Option<usize>,
    pub vocabulary_conflicts: Option<Vec<String>>,
}

pub(crate) struct PreparedRestore {
    archive: archive::VerifiedArchive,
    // Archive closes first, then every private validation copy is removed.
    scratch: scratch::Stage,
    preview: RestorePreview,
    vocabulary_fingerprint: String,
    destination_root: PathBuf,
}

fn source_file(path: &Path) -> Result<File, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        || !path
            .extension()
            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("monbackup"))
    {
        return Err("絶対パスのMONバックアップを選択してください".into());
    }
    let before = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err("バックアップはリンクではない通常ファイルを選択してください".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.nlink() != 1
            || opened.nlink() != 1
            || before.dev() != opened.dev()
            || before.ino() != opened.ino()
        {
            return Err("バックアップの実体またはリンク数が変更されています".into());
        }
    }
    if before.len() != opened.len() || before.modified().ok() != opened.modified().ok() {
        return Err("バックアップが選択中に変更されています".into());
    }
    Ok(file)
}

impl PreparedRestore {
    pub(crate) fn preview(&self) -> RestorePreview {
        self.preview.clone()
    }

    /// Reconfirm the same owned source and batch; stale confirmations stop working.
    pub(crate) fn refresh(
        &mut self,
        storage: &Storage,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<RestorePreview, String> {
        if self.destination_root != storage.root {
            return Err("復元の確認対象の保存領域が変更されています".into());
        }
        let maintenance = storage.backup_maintenance()?;
        self.archive.verify_validation_database(cancelled)?;
        let ids = {
            let db = storage
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            match RestoreIds::prepare_for_batch(
                &self.archive,
                self.scratch.path(),
                &self.preview.batch_id,
                &db,
                cancelled,
            )? {
                BatchPreparation::Pending(ids) => ids,
                BatchPreparation::AlreadyPublished { .. } => {
                    return Err("この復元操作は確定済みです。結果を再確認してください".into())
                }
            }
        };
        progress("reconstructing");
        let fixed = FixedData::new(&ids, self.scratch.path(), cancelled)?;
        let sealed = SealedWorkspace::prepare(&fixed, self.scratch.path(), cancelled)?;
        progress("verifying");
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let merge = sealed.global_merge(&db, cancelled)?;
        let required = required_bytes(&db, &sealed, &ids)?
            .checked_add(reservation_bytes(&ids, cancelled)?)
            .ok_or("復元確認容量が範囲外です")?;
        check_capacity(&storage.root, required)?;
        let receipt = merge.receipt();
        let next = RestorePreview {
            confirmation_id: Uuid::new_v4().to_string(),
            batch_id: self.preview.batch_id.clone(),
            archive_id: self.archive.manifest.archive_id.clone(),
            meeting_titles: self
                .archive
                .manifest
                .meetings
                .iter()
                .map(|m| m.title.clone())
                .collect(),
            required_bytes: required
                .checked_add(CAPACITY_RESERVE)
                .ok_or("復元確認容量が範囲外です")?,
            available_bytes: crate::audio_import::available_bytes(&storage.root)?,
            vocabulary_added: receipt.added,
            vocabulary_deduplicated: receipt.deduplicated,
            vocabulary_conflicts: receipt.conflicts.iter().map(|c| c.term.clone()).collect(),
        };
        check_cancelled(cancelled)?;
        self.archive.verify_validation_database(cancelled)?;
        self.vocabulary_fingerprint = merge.confirmation_fingerprint().into();
        self.preview = next;
        drop(db);
        drop(maintenance);
        Ok(self.preview())
    }

    /// Retry preserves batch identity. Verified copies are rechecked; the original
    /// external file may have moved without redirecting this operation's input.
    pub(crate) fn publish(
        &self,
        storage: &Storage,
        confirmation_id: &str,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<RestoreReceipt, String> {
        if self.destination_root != storage.root {
            return Err("復元の確認対象の保存領域が変更されています".into());
        }
        if confirmation_id != self.preview.confirmation_id {
            return Err("復元の確認内容が変更されています。再確認してください".into());
        }
        check_cancelled(cancelled)?;
        self.archive.verify_validation_database(cancelled)?;
        let ids = {
            let db = storage
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            match RestoreIds::prepare_for_batch(
                &self.archive,
                self.scratch.path(),
                &self.preview.batch_id,
                &db,
                cancelled,
            )? {
                BatchPreparation::AlreadyPublished { .. } => return self.terminal_receipt(&db),
                BatchPreparation::Pending(ids) => ids,
            }
        };
        progress("reconstructing");
        let fixed = FixedData::new(&ids, self.scratch.path(), cancelled)?;
        let sealed = SealedWorkspace::prepare(&fixed, self.scratch.path(), cancelled)?;
        let meetings = self
            .archive
            .manifest
            .meetings
            .iter()
            .map(|m| ids.get("meetings", "", &m.id))
            .collect::<Result<Vec<_>, _>>()?;
        self.archive.verify_validation_database(cancelled)?;
        progress("publishing");
        let outcome = storage.publish_restore_confirmed(
            &sealed,
            &ids,
            (&self.vocabulary_fingerprint, self.preview.required_bytes),
            cancelled,
        )?;
        let (already_published, counts) = match outcome {
            Publication::Published { vocabulary, .. } => (false, Some(vocabulary)),
            Publication::AlreadyPublished => (true, None),
        };
        // No cancellation/error after commit can turn successful publication into
        // a cleanup path. Receipt construction uses only already owned values.
        Ok(RestoreReceipt {
            batch_id: self.preview.batch_id.clone(),
            archive_id: self.preview.archive_id.clone(),
            meeting_ids: meetings,
            already_published,
            vocabulary_added: counts.as_ref().map(|r| r.added),
            vocabulary_deduplicated: counts.as_ref().map(|r| r.deduplicated),
            vocabulary_conflicts: counts.map(|r| r.conflicts.into_iter().map(|c| c.term).collect()),
        })
    }

    fn terminal_receipt(&self, db: &Connection) -> Result<RestoreReceipt, String> {
        let mut query=db.prepare("SELECT target_key FROM restore_identity_maps WHERE batch_id=?1 AND domain='meetings' AND source_scope='' ORDER BY source_key").map_err(|e|e.to_string())?;
        let ids = query
            .query_map(params![self.preview.batch_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(RestoreReceipt {
            batch_id: self.preview.batch_id.clone(),
            archive_id: self.preview.archive_id.clone(),
            meeting_ids: ids,
            already_published: true,
            vocabulary_added: None,
            vocabulary_deduplicated: None,
            vocabulary_conflicts: None,
        })
    }
}

// The preview has not reserved anything in the live DB. Allow for mapping rows,
// indexes, commit/WAL and both live-DB copies before comparing actual capacity.
fn reservation_bytes(ids: &RestoreIds<'_>, cancelled: &impl Fn() -> bool) -> Result<u64, String> {
    let mut bytes = 64 * 1024_u64;
    ids.visit_mappings(cancelled, |map| {
        let encoded = serde_json::to_vec(&map).map_err(|e| e.to_string())?;
        bytes = bytes
            .checked_add(
                (encoded.len() as u64)
                    .checked_mul(8)
                    .ok_or("復元ID容量が範囲外です")?,
            )
            .and_then(|n| n.checked_add(16 * 1024))
            .ok_or("復元ID容量が範囲外です")?;
        Ok(())
    })?;
    Ok(bytes)
}

impl Storage {
    pub(crate) fn prepare_backup_destination(
        &self,
        path: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<PreparedBackup, String> {
        check_cancelled(cancelled)?;
        let _maintenance = self.backup_maintenance()?;
        let destination =
            crate::exports::Destination::selected_backup(&self.root, path, cancelled)?;
        let preview = BackupPreview {
            confirmation_id: Uuid::new_v4().to_string(),
            destination: destination.path.to_string_lossy().into_owned(),
            replaces_existing: destination.replaces_existing(),
        };
        Ok(PreparedBackup {
            destination,
            preview,
            terminal: None,
        })
    }
    pub(crate) fn prepare_additive_restore(
        &self,
        path: &Path,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Result<PreparedRestore, String> {
        check_cancelled(cancelled)?;
        // Read/extract only while the normal storage write gate is held.
        let maintenance = self.backup_maintenance()?;
        progress("reading");
        self.import_scratch()?;
        let scratch = self.backup_stage()?;
        let archive = archive::read(&mut source_file(path)?, scratch.path(), cancelled)?;
        drop(maintenance);
        let mut selection = PreparedRestore {
            archive,
            scratch,
            preview: RestorePreview {
                confirmation_id: String::new(),
                batch_id: Uuid::new_v4().to_string(),
                archive_id: String::new(),
                meeting_titles: vec![],
                required_bytes: 0,
                available_bytes: 0,
                vocabulary_added: 0,
                vocabulary_deduplicated: 0,
                vocabulary_conflicts: vec![],
            },
            vocabulary_fingerprint: String::new(),
            destination_root: self.root.clone(),
        };
        selection.refresh(self, cancelled, progress)?;
        Ok(selection)
    }
}

#[cfg(test)]
mod tests;
