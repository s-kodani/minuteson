//! Hashing/copying holds the same admission gate as local inference. Recording
//! and shutdown request cancellation, but keep waiting until the actual work
//! (including partial-copy DB finalization) has released its lease.
use super::*;
use std::sync::Arc;

pub(crate) struct ModelPreparationLease<'a> {
    owner: &'a AnalysisController,
    cancel: Arc<AtomicBool>,
}

struct WakeWorker<'a>(&'a AppHandle);

impl Drop for WakeWorker<'_> {
    fn drop(&mut self) {
        // Run after the actual lease drops, even if a blocking task panics or
        // the IPC receiver disappears. Recording/exit/import gates still win.
        let _ = AnalysisController::kick(self.0);
    }
}

impl AnalysisController {
    pub(crate) fn with_model_preparation<T>(
        app: &AppHandle,
        operation_id: Option<&str>,
        cancelled: &impl Fn() -> bool,
        work: impl FnOnce(&ModelPreparationLease<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        crate::internal_logs::observe(
            crate::internal_logs::Context {
                category: "model".into(),
                operation_id: operation_id.map(str::to_owned),
                ..Default::default()
            },
            || {
                app.state::<Storage>().require_global_data_writable()?;
                let _wake = WakeWorker(app);
                let controller = app.state::<Self>();
                let importing = app
                    .state::<crate::import_controller::ImportController>()
                    .snapshot()?;
                let lease = controller.begin_model_preparation(
                    app.state::<crate::session::SessionStore>()
                        .snapshot()
                        .is_active(),
                    importing.busy || importing.recording_blocked,
                    cancelled,
                )?;
                app.state::<Storage>().require_global_data_writable()?;
                lease.wait_for_worker(cancelled, |waiting| {
                    crate::internal_logs::phase(
                        "model",
                        operation_id.map(str::to_owned),
                        None,
                        if waiting { "waiting" } else { "preparing" },
                        None,
                    );
                    if let Some(operation_id) = operation_id {
                        let _ = app.emit(
                            "model-preparation-wait",
                            serde_json::json!({
                                "operationId": operation_id, "waiting": waiting,
                            }),
                        );
                    }
                })?;
                work(&lease)
            },
        )
    }

    pub(crate) fn begin_model_preparation(
        &self,
        recording_active: bool,
        import_blocked: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<ModelPreparationLease<'_>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if cancelled()
            || recording_active
            || import_blocked
            || self.shutdown.load(Ordering::SeqCst)
            || state.blocks_worker()
            || !state.meeting_deletions.is_empty()
        {
            return Err("AI処理・録音・取り込み・モデル準備・バックアップ・復元・終了処理が停止してからモデルを準備してください".into());
        }
        // Reserve admission before waiting. A worker woken by the preceding
        // download/copy step may still be running; it must finish safely, but
        // cannot acquire another queued job ahead of this preparation.
        let cancel = Arc::new(AtomicBool::new(false));
        state.model_preparation = Some(cancel.clone());
        Ok(ModelPreparationLease {
            owner: self,
            cancel,
        })
    }
}

impl ModelPreparationLease<'_> {
    pub(super) fn wait_for_worker(
        &self,
        cancelled: &impl Fn() -> bool,
        waiting: impl Fn(bool),
    ) -> Result<(), String> {
        let mut state = self
            .owner
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        let was_busy = state.busy;
        if was_busy {
            waiting(true);
        }
        loop {
            if cancelled() || self.cancelled() {
                return Err(
                    "モデル準備の待機を取り消しました。現在の状態を確認してください".into(),
                );
            }
            if !state.busy {
                break;
            }
            state = self
                .owner
                .idle
                .wait_timeout(state, Duration::from_millis(100))
                .map_err(|_| "AI処理の完了待ちに失敗しました")?
                .0;
        }
        if was_busy {
            waiting(false);
        }
        Ok(())
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst) || self.owner.shutdown.load(Ordering::SeqCst)
    }
}

impl Drop for ModelPreparationLease<'_> {
    fn drop(&mut self) {
        let mut state = self.owner.state.lock().unwrap_or_else(|e| e.into_inner());
        // A stale guard can never release a newer preparation operation.
        if state
            .model_preparation
            .as_ref()
            .is_some_and(|cancel| Arc::ptr_eq(cancel, &self.cancel))
        {
            state.model_preparation = None;
            self.owner.idle.notify_all();
        }
    }
}
