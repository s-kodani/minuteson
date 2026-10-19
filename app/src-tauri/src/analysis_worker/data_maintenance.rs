//! Actual data work shares admission with inference, recording and model work.
//! Cancellation never releases admission before the worker finishes cleanup.
use super::*;
use std::sync::Arc;

pub(crate) struct DataMaintenanceLease {
    state: Arc<Mutex<State>>,
    idle: Arc<Condvar>,
    shutdown: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

pub(crate) struct ExportAdmissionLease(Arc<Mutex<State>>);
pub(crate) struct NoticeStorageLease {
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
    idle: Arc<Condvar>,
    cancel: Arc<AtomicBool>,
}

impl AnalysisController {
    pub(crate) fn require_storage_restart(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.storage_restart_required = true;
        state.cancel_notice_writers();
        state.wake = false;
    }
    // Informational only. Actual starts recheck all gates atomically.
    pub(crate) fn data_operation_blocker(&self) -> Result<Option<String>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        Ok(if self.shutdown.load(Ordering::SeqCst) {
            Some("終了処理中")
        } else if state.storage_restart_required {
            Some("保存先の切替後は終了して再起動が必要です")
        } else if state.data_maintenance.is_some() {
            Some("バックアップ／復元中")
        } else if state.recording_intent {
            Some("録音開始・録音処理中")
        } else if state.import_intent {
            Some("音声取り込み中")
        } else if state.model_preparation.is_some() {
            Some("AIモデルの検査・準備中")
        } else if state.export_intent {
            Some("Markdownの書き出し中")
        } else if !state.notice_writers.is_empty() {
            Some("録音通知履歴の保存中")
        } else if state.busy {
            Some("AI処理中")
        } else {
            None
        }
        .map(str::to_owned))
    }

    pub(crate) fn begin_data_maintenance(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        self.begin_data_maintenance_inner(recording_active, import_blocked, cancelled, false)
    }

    #[cfg(unix)]
    pub(crate) fn begin_deletion_maintenance(
        &self,
        storage: &Storage,
        operation: &str,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        crate::storage::deletion::operations::authorize_review(storage, Some(operation))?;
        storage.clear_analysis_pcm();
        self.begin_data_maintenance_inner(
            recording_active,
            import_blocked,
            cancelled,
            storage.deletion_resume_restart_required(),
        )
    }

    fn begin_data_maintenance_inner(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
        deletion_resume: bool,
    ) -> Result<DataMaintenanceLease, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if cancelled()
            || recording_active
            || import_blocked
            || self.shutdown.load(Ordering::SeqCst)
            || state.busy
            || (state.storage_restart_required && !deletion_resume)
            || state.recording_intent
            || state.import_intent
            || state.model_preparation.is_some()
            || state.data_maintenance.is_some()
            || state.export_intent
            || !state.notice_writers.is_empty()
        {
            return Err("AI処理・録音・取り込み・モデル準備・書き出し・終了処理が停止してからバックアップ／復元してください".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.data_maintenance = Some(cancel.clone());
        Ok(DataMaintenanceLease {
            state: self.state.clone(),
            idle: self.idle.clone(),
            shutdown: self.shutdown.clone(),
            cancel,
        })
    }

    #[cfg(unix)]
    pub(crate) fn begin_audio_deletion_maintenance(
        &self,
        storage: &Storage,
        operation: &str,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        crate::storage::deletion::audio::intent::authorize_review(storage, operation)?;
        self.begin_data_maintenance_inner(
            recording_active,
            import_blocked,
            cancelled,
            storage.deletion_resume_restart_required(),
        )
    }

    #[cfg(unix)]
    pub(crate) fn begin_model_deletion_maintenance(
        &self,
        storage: &Storage,
        operation: &str,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        crate::storage::deletion::models::intent::authorize_review(storage, operation)?;
        self.begin_data_maintenance_inner(
            recording_active,
            import_blocked,
            cancelled,
            storage.deletion_resume_restart_required(),
        )
    }

    pub(crate) fn begin_export(&self) -> Result<ExportAdmissionLease, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if state.data_maintenance.is_some()
            || state.export_intent
            || state.storage_restart_required
            || self.shutdown.load(Ordering::SeqCst)
        {
            return Err("バックアップ・復元・書き出し・終了処理が進行中です".into());
        }
        state.export_intent = true;
        Ok(ExportAdmissionLease(self.state.clone()))
    }
    // Companion-file writes may coexist with inference/recording, but not with
    // data copying, deletion or root cutover. Never reuse the Markdown lease or
    // make recording wait for a notification fsync.
    pub(crate) fn begin_notice_storage(&self) -> Result<NoticeStorageLease, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "通知履歴の共有所有を取得できません")?;
        if state.data_maintenance.is_some()
            || state.storage_restart_required
            || self.shutdown.load(Ordering::SeqCst)
        {
            return Err("データ操作・保存先切替・終了処理中は通知履歴を保存できません".into());
        }
        if state.notice_writers.len() >= 128 {
            return Err("通知履歴の共有所有が上限に達しました".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.notice_writers.push(cancel.clone());
        Ok(NoticeStorageLease {
            state: self.state.clone(),
            shutdown: self.shutdown.clone(),
            idle: self.idle.clone(),
            cancel,
        })
    }
}

impl NoticeStorageLease {
    pub(crate) fn validate_active(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "通知履歴の共有所有を確認できません")?;
        if self.cancel.load(Ordering::SeqCst)
            || self.shutdown.load(Ordering::SeqCst)
            || state.storage_restart_required
            || state.data_maintenance.is_some()
            || !state
                .notice_writers
                .iter()
                .any(|c| Arc::ptr_eq(c, &self.cancel))
        {
            return Err("通知履歴の共有所有は終了しました".into());
        }
        Ok(())
    }
}
impl Drop for NoticeStorageLease {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .notice_writers
            .retain(|c| !Arc::ptr_eq(c, &self.cancel));
        self.idle.notify_all();
    }
}

impl DataMaintenanceLease {
    pub(crate) fn validate_active(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "共有データ操作の所有を確認できません")?;
        if self.cancelled()
            || !state
                .data_maintenance
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &self.cancel))
        {
            return Err("削除に必要な共有実行排他を保持していません。対象を保持します".into());
        }
        Ok(())
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst) || self.shutdown.load(Ordering::SeqCst)
    }
}

impl Drop for DataMaintenanceLease {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .data_maintenance
            .as_ref()
            .is_some_and(|cancel| Arc::ptr_eq(cancel, &self.cancel))
        {
            state.data_maintenance = None;
            self.idle.notify_all();
        }
    }
}

impl Drop for ExportAdmissionLease {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .export_intent = false;
    }
}
