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

impl AnalysisController {
    // Informational only. Actual starts recheck all gates atomically.
    pub(crate) fn data_operation_blocker(&self) -> Result<Option<String>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        Ok(if self.shutdown.load(Ordering::SeqCst) {
            Some("終了処理中")
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
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if cancelled()
            || recording_active
            || import_blocked
            || self.shutdown.load(Ordering::SeqCst)
            || state.busy
            || state.blocks_worker()
            || state.export_intent
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

    pub(crate) fn begin_export(&self) -> Result<ExportAdmissionLease, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if state.data_maintenance.is_some()
            || state.export_intent
            || self.shutdown.load(Ordering::SeqCst)
        {
            return Err("バックアップ・復元・書き出し・終了処理が進行中です".into());
        }
        state.export_intent = true;
        Ok(ExportAdmissionLease(self.state.clone()))
    }
}

impl DataMaintenanceLease {
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
