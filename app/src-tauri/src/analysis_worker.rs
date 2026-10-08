use crate::storage::{JobSnapshot, Storage};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Condvar, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub(crate) mod data_maintenance;
mod model_preparation;

#[cfg(test)]
#[path = "analysis_worker_tests.rs"]
mod tests;

#[derive(Default)]
struct State {
    busy: bool,
    wake: bool,
    recording_intent: bool,
    import_intent: bool,
    model_preparation: Option<std::sync::Arc<AtomicBool>>,
    data_maintenance: Option<std::sync::Arc<AtomicBool>>,
    export_intent: bool,
    storage_restart_required: bool,
    notice_writers: Vec<std::sync::Arc<AtomicBool>>,
}

impl State {
    fn blocks_worker(&self) -> bool {
        self.storage_restart_required
            || self.recording_intent
            || self.import_intent
            || self.model_preparation.is_some()
            || self.data_maintenance.is_some()
    }

    fn cancel_model_preparation(&self) {
        if let Some(cancel) = &self.model_preparation {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    fn cancel_notice_writers(&self) {
        for cancel in &self.notice_writers {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
pub struct AnalysisController {
    state: std::sync::Arc<Mutex<State>>,
    idle: std::sync::Arc<Condvar>,
    shutdown: std::sync::Arc<AtomicBool>,
}

impl AnalysisController {
    pub fn kick(app: &AppHandle) -> Result<(), String> {
        if app.state::<Storage>().data_write_blocked()
            || app
                .state::<crate::capture_test::CaptureTests>()
                .recovery_blocked()
        {
            return Ok(());
        }
        let importing = app
            .state::<crate::import_controller::ImportController>()
            .snapshot()?;
        if importing.busy || importing.recording_blocked {
            return Ok(());
        }
        let controller = app.state::<Self>();
        let mut state = controller
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if controller.shutdown.load(Ordering::SeqCst) || state.blocks_worker() {
            return Ok(());
        }
        state.wake = true;
        if state.busy {
            return Ok(());
        }
        state.busy = true;
        let app = app.clone();
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::work(&app)));
            let controller = app.state::<Self>();
            let wake = if let Ok(mut state) = controller.state.lock() {
                state.busy = false;
                controller.idle.notify_all();
                state.wake && !state.blocks_worker() && !controller.shutdown.load(Ordering::SeqCst)
            } else {
                false
            };
            if result.is_err() {
                let _ = app.emit(
                    "analysis-worker-error",
                    "解析ワーカーが予期せず停止しました。再起動後に再開状態を確認してください",
                );
            }
            if wake {
                let _ = Self::kick(&app);
            }
        });
        Ok(())
    }

    fn work(app: &AppHandle) {
        loop {
            let controller = app.state::<Self>();
            let stopping = || {
                app.state::<Storage>().data_write_blocked()
                    || app
                        .state::<crate::capture_test::CaptureTests>()
                        .recovery_blocked()
                    || controller.shutdown.load(Ordering::SeqCst)
                    || controller
                        .state
                        .lock()
                        .map_or(true, |state| state.blocks_worker())
            };
            if stopping() {
                return;
            }
            if let Err(error) =
                crate::analysis_flow::advance(&app.state::<Storage>(), &stopping, &|flow| {
                    let _ = app.emit("analysis-flow-changed", flow);
                })
            {
                let _ = app.emit("analysis-worker-error", error);
            }
            let claimed = {
                let Ok(mut state) = controller.state.lock() else {
                    return;
                };
                if controller.shutdown.load(Ordering::SeqCst) || state.blocks_worker() {
                    return;
                }
                state.wake = false;
                app.state::<Storage>().acquire_local_inference_job()
            };
            match claimed {
                Ok(Some(job)) => {
                    emit(app, job.clone());
                    let stopping = || controller.shutdown.load(Ordering::SeqCst);
                    let result = match job.kind.as_str() {
                        "transcription" => crate::transcription::process(
                            &app.state::<Storage>(),
                            &job.id,
                            &stopping,
                            &|job| emit(app, job),
                        ),
                        "minutes_generation" => crate::generation::process(
                            &app.state::<Storage>(),
                            &job.id,
                            &stopping,
                            &|job| emit(app, job),
                        ),
                        "diarization" => crate::diarization::process(
                            &app.state::<Storage>(),
                            &job.id,
                            &stopping,
                            &|job| emit(app, job),
                        ),
                        _ => Err("未対応の解析工程です".into()),
                    };
                    if let Err(error) = result {
                        let storage = app.state::<Storage>();
                        if let Ok(current) = storage.get_job(&job.id) {
                            if let Ok(failed) = storage.finish_job(
                                &job.id,
                                current.completed_units,
                                "failed",
                                Some(match job.kind.as_str() {
                                    "minutes_generation" => "minutes_generation_failed",
                                    "diarization" => "diarization_failed",
                                    _ => "transcription_failed",
                                }),
                            ) {
                                emit(app, failed);
                            }
                        }
                        // Retain detailed diagnostics locally, without transcript text.
                        let _ = app.emit("analysis-worker-error", error);
                    }
                    let resume = controller.state.lock().is_ok_and(|state| {
                        !state.blocks_worker() && !controller.shutdown.load(Ordering::SeqCst)
                    });
                    if resume {
                        let _ = app.state::<Storage>().resume_recording_jobs();
                    }
                }
                Ok(None) => {
                    // Enqueue can wake us while the previous job is finishing.
                    // The thread epilogue clears busy and checks wake atomically.
                    let Ok(state) = controller.state.lock() else {
                        return;
                    };
                    if state.wake {
                        continue;
                    }
                    return;
                }
                Err(error) => {
                    let _ = app.emit("analysis-worker-error", error);
                    return;
                }
            }
        }
    }

    pub fn prepare_recording(app: &AppHandle) -> Result<(), String> {
        app.state::<Storage>().require_data_writable()?;
        let controller = app.state::<Self>();
        {
            let mut state = controller
                .state
                .lock()
                .map_err(|_| "解析ロックを取得できません")?;
            if state.recording_intent
                || state.import_intent
                || state.data_maintenance.is_some()
                || state.storage_restart_required
                || controller.shutdown.load(Ordering::SeqCst)
            {
                return Err("録音開始・取り込み・バックアップ・復元・終了処理が進行中です".into());
            }
            app.state::<Storage>().request_pause_for_recording()?;
            state.recording_intent = true;
            state.cancel_model_preparation();
        }
        if let Err(error) = controller.wait_idle(Duration::from_secs(15)) {
            Self::recording_finished(app);
            return Err(error);
        }
        Ok(())
    }

    pub fn recording_finished(app: &AppHandle) {
        let controller = app.state::<Self>();
        if let Err(error) = app.state::<Storage>().resume_recording_jobs() {
            let _ = app.emit("analysis-worker-error", error);
        }
        if let Ok(mut state) = controller.state.lock() {
            state.recording_intent = false;
        }
        crate::import_controller::ImportController::recording_finished(app);
        let _ = Self::kick(app);
    }

    pub(crate) fn prepare_import(
        app: &AppHandle,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        app.state::<Storage>().require_data_writable()?;
        let c = app.state::<Self>();
        {
            let mut state = c.state.lock().map_err(|_| "解析ロックを取得できません")?;
            if state.recording_intent
                || state.import_intent
                || state.model_preparation.is_some()
                || state.data_maintenance.is_some()
                || state.storage_restart_required
                || c.shutdown.load(Ordering::SeqCst)
                || cancelled()
            {
                return Err(
                    "録音開始・モデル準備・取り込み・バックアップ・復元・終了処理が進行中です"
                        .into(),
                );
            }
            if app
                .state::<crate::session::SessionStore>()
                .snapshot()
                .is_active()
            {
                return Err("録音中は取り込めません".into());
            }
            app.state::<Storage>().request_pause_for_recording()?;
            state.import_intent = true;
        }
        if let Err(error) = c.wait_idle_cancellable(Duration::from_secs(15), cancelled) {
            Self::import_finished(app);
            return Err(error);
        }
        Ok(())
    }
    pub(crate) fn import_finished(app: &AppHandle) {
        let c = app.state::<Self>();
        if let Ok(mut state) = c.state.lock() {
            state.import_intent = false;
        }
        if let Err(error) = app.state::<Storage>().resume_recording_jobs() {
            let _ = app.emit("analysis-worker-error", error);
        }
        let _ = Self::kick(app);
    }

    pub fn shutdown(app: &AppHandle) -> Result<(), String> {
        let controller = app.state::<Self>();
        {
            let state = controller
                .state
                .lock()
                .map_err(|_| "解析ロックを取得できません")?;
            if !app.state::<Storage>().data_write_blocked() {
                app.state::<Storage>().pause_for_exit()?;
            }
            controller.shutdown.store(true, Ordering::SeqCst);
            state.cancel_notice_writers();
            state.cancel_model_preparation();
            if let Some(cancel) = &state.data_maintenance {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        let result = controller.wait_idle_inner(Duration::from_secs(15), &|| false, true);
        if result.is_err() {
            // The app remains open on timeout. Jobs stay explicitly paused;
            // clear the exit gate so their eventual stop can be inspected/retried.
            controller.shutdown.store(false, Ordering::SeqCst);
        }
        result
    }

    // Seal admission before waiting for imports/data cleanup. This also closes
    // the previous export/start-recording race between exit checks and shutdown.
    pub(crate) fn begin_exit(&self) -> Result<(), String> {
        self.begin_exit_inner(true)
    }
    pub(crate) fn seal_exit(&self) -> Result<(), String> {
        self.begin_exit_inner(false)
    }
    pub(crate) fn close_activity(&self) -> Result<(bool, bool), String> {
        let state = self.state.lock().map_err(|_| "解析状態を取得できません")?;
        Ok((state.busy, state.model_preparation.is_some()))
    }
    fn begin_exit_inner(&self, cancel_work: bool) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if state.export_intent || state.recording_intent || self.shutdown.load(Ordering::SeqCst) {
            return Err("書き出し・録音開始・終了処理が進行中です".into());
        }
        self.shutdown.store(true, Ordering::SeqCst);
        state.cancel_notice_writers();
        if cancel_work {
            state.cancel_model_preparation();
            if let Some(cancel) = &state.data_maintenance {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    pub(crate) fn abort_shutdown(&self) {
        self.shutdown.store(false, Ordering::SeqCst);
    }

    fn wait_idle(&self, timeout: Duration) -> Result<(), String> {
        self.wait_idle_cancellable(timeout, &|| false)
    }
    fn wait_idle_cancellable(
        &self,
        timeout: Duration,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.wait_idle_inner(timeout, cancelled, false)
    }
    fn wait_idle_inner(
        &self,
        timeout: Duration,
        cancelled: &impl Fn() -> bool,
        include_notices: bool,
    ) -> Result<(), String> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        while state.busy
            || state.model_preparation.is_some()
            || state.data_maintenance.is_some()
            || (include_notices && !state.notice_writers.is_empty())
        {
            if cancelled() {
                return Err("解析停止の待機を取り消しました".into());
            }
            let remaining = timeout
                .checked_sub(started.elapsed())
                .ok_or("解析・モデル準備・データ操作の安全な停止を待っています。完了後に再試行してください")?;
            state = self
                .idle
                .wait_timeout(state, remaining.min(Duration::from_millis(100)))
                .map_err(|_| "解析の停止待ちに失敗しました")?
                .0;
        }
        if cancelled() {
            return Err("解析停止の待機を取り消しました".into());
        }
        Ok(())
    }
}

fn emit(app: &AppHandle, job: JobSnapshot) {
    let _ = app.emit("analysis-job-changed", job);
}
