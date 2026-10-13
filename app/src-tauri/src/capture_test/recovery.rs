//! Background collection is ownership reconciliation, never discovery by
//! filename. Live writers and unverified ownership always retain their files.
use serde::Serialize;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub(crate) schema_version: u8,
    pub(crate) revision: u64,
    pub(crate) phase: String,
    pub(crate) run_id: Option<String>,
    pub(crate) error_message: Option<String>,
}
pub(super) struct Ownership {
    snapshot: Snapshot,
    #[cfg(target_os = "macos")]
    root: Option<PathBuf>,
    #[cfg(target_os = "macos")]
    registry: Option<Arc<super::journal::Registry>>,
}
impl Default for Ownership {
    fn default() -> Self {
        Self {
            snapshot: Snapshot {
                schema_version: 1,
                revision: 1,
                phase: if cfg!(target_os = "macos") {
                    "checking"
                } else {
                    "clear"
                }
                .into(),
                run_id: None,
                error_message: None,
            },
            #[cfg(target_os = "macos")]
            root: None,
            #[cfg(target_os = "macos")]
            registry: None,
        }
    }
}
impl Ownership {
    pub(super) fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }
    pub(super) fn blocked(&self) -> bool {
        self.snapshot.phase != "clear"
    }
    pub(super) fn checking(&self) -> bool {
        self.snapshot.phase == "checking"
    }
    pub(super) fn retry_revision(&self) -> Option<u64> {
        (self.snapshot.phase == "attention").then_some(self.snapshot.revision)
    }
    pub(super) fn require_clear(&self) -> Result<(), String> {
        if self.blocked() {
            return Err(
                "一時テスト音声をバックグラウンドで整理しています。安全に解放できるまで録音は開始できません。少し待って再試行してください".into(),
            );
        }
        Ok(())
    }
    pub(super) fn configure(&mut self, root: &Path) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            if self.root.is_some() {
                return Err("テスト音声の回復先は設定済みです".into());
            }
            self.root = Some(root.to_path_buf());
        }
        #[cfg(not(target_os = "macos"))]
        let _ = root;
        Ok(())
    }
    pub(super) fn require_retry(&self, revision: u64) -> Result<(), String> {
        if revision != self.snapshot.revision || self.snapshot.phase != "attention" {
            return Err("回収確認の状態が変更されています。別の回収は開始していません".into());
        }
        Ok(())
    }
    pub(super) fn attention(&mut self, error: String) {
        self.snapshot.revision += 1;
        self.snapshot.phase = "attention".into();
        // Keep the response bounded even when an OS error contains a long path.
        self.snapshot.error_message = Some(error.chars().take(2048).collect());
    }
    pub(super) fn reconcile(&mut self) -> Snapshot {
        self.snapshot.phase = "checking".into();
        self.snapshot.error_message = None;
        self.snapshot.run_id = None;
        #[cfg(target_os = "macos")]
        let result = (|| {
            if self.registry.is_none() {
                let root = self.root.as_ref().ok_or("テスト音声の回復先がありません")?;
                std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
                self.registry = Some(super::journal::Registry::open(root)?);
            }
            let registry = self.registry.as_ref().unwrap();
            for record in registry.pending()? {
                self.snapshot.run_id = Some(record.request.run_id.clone());
                registry.collect(record)?;
            }
            Ok::<_, String>(())
        })();
        #[cfg(not(target_os = "macos"))]
        let result = Ok::<_, String>(());
        match result {
            Ok(()) => {
                self.snapshot.revision += 1;
                self.snapshot.phase = "clear".into();
                self.snapshot.run_id = None;
            }
            Err(error) => self.attention(format!(
                "テスト音声を保持しています。取得プロセスの終了または所有情報を再確認してください: {error}"
            )),
        }
        self.snapshot()
    }
    #[cfg(target_os = "macos")]
    pub(super) fn allocate(
        &mut self,
        request: super::Request,
    ) -> Result<super::files::TestDirectory, String> {
        self.require_clear()?;
        let result = self
            .registry
            .as_ref()
            .ok_or("テスト音声の所有台帳がありません".to_string())
            .and_then(|registry| super::files::TestDirectory::new_owned(registry, request));
        if let Err(error) = &result {
            // An allocation can commit before pinning the directory fails.
            // Never allow clearing the in-memory run to forget that record.
            self.attention(format!(
                "テスト音声の所有を確認できません。未完了データは保持しています: {error}"
            ));
        }
        result
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn reads_never_collect_and_startup_collects_only_the_journaled_private_audio() {
        let fixture = tempfile::tempdir().unwrap();
        let registry = crate::capture_test::journal::Registry::open(fixture.path()).unwrap();
        let area = registry
            .allocate(crate::capture_test::tests::request())
            .unwrap();
        let path = area.path();
        std::fs::write(path.join("system.caf"), b"synthetic PCM").unwrap();
        drop(area);
        drop(registry);
        let mut owner = Ownership::default();
        owner.configure(fixture.path()).unwrap();
        assert!(owner.blocked());
        assert_eq!(owner.retry_revision(), None);
        for _ in 0..3 {
            assert_eq!(owner.snapshot().phase, "checking");
            assert!(path.exists());
        }
        assert_eq!(owner.reconcile().phase, "clear");
        assert_eq!(owner.retry_revision(), None);
        assert!(!path.exists());
        assert!(!owner.blocked());
    }
    #[test]
    fn unknown_payload_is_retained_and_background_retry_rejects_stale_revision() {
        let fixture = tempfile::tempdir().unwrap();
        let registry = crate::capture_test::journal::Registry::open(fixture.path()).unwrap();
        let area = registry
            .allocate(crate::capture_test::tests::request())
            .unwrap();
        let path = area.path();
        let id = registry.pending().unwrap()[0].request.run_id.clone();
        std::fs::write(path.join("unregistered"), b"keep").unwrap();
        drop(area);
        drop(registry);
        let mut owner = Ownership::default();
        owner.configure(fixture.path()).unwrap();
        let status = owner.reconcile();
        assert_eq!(status.phase, "attention");
        assert_eq!(owner.retry_revision(), Some(status.revision));
        assert_eq!(status.run_id.as_deref(), Some(id.as_str()));
        assert!(owner.require_clear().is_err());
        assert!(owner.require_retry(status.revision - 1).is_err());
        assert!(path.exists());
        owner.require_retry(status.revision).unwrap();
        assert_eq!(owner.reconcile().phase, "attention");
        assert_ne!(owner.retry_revision(), Some(status.revision));
        assert!(owner.require_retry(status.revision).is_err());
        // Explicit fixture repair, never a production deletion fallback.
        std::fs::remove_file(path.join("unregistered")).unwrap();
        assert_eq!(owner.reconcile().phase, "clear");
        assert!(!path.exists());
        assert_eq!(owner.retry_revision(), None);
    }
    #[test]
    fn untrusted_root_does_not_abort_or_reset_the_application_and_cannot_admit_capture() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join(".mon-capture-tests");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("keep"), b"foreign bytes").unwrap();
        let mut owner = Ownership::default();
        owner.configure(fixture.path()).unwrap();
        for _ in 0..2 {
            assert_eq!(owner.reconcile().phase, "attention");
            assert!(owner.require_clear().is_err());
            assert_eq!(std::fs::read(root.join("keep")).unwrap(), b"foreign bytes");
        }
    }
    #[test]
    fn missing_application_data_root_can_initialize_without_resetting_an_existing_owner() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("app-data");
        let mut owner = Ownership::default();
        owner.configure(&root).unwrap();
        assert_eq!(owner.reconcile().phase, "clear");
        assert!(owner.configure(fixture.path()).is_err());
        assert!(root.join(".mon-capture-tests/root-proof.json").is_file());
    }
}
