use crate::storage::{JobSnapshot, Storage};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Condvar, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(test)]
#[path = "analysis_worker_tests.rs"]
mod tests;

#[derive(Default)]
struct State {
    busy: bool,
    wake: bool,
    recording_intent: bool,
}

#[derive(Default)]
pub struct AnalysisController {
    state: Mutex<State>,
    idle: Condvar,
    shutdown: AtomicBool,
}

impl AnalysisController {
    pub fn kick(app: &AppHandle) -> Result<(), String> {
        let controller = app.state::<Self>();
        let mut state = controller
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        if controller.shutdown.load(Ordering::SeqCst) || state.recording_intent {
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
                state.wake && !state.recording_intent && !controller.shutdown.load(Ordering::SeqCst)
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
            let claimed = {
                let Ok(mut state) = controller.state.lock() else {
                    return;
                };
                if controller.shutdown.load(Ordering::SeqCst) || state.recording_intent {
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
                        !state.recording_intent && !controller.shutdown.load(Ordering::SeqCst)
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
        let controller = app.state::<Self>();
        {
            let mut state = controller
                .state
                .lock()
                .map_err(|_| "解析ロックを取得できません")?;
            if state.recording_intent || controller.shutdown.load(Ordering::SeqCst) {
                return Err("録音開始または終了処理が進行中です".into());
            }
            app.state::<Storage>().request_pause_for_recording()?;
            state.recording_intent = true;
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
        let _ = Self::kick(app);
    }

    pub fn shutdown(app: &AppHandle) -> Result<(), String> {
        let controller = app.state::<Self>();
        {
            let _state = controller
                .state
                .lock()
                .map_err(|_| "解析ロックを取得できません")?;
            app.state::<Storage>().pause_for_exit()?;
            controller.shutdown.store(true, Ordering::SeqCst);
        }
        controller.wait_idle(Duration::from_secs(15))
    }

    fn wait_idle(&self, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "解析ロックを取得できません")?;
        while state.busy {
            let remaining = timeout
                .checked_sub(started.elapsed())
                .ok_or("解析の安全な停止を待っています。完了後に再試行してください")?;
            state = self
                .idle
                .wait_timeout(state, remaining)
                .map_err(|_| "解析の停止待ちに失敗しました")?
                .0;
        }
        Ok(())
    }
}

fn emit(app: &AppHandle, job: JobSnapshot) {
    let _ = app.emit("analysis-job-changed", job);
}
