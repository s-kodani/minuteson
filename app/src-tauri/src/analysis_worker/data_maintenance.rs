//! Actual data work shares admission with inference, recording and model work.
//! Cancellation never releases admission before the worker finishes cleanup.
use super::*;
use std::sync::Arc;

pub(crate) struct DataMaintenanceLease {
    state: Arc<Mutex<State>>,
    idle: Arc<Condvar>,
    shutdown: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    scope: MaintenanceScope,
}

enum MaintenanceScope {
    Global,
    Meeting(String),
    ModelDeletion,
}

pub(crate) struct ExportAdmissionLease(Arc<Mutex<State>>);
pub(super) struct NoticeWriter {
    pub(super) cancel: Arc<AtomicBool>,
    meeting: Option<String>,
}
pub(crate) struct NoticeStorageLease {
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
    idle: Arc<Condvar>,
    cancel: Arc<AtomicBool>,
}

struct MeetingReservation<'a> {
    storage: &'a Storage,
    meeting: &'a str,
    keep: bool,
}
impl Drop for MeetingReservation<'_> {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.storage.release_meeting_deletion(self.meeting);
        }
    }
}

impl AnalysisController {
    pub(crate) fn notice_storage_busy(&self, meeting: Option<&str>) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .notice_writers
            .iter()
            .any(|w| meeting.is_none() || w.meeting.as_deref() == meeting)
    }
    pub(crate) fn notice_storage_paused(&self, meeting: Option<&str>) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.data_maintenance.is_some()
            || state.storage_restart_required
            || self.shutdown.load(Ordering::SeqCst)
            || meeting.is_some_and(|id| state.meeting_deletions.contains_key(id))
    }
    pub(crate) fn require_storage_restart(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.storage_restart_required = true;
        state.cancel_notice_writers();
        state.wake = false;
    }
    // Informational only. Actual starts recheck all gates atomically.
    pub(crate) fn data_operation_blocker(&self) -> Result<Option<String>, String> {
        // Short notification reads/writes drain after maintenance reserves its
        // admission. Sampling them here makes idle backup buttons flicker.
        self.operation_blocker(true)
    }
    pub(crate) fn deletion_operation_blocker(&self) -> Result<Option<String>, String> {
        self.operation_blocker(true)
    }
    fn operation_blocker(&self, ignore_notice_writers: bool) -> Result<Option<String>, String> {
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
        } else if state.model_deletion.is_some() {
            Some("モデルの削除確認・処理中")
        } else if !state.meeting_deletions.is_empty() {
            Some("会議の削除確認・処理中")
        } else if state.recording_intent {
            Some("録音開始・録音処理中")
        } else if state.import_intent {
            Some("音声取り込み中")
        } else if state.model_preparation.is_some() {
            Some("AIモデルの検査・準備中")
        } else if state.export_intent {
            Some("Markdownの書き出し中")
        } else if !ignore_notice_writers && !state.notice_writers.is_empty() {
            Some("録音通知履歴の確認・更新中")
        } else if state.busy {
            Some("AI処理中")
        } else {
            None
        }
        .map(str::to_owned))
    }

    /// Startup reset can replace a refused/restart-fenced database, while
    /// retaining the same exclusion of active recording/import/model work.
    pub(crate) fn begin_startup_reset_maintenance(
        &self,
        recording_active: bool,
        import_blocked: bool,
    ) -> Result<DataMaintenanceLease, String> {
        self.begin_data_maintenance_inner(recording_active, import_blocked, &|| false, true, true)
            .map_err(|_| {
                "録音・取り込み・モデル処理などが終了するまで、データを初期化できません".to_owned()
            })
    }

    pub(crate) fn begin_data_maintenance(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        self.begin_data_maintenance_inner(recording_active, import_blocked, cancelled, false, false)
    }

    pub(crate) fn begin_data_maintenance_wait(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        self.begin_data_maintenance_inner(recording_active, import_blocked, cancelled, false, true)
    }

    pub(crate) fn begin_deletion_wait(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        self.begin_data_maintenance_inner(recording_active, import_blocked, cancelled, false, true)
    }

    pub(crate) fn meeting_deletion_blocker(&self) -> Result<Option<String>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        Ok(if self.shutdown.load(Ordering::SeqCst) {
            Some("終了処理中")
        } else if state.storage_restart_required {
            Some("保存先の切替後は終了して再起動が必要です")
        } else if state.data_maintenance.is_some()
            || state.model_deletion.is_some()
            || !state.meeting_deletions.is_empty()
        {
            Some("別のデータ操作が進行中です")
        } else if state.model_preparation.is_some() {
            Some("AIモデルの検査・準備中")
        } else {
            None
        }
        .map(str::to_owned))
    }

    pub(crate) fn begin_meeting_deletion(
        &self,
        storage: &Storage,
        meeting: &str,
    ) -> Result<DataMaintenanceLease, String> {
        self.begin_meeting_deletion_wait(storage, meeting, &|| false, Duration::from_secs(5))
    }

    pub(super) fn begin_meeting_deletion_wait(
        &self,
        storage: &Storage,
        meeting: &str,
        cancelled: &impl Fn() -> bool,
        timeout: Duration,
    ) -> Result<DataMaintenanceLease, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if self.shutdown.load(Ordering::SeqCst)
            || state.storage_restart_required
            || state.data_maintenance.is_some()
            || state.model_deletion.is_some()
            || state.model_preparation.is_some()
            || !state.meeting_deletions.is_empty()
        {
            return Err("別のデータ操作・モデル準備・終了処理の完了後に削除してください".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state
            .meeting_deletions
            .insert(meeting.into(), cancel.clone());
        drop(state);
        let lease = DataMaintenanceLease {
            state: self.state.clone(),
            idle: self.idle.clone(),
            shutdown: self.shutdown.clone(),
            cancel,
            scope: MaintenanceScope::Meeting(meeting.into()),
        };
        let mut state = self.state.lock().map_err(|_| "削除予約を確認できません")?;
        let started = Instant::now();
        while state
            .notice_writers
            .iter()
            .any(|w| w.meeting.as_deref() == Some(meeting))
        {
            if cancelled() || lease.cancelled() || started.elapsed() >= timeout {
                drop(state);
                return Err(
                    "通知履歴の処理が終わらないため、データ操作を開始できませんでした".into(),
                );
            }
            state = self
                .idle
                .wait_timeout(state, Duration::from_millis(25))
                .map_err(|_| "通知履歴の停止待ちを確認できません")?
                .0;
        }
        if cancelled() || lease.cancelled() {
            drop(state);
            return Err("削除の開始待ちを取り消しました".into());
        }
        // A different meeting's restored notice can hold SQLite and validate
        // its notice lease. Never wait for SQLite while holding this mutex.
        drop(state);
        storage.reserve_meeting_deletion(meeting)?;
        let mut reservation = MeetingReservation {
            storage,
            meeting,
            keep: false,
        };
        if cancelled() || lease.cancelled() {
            return Err("削除の開始待ちを取り消しました".into());
        }
        lease.validate_active()?;
        reservation.keep = true;
        Ok(lease)
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn begin_deletion_maintenance(
        &self,
        storage: &Storage,
        operation: &str,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        crate::storage::deletion::operations::authorize_review(storage, Some(operation))?;
        self.begin_data_maintenance_inner(
            recording_active,
            import_blocked,
            cancelled,
            storage.deletion_resume_restart_required(),
            true,
        )
    }

    fn begin_data_maintenance_inner(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
        deletion_resume: bool,
        wait_notices: bool,
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
            || state.model_deletion.is_some()
            || !state.meeting_deletions.is_empty()
            || state.export_intent
            || (!wait_notices && !state.notice_writers.is_empty())
        {
            return Err("AI処理・録音・取り込み・モデル準備・書き出し・終了処理が停止してからバックアップ／復元してください".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.data_maintenance = Some(cancel.clone());
        drop(state);
        let lease = DataMaintenanceLease {
            state: self.state.clone(),
            idle: self.idle.clone(),
            shutdown: self.shutdown.clone(),
            cancel,
            scope: MaintenanceScope::Global,
        };
        let mut state = self.state.lock().map_err(|_| "削除予約を確認できません")?;
        let started = Instant::now();
        while !state.notice_writers.is_empty() {
            if cancelled() || lease.cancelled() || started.elapsed() >= Duration::from_secs(5) {
                drop(state);
                return Err("通知履歴の処理が終わらないため、削除を開始できませんでした".into());
            }
            state = self
                .idle
                .wait_timeout(state, Duration::from_millis(25))
                .map_err(|_| "通知履歴の停止待ちを確認できません")?
                .0;
        }
        drop(state);
        if cancelled() || lease.cancelled() {
            return Err("削除の開始待ちを取り消しました".into());
        }
        Ok(lease)
    }

    #[cfg(any(unix, windows))]
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
            true,
        )
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn begin_model_deletion_maintenance(
        &self,
        storage: &Storage,
        operation: &str,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        crate::storage::deletion::models::intent::authorize_review(storage, operation)?;
        self.begin_model_deletion_inner(
            recording_active,
            import_blocked,
            cancelled,
            storage.deletion_resume_restart_required(),
        )
    }

    pub(crate) fn begin_model_deletion(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DataMaintenanceLease, String> {
        self.begin_model_deletion_inner(recording_active, import_blocked, cancelled, false)
    }

    fn begin_model_deletion_inner(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
        deletion_resume: bool,
    ) -> Result<DataMaintenanceLease, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "モデル削除の所有を取得できません")?;
        if cancelled()
            || recording_active
            || import_blocked
            || self.shutdown.load(Ordering::SeqCst)
            || (state.storage_restart_required && !deletion_resume)
            || state.busy
            || state.recording_intent
            || state.import_intent
            || state.model_preparation.is_some()
            || state.data_maintenance.is_some()
            || state.model_deletion.is_some()
            || !state.meeting_deletions.is_empty()
            || state.export_intent
        {
            return Err("AI処理・録音・取り込み・モデル準備・データ操作・書き出し・終了処理が停止してからモデルを削除してください".into());
        }
        // Model files have no notification-history IO. Reserve the same start
        // gates as maintenance, but never pause or drain notification owners.
        let cancel = Arc::new(AtomicBool::new(false));
        state.model_deletion = Some(cancel.clone());
        Ok(DataMaintenanceLease {
            state: self.state.clone(),
            idle: self.idle.clone(),
            shutdown: self.shutdown.clone(),
            cancel,
            scope: MaintenanceScope::ModelDeletion,
        })
    }

    pub(crate) fn begin_export(&self) -> Result<ExportAdmissionLease, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if state.data_maintenance.is_some()
            || state.model_deletion.is_some()
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
    // data copying, meeting/audio deletion or root cutover. Model deletion is
    // independent. Never reuse the Markdown lease or
    // make recording wait for a notification fsync.
    pub(crate) fn begin_notice_storage(&self) -> Result<NoticeStorageLease, String> {
        self.begin_scoped_notice_storage(None)
    }
    pub(crate) fn begin_scoped_notice_storage(
        &self,
        meeting: Option<&str>,
    ) -> Result<NoticeStorageLease, String> {
        self.try_begin_scoped_notice_storage(meeting)?
            .ok_or_else(|| "データ操作・保存先切替・終了処理中は通知履歴を保存できません".into())
    }
    pub(crate) fn try_begin_scoped_notice_storage(
        &self,
        meeting: Option<&str>,
    ) -> Result<Option<NoticeStorageLease>, String> {
        if meeting.is_some_and(|id| !crate::input_levels::valid_owner(id)) {
            return Err("通知履歴の会議IDが不正です".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "通知履歴の共有所有を取得できません")?;
        if state.data_maintenance.is_some()
            || state.storage_restart_required
            || self.shutdown.load(Ordering::SeqCst)
            || meeting.is_some_and(|id| state.meeting_deletions.contains_key(id))
        {
            return Ok(None);
        }
        if state.notice_writers.len() >= 128 {
            return Err("通知履歴の共有所有が上限に達しました".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state.notice_writers.push(NoticeWriter {
            cancel: cancel.clone(),
            meeting: meeting.map(str::to_owned),
        });
        Ok(Some(NoticeStorageLease {
            state: self.state.clone(),
            shutdown: self.shutdown.clone(),
            idle: self.idle.clone(),
            cancel,
        }))
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
            || !state
                .notice_writers
                .iter()
                .any(|w| Arc::ptr_eq(&w.cancel, &self.cancel))
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
            .retain(|w| !Arc::ptr_eq(&w.cancel, &self.cancel));
        self.idle.notify_all();
    }
}

impl DataMaintenanceLease {
    pub(crate) fn meeting(&self) -> Option<&str> {
        match &self.scope {
            MaintenanceScope::Meeting(id) => Some(id),
            _ => None,
        }
    }
    pub(crate) fn is_model_deletion(&self) -> bool {
        matches!(self.scope, MaintenanceScope::ModelDeletion)
    }
    pub(crate) fn validate_meeting(&self, meeting: &str) -> Result<(), String> {
        self.validate_active()?;
        if self.meeting().is_some_and(|id| id != meeting) {
            return Err("削除予約の会議が一致しません".into());
        }
        Ok(())
    }
    pub(crate) fn validate_active(&self) -> Result<(), String> {
        if self.is_model_deletion() {
            return Err("モデル削除の所有では会議・音声・全体保守を実行できません".into());
        }
        self.validate_owned()
    }
    pub(crate) fn validate_model_deletion(&self) -> Result<(), String> {
        if !self.is_model_deletion() {
            return Err("モデル削除専用の実行排他を保持していません。対象を保持します".into());
        }
        self.validate_owned()
    }
    fn validate_owned(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "共有データ操作の所有を確認できません")?;
        let owner = match &self.scope {
            MaintenanceScope::Global => state.data_maintenance.as_ref(),
            MaintenanceScope::Meeting(id) => state.meeting_deletions.get(id),
            MaintenanceScope::ModelDeletion => state.model_deletion.as_ref(),
        };
        if self.cancelled() || !owner.is_some_and(|active| Arc::ptr_eq(active, &self.cancel)) {
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
        if let Some(meeting) = self.meeting() {
            if state
                .meeting_deletions
                .get(meeting)
                .is_some_and(|active| Arc::ptr_eq(active, &self.cancel))
            {
                state.meeting_deletions.remove(meeting);
                self.idle.notify_all();
            }
        }
        let owner = match self.scope {
            MaintenanceScope::Global => &mut state.data_maintenance,
            MaintenanceScope::ModelDeletion => &mut state.model_deletion,
            MaintenanceScope::Meeting(_) => return,
        };
        if owner
            .as_ref()
            .is_some_and(|cancel| Arc::ptr_eq(cancel, &self.cancel))
        {
            *owner = None;
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
