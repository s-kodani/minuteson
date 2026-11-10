use crate::storage::{JobSnapshot, Storage};
use mon_core::operation::{self, Active, Decision, Kind, Request, Scope};
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

#[cfg(test)]
mod model_deletion_tests;

#[derive(Default)]
struct State {
    busy: bool,
    wake: bool,
    recording_intent: bool,
    import_intent: bool,
    model_preparation: Option<std::sync::Arc<AtomicBool>>,
    data_maintenance: Option<std::sync::Arc<AtomicBool>>,
    model_deletion: Option<std::sync::Arc<AtomicBool>>,
    meeting_deletions: std::collections::HashMap<String, std::sync::Arc<AtomicBool>>,
    export_intent: bool,
    export_owner: Option<std::sync::Arc<AtomicBool>>,
    storage_restart_required: bool,
    notice_writers: Vec<data_maintenance::NoticeWriter>,
}

impl State {
    fn admission(&self, request: Request<'_>, shutdown: bool) -> Decision {
        // Ordering also controls the first informative blocker returned to UI.
        let mut active = Vec::new();
        let mut global = |kind, enabled| {
            if enabled {
                active.push(Active {
                    kind,
                    scope: Scope::Global,
                });
            }
        };
        global(Kind::Shutdown, shutdown);
        global(Kind::Restart, self.storage_restart_required);
        global(Kind::GlobalMaintenance, self.data_maintenance.is_some());
        global(Kind::ModelDeletion, self.model_deletion.is_some());
        for meeting in self.meeting_deletions.keys() {
            active.push(Active {
                kind: Kind::MeetingDeletion,
                scope: Scope::Meeting(meeting),
            });
        }
        for (kind, enabled) in [
            (Kind::Recording, self.recording_intent),
            (Kind::Import, self.import_intent),
            (Kind::ModelPreparation, self.model_preparation.is_some()),
            (Kind::Export, self.export_intent),
        ] {
            if enabled {
                active.push(Active {
                    kind,
                    scope: Scope::Global,
                });
            }
        }
        for writer in &self.notice_writers {
            active.push(Active {
                kind: Kind::Notice,
                scope: writer
                    .meeting
                    .as_deref()
                    .map_or(Scope::Unscoped, Scope::Meeting),
            });
        }
        if self.busy {
            active.push(Active {
                kind: Kind::Worker,
                scope: Scope::Global,
            });
        }
        operation::decide(request, &active)
    }
    fn blocks_worker(&self) -> bool {
        !self
            .admission(Request::new(Kind::Worker, Scope::Global), false)
            .can_reserve()
    }

    fn cancel_model_preparation(&self) {
        if let Some(cancel) = &self.model_preparation {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    fn cancel_model_deletion(&self) {
        if let Some(cancel) = &self.model_deletion {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    fn cancel_notice_writers(&self) {
        for writer in &self.notice_writers {
            writer.cancel.store(true, Ordering::SeqCst);
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
        if app
            .try_state::<crate::startup_attention::Startup>()
            .is_some_and(|s| !s.ready_for_work())
        {
            return Ok(());
        }
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
                crate::internal_logs::report_error(
                    "analysis",
                    "解析ワーカーが予期せず停止しました",
                );
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
            if let Err(error) = app.state::<Storage>().drain_post_analysis(&stopping) {
                crate::internal_logs::report_error("post_analysis", &error);
                let _ = app.emit("analysis-worker-error", error.to_string());
            }
            if let Err(error) =
                crate::analysis_flow::advance(&app.state::<Storage>(), &stopping, &|flow| {
                    if flow.stage == "review"
                        && app
                            .state::<Storage>()
                            .load_current_analysis(&flow.meeting_id)
                            .ok()
                            .flatten()
                            .is_some_and(|v| {
                                Some(v.id)
                                    == flow
                                        .diarization_job
                                        .as_ref()
                                        .and_then(|j| j.output_analysis_id.clone())
                            })
                    {
                        let _ = app.emit("analysis-version-changed", &flow.meeting_id);
                        if let Err(error) = crate::speaker_completion::after_analysis(
                            &app.state::<Storage>(),
                            &flow.meeting_id,
                        ) {
                            crate::internal_logs::report_error("speaker_completion", &error);
                            let _ = app.emit(
                                "analysis-worker-error",
                                format!("話者補完の開始を確認できません：{}", error),
                            );
                        }
                    }
                    let _ = app.emit("analysis-flow-changed", flow);
                })
            {
                crate::internal_logs::report_error("analysis", &error.to_string());
                let _ = app.emit("analysis-worker-error", error.to_string());
            }
            let claimed = {
                let Ok(mut state) = controller.state.lock() else {
                    return;
                };
                if controller.shutdown.load(Ordering::SeqCst) || state.blocks_worker() {
                    return;
                }
                state.wake = false;
                app.state::<Storage>().acquire_owned_inference_job()
            };
            match claimed {
                Ok(Some((job, _meeting_owner))) => {
                    let _log_scope =
                        crate::internal_logs::Scope::enter(crate::internal_logs::Context {
                            category: job.kind.to_string(),
                            meeting_id: Some(job.meeting_id.clone()),
                            job_id: Some(job.id.clone()),
                            operation_id: Some(job.operation_id.clone()),
                        });
                    emit(app, job.clone());
                    let stopping = || controller.shutdown.load(Ordering::SeqCst);
                    let result = match job.kind {
                        crate::storage::JobKind::Transcription => crate::transcription::process(
                            &app.state::<Storage>(),
                            &job.id,
                            &stopping,
                            &|job| emit(app, job),
                        ),
                        crate::storage::JobKind::MinutesGeneration => {
                            crate::generation::process_observed(
                                &app.state::<Storage>(),
                                &job.id,
                                &stopping,
                                &|job| emit(app, job),
                                &|runtime| {
                                    crate::internal_logs::runtime(&runtime);
                                    let _ = app.emit("minutes-runtime-progress", runtime);
                                },
                            )
                            .map_err(mon_contracts::ProcessingError::from)
                        }
                        crate::storage::JobKind::SpeakerCompletion => {
                            crate::speaker_completion::process(
                                &app.state::<Storage>(),
                                &job.id,
                                &stopping,
                                &|job| emit(app, job),
                            )
                            .map_err(mon_contracts::ProcessingError::from)
                        }
                        crate::storage::JobKind::VoiceRegistration
                        | crate::storage::JobKind::VoiceMatching => crate::voice::process(
                            &app.state::<Storage>(),
                            &job.id,
                            &stopping,
                            &|job| emit(app, job),
                        )
                        .map_err(mon_contracts::ProcessingError::from),
                        crate::storage::JobKind::Diarization => {
                            crate::diarization::process_observed(
                                &app.state::<Storage>(),
                                &job.id,
                                &stopping,
                                &|job| emit(app, job),
                                &|runtime| {
                                    let _ = app.emit("diarization-runtime-progress", runtime);
                                },
                            )
                            .map_err(mon_contracts::ProcessingError::from)
                        }
                    };
                    if result.is_ok()
                        && app
                            .state::<Storage>()
                            .get_job(&job.id)
                            .is_ok_and(|j| j.state == "succeeded")
                    {
                        if job.kind == "speaker_completion" {
                            let _ = app.emit("analysis-version-changed", &job.meeting_id);
                            if let Err(e) = app.state::<Storage>().drain_post_analysis(&stopping) {
                                crate::internal_logs::report_error("voice_matching", &e);
                            }
                        } else if job.kind == "diarization"
                            && app
                                .state::<Storage>()
                                .load_current_analysis(&job.meeting_id)
                                .ok()
                                .flatten()
                                .is_some_and(|v| {
                                    Some(v.id)
                                        == app
                                            .state::<Storage>()
                                            .get_job(&job.id)
                                            .ok()
                                            .and_then(|j| j.output_analysis_id)
                                })
                        {
                            if let Err(e) = crate::speaker_completion::after_analysis(
                                &app.state::<Storage>(),
                                &job.meeting_id,
                            ) {
                                crate::internal_logs::report_error("speaker_completion", &e);
                                let _ = app.emit(
                                    "analysis-worker-error",
                                    format!("話者補完の開始を確認できません：{}", e),
                                );
                            }
                        }
                    }
                    if result.is_ok()
                        && matches!(job.kind.as_str(), "voice_registration" | "voice_matching")
                        && app
                            .state::<Storage>()
                            .get_job(&job.id)
                            .is_ok_and(|j| j.state == "succeeded")
                    {
                        let _ = app.emit("voice-identity-changed", &job.meeting_id);
                    }
                    if let Err(error) = result {
                        let storage = app.state::<Storage>();
                        let failed = storage.get_job(&job.id).and_then(|current| {
                            if current.state == "failed" {
                                return Ok(current);
                            }
                            storage.finish_job(
                                &job.id,
                                current.completed_units,
                                "failed",
                                Some(
                                    if job.kind == crate::storage::JobKind::Transcription
                                        && error.is_quality_rejection()
                                    {
                                        "transcription_quality_failed"
                                    } else {
                                        job.kind.failure_code()
                                    },
                                ),
                            )
                        });
                        // Retain detailed diagnostics locally, without transcript text.
                        crate::internal_logs::report_error("analysis", &error.to_string());
                        if let Some(failed) = failed.ok().or_else(|| {
                            storage
                                .get_job(&job.id)
                                .ok()
                                .filter(|current| current.state == "failed")
                        }) {
                            emit(app, failed);
                        } else {
                            let _ = app.emit("analysis-worker-error", error.to_string());
                        }
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
                    crate::internal_logs::report_error("analysis", &error.to_string());
                    let _ = app.emit("analysis-worker-error", error.to_string());
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
            if !state
                .admission(
                    Request::new(Kind::Recording, Scope::Global),
                    controller.shutdown.load(Ordering::SeqCst),
                )
                .can_reserve()
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
            crate::internal_logs::report_error("analysis", &error.to_string());
            let _ = app.emit("analysis-worker-error", error.to_string());
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
            if cancelled()
                || !state
                    .admission(
                        Request::new(Kind::Import, Scope::Global),
                        c.shutdown.load(Ordering::SeqCst),
                    )
                    .can_reserve()
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
            crate::internal_logs::report_error("analysis", &error.to_string());
            let _ = app.emit("analysis-worker-error", error.to_string());
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
            for cancel in state.meeting_deletions.values() {
                cancel.store(true, Ordering::SeqCst);
            }
            state.cancel_notice_writers();
            state.cancel_model_preparation();
            state.cancel_model_deletion();
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
        if !state
            .admission(
                Request::new(Kind::Shutdown, Scope::Global),
                self.shutdown.load(Ordering::SeqCst),
            )
            .can_reserve()
        {
            return Err("書き出し・録音開始・終了処理が進行中です".into());
        }
        self.shutdown.store(true, Ordering::SeqCst);
        state.cancel_notice_writers();
        if cancel_work {
            for cancel in state.meeting_deletions.values() {
                cancel.store(true, Ordering::SeqCst);
            }
            state.cancel_model_preparation();
            state.cancel_model_deletion();
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
            || !state.meeting_deletions.is_empty()
            || state.model_preparation.is_some()
            || state.data_maintenance.is_some()
            || state.model_deletion.is_some()
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
    crate::internal_logs::job(&job);
    let _ = app.emit("analysis-job-changed", job);
}
