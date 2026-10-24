//! Sealed local recovery history, tied to a held native source snapshot. This is
//! an input to the destination handoff, not permission to activate or clean up.
use super::*;
use std::sync::Arc;

#[derive(Serialize)]
struct Envelope {
    version: u32,
    native_manifest_hash: String,
    history: scratch::transfer::Bundle,
    #[serde(skip_serializing_if = "Option::is_none")]
    deletion_history: Option<crate::storage::deletion::relocation::Source>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flac_history: Option<crate::storage::recordings::flac::relocation::Source>,
    excluded_area: String,
}

#[allow(dead_code)] // Destination handoff and cutover coordinator follow.
pub(crate) struct ControlHandoff<'a, 'storage> {
    snapshot: &'a NativeSnapshot<'storage>,
    registry: Arc<scratch::Registry>,
    path: PathBuf,
    proof: Proof,
    bytes: Vec<u8>,
    hash: String,
    history: scratch::transfer::Bundle,
    deletion_history: Option<crate::storage::deletion::relocation::Source>,
    flac_history: Option<crate::storage::recordings::flac::relocation::Source>,
}

#[allow(dead_code)]
impl<'storage> NativeSnapshot<'storage> {
    fn deletion_history(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<crate::storage::deletion::relocation::Source>, String> {
        crate::storage::deletion::relocation::Source::capture(
            self.maintenance.storage,
            self,
            cancelled,
        )
    }
    fn flac_history(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<crate::storage::recordings::flac::relocation::Source>, String> {
        crate::storage::recordings::flac::relocation::Source::capture(
            self.maintenance.storage,
            self,
            cancelled,
        )
    }
    pub(crate) fn prepare_control_handoff<'a>(
        &'a self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<ControlHandoff<'a, 'storage>, String> {
        self.verify(cancelled)?;
        self.verify_source(cancelled)?;
        let registry = self
            .maintenance
            .storage
            .scratch
            .as_ref()
            .ok_or("端末内台帳がありません")?
            .clone();
        let history = registry.handoff(
            self.stage.path(),
            &read_only(&self.image.join("meetings.sqlite3"))?,
            cancelled,
        )?;
        let deletion_history = self.deletion_history(cancelled)?;
        let flac_history = self.flac_history(cancelled)?;
        let bytes = serde_json::to_vec(&Envelope {
            version: if flac_history.is_some() {
                5
            } else if deletion_history.is_some() {
                4
            } else {
                3
            },
            native_manifest_hash: self.manifest_hash.clone(),
            history: history.clone(),
            deletion_history: deletion_history.clone(),
            flac_history: flac_history.clone(),
            excluded_area: self
                .stage
                .path()
                .file_name()
                .ok_or("構築領域名がありません")?
                .to_string_lossy()
                .into_owned(),
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("移行用制御記録が安全容量を超えています".into());
        }
        let path = self.stage.path().join("relocation-control.json");
        let mut file = private_file(&path)?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
        let proof = Proof::read(&crate::native_fs::metadata(&file).map_err(|e| e.to_string())?)?;
        crate::native_fs::sync_path(&self.stage.path())?;
        let result = ControlHandoff {
            snapshot: self,
            registry,
            path,
            proof,
            hash: digest(&bytes),
            bytes,
            history,
            deletion_history,
            flac_history,
        };
        result.verify(cancelled)?;
        Ok(result)
    }
}
#[allow(dead_code)]
impl ControlHandoff<'_, '_> {
    pub(crate) fn hash(&self) -> &str {
        &self.hash
    }
    pub(crate) fn snapshot(&self) -> &NativeSnapshot<'_> {
        self.snapshot
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(in crate::storage) fn native_deletion_history(
        &self,
    ) -> Option<&crate::storage::deletion::relocation::Source> {
        self.deletion_history.as_ref()
    }
    pub(crate) fn check_compatible(&self, saved: &[u8]) -> Result<(), String> {
        if saved == self.bytes {
            return Ok(());
        }
        if saved.len() > 64 * 1024 * 1024 {
            return Err("元の移行確認が安全容量を超えています".into());
        }
        let old: serde_json::Value = serde_json::from_slice(saved).map_err(|e| e.to_string())?;
        let version = if self.flac_history.is_some() {
            5
        } else if self.deletion_history.is_some() {
            4
        } else {
            3
        };
        let expected_deletion =
            serde_json::to_value(&self.deletion_history).map_err(|e| e.to_string())?;
        let expected_flac = serde_json::to_value(&self.flac_history).map_err(|e| e.to_string())?;
        if old["version"] != version
            || old["native_manifest_hash"] != self.snapshot.manifest_hash()
            || match &self.deletion_history {
                Some(_) => old.get("deletion_history") != Some(&expected_deletion),
                None => old.get("deletion_history").is_some(),
            }
            || match &self.flac_history {
                Some(_) => old.get("flac_history") != Some(&expected_flac),
                None => old.get("flac_history").is_some(),
            }
        {
            return Err("元の移行確認と会議コピーが一致しません".into());
        }
        let current = serde_json::to_value(&self.history).map_err(|e| e.to_string())?;
        let original = &old["history"];
        for key in [
            "version",
            "source_root",
            "source_identity",
            "decisions",
            "restores",
            "remaining_root_objects",
            "origins",
            "bindings",
        ] {
            if original.get(key) != current.get(key) {
                return Err("配置完了後に元の保持判断・由来・残存対象が変更されています".into());
            }
        }
        let before = original["areas"]
            .as_array()
            .ok_or("元の構築履歴が不正です")?;
        let after = current["areas"]
            .as_array()
            .ok_or("現在の構築履歴が不正です")?;
        for area in before {
            let id = area["id"].as_str().ok_or("元の構築履歴IDが不正です")?;
            if !after.iter().any(|a| a["id"] == id && a == area) {
                return Err("配置完了後に元の構築履歴が変更されています".into());
            }
        }
        for area in after {
            if !before.iter().any(|a| a["id"] == area["id"])
                && !(area["state"] == "recovered"
                    && area["record_valid"] == true
                    && area["entries"]
                        .as_array()
                        .is_some_and(|entries| entries.iter().all(|e| e["removed"] == true)))
            {
                return Err(
                    "新たな未解決の構築領域があります。移行確認をやり直してください".into(),
                );
            }
        }
        Ok(())
    }
    pub(crate) fn build_registry_image(
        &self,
        operation: &str,
        target: &Path,
        placement: &crate::bootstrap::placement::Placement<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<RegistryImage, String> {
        self.verify(cancelled)?;
        let bindings = publication::journal::relocation::build(
            self.snapshot,
            placement,
            operation,
            cancelled,
        )?;
        let deletion_binding = self
            .deletion_history
            .as_ref()
            .map(|source| source.bind_destination(self.snapshot, placement, operation, cancelled))
            .transpose()?;
        let flac_binding = self
            .flac_history
            .as_ref()
            .map(|source| source.bind_destination(self.snapshot, placement, operation, cancelled))
            .transpose()?;
        let path = self
            .snapshot
            .stage
            .path()
            .join(format!("registry-{}.sqlite3", Uuid::new_v4()));
        let file = private_file(&path)?;
        let db = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| e.to_string())?;
        scratch::history::write_image(
            &db,
            target,
            operation,
            self.snapshot.manifest_hash(),
            &self.hash,
            &self.history,
            self.deletion_history.as_ref(),
            deletion_binding.as_ref(),
            self.flac_history.as_ref(),
            flac_binding.as_ref(),
            &bindings,
        )?;
        drop(db);
        crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
        crate::native_fs::sync_path(&self.snapshot.stage.path())?;
        let proof = Proof::read(&crate::native_fs::metadata(&file).map_err(|e| e.to_string())?)?;
        let (size, hash) = digest_file(&path, cancelled)?;
        self.verify(cancelled)?;
        Ok(RegistryImage {
            path,
            proof,
            size,
            hash,
        })
    }
    pub(crate) fn verify(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.snapshot.verify(cancelled)?;
        self.snapshot.verify_source(cancelled)?;
        if Proof::read(&crate::native_fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?)?
            != self.proof
            || digest_file(&self.path, cancelled)? != (self.bytes.len() as u64, self.hash.clone())
        {
            return Err("移行用制御記録が変更されています".into());
        }
        let history = self.registry.handoff(
            self.snapshot.stage.path(),
            &read_only(&self.snapshot.image.join("meetings.sqlite3"))?,
            cancelled,
        )?;
        let deletion_history = self.snapshot.deletion_history(cancelled)?;
        let flac_history = self.snapshot.flac_history(cancelled)?;
        let actual = serde_json::to_vec(&Envelope {
            version: if flac_history.is_some() {
                5
            } else if deletion_history.is_some() {
                4
            } else {
                3
            },
            native_manifest_hash: self.snapshot.manifest_hash.clone(),
            history,
            deletion_history,
            flac_history,
            excluded_area: self
                .snapshot
                .stage
                .path()
                .file_name()
                .ok_or("構築領域名がありません")?
                .to_string_lossy()
                .into_owned(),
        })
        .map_err(|e| e.to_string())?;
        if actual != self.bytes {
            return Err("元の所有台帳・保持記録が変更されています。移行確認は失効しました".into());
        }
        self.snapshot.verify_source(cancelled)?;
        if Proof::read(&crate::native_fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?)?
            != self.proof
        {
            return Err("元の台帳を再照合する間に移行用制御記録が変更されています".into());
        }
        Ok(())
    }
}

pub(crate) fn verify_registry_image(
    root: &Path,
    path: &Path,
    operation: &str,
    native_hash: &str,
    control_hash: &str,
) -> Result<(), String> {
    scratch::history::verify_image(root, path, operation, native_hash, control_hash)
}
pub(crate) fn registry_digest(path: &Path) -> Result<(u64, String), String> {
    digest_file(path, &|| false)
}

pub(crate) struct RegistryImage {
    path: PathBuf,
    proof: Proof,
    pub(crate) size: u64,
    pub(crate) hash: String,
}
impl RegistryImage {
    pub(crate) fn write_owned(
        &self,
        output: &mut File,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&self.path)
            .map_err(|e| e.to_string())?;
        if Proof::read(&crate::native_fs::metadata(&input).map_err(|e| e.to_string())?)?
            != self.proof
        {
            return Err("移行用台帳の一時コピーが変更されています".into());
        }
        check(cancelled)?;
        output.set_len(0).map_err(|e| e.to_string())?;
        std::io::Seek::seek(output, std::io::SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let (_, hash) = stream(&mut input, Some(output), self.size, cancelled)?;
        if hash != self.hash
            || Proof::read(&crate::native_fs::metadata(&input).map_err(|e| e.to_string())?)?
                != self.proof
            || Proof::read(
                &crate::native_fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?,
            )? != self.proof
        {
            return Err("移行用台帳のコピー中に内容・実体が変更されています".into());
        }
        crate::native_fs::sync_handle(&output).map_err(|e| e.to_string())
    }
}

#[cfg(all(test, unix))]
mod tests;
