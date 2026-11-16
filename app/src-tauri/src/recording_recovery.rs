//! Display-only startup recovery inventory. Never grants storage authority.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager, WebviewWindow};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub session_id: String,
    pub meeting_id: String,
    pub meeting_title: String,
    pub phase: String,
    pub started_at_ms: Option<i64>,
    pub error: Option<String>,
    pub progress: Option<crate::operation_observation::Progress>,
    pub no_audio: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub run_id: String,
    pub revision: u64,
    pub phase: String,
    pub items: Vec<Item>,
    pub error: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            run_id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            phase: "idle".into(),
            items: vec![],
            error: None,
        }
    }
}
#[derive(Default)]
pub(crate) struct Recovery(Mutex<Snapshot>, AtomicBool);
struct Reservation<'a>(&'a AtomicBool);
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Recovery {
    fn reserve(&self) -> Result<Reservation<'_>, String> {
        self.1
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "音声の保存処理が進行中です".to_string())?;
        Ok(Reservation(&self.1))
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub(crate) fn busy(&self) -> bool {
        self.1.load(Ordering::Acquire)
            || matches!(self.snapshot().phase.as_str(), "scanning" | "running")
    }
    fn update(&self, change: impl FnOnce(&mut Snapshot)) -> Snapshot {
        let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut s);
        s.revision += 1;
        s.clone()
    }
}
fn publish(app: &tauri::AppHandle, change: impl FnOnce(&mut Snapshot)) {
    let s = app.state::<Recovery>().update(change);
    for item in &s.items {
        observe_history(app, item);
    }
    let _ = app.emit_to("main", "recording-recovery-changed", s);
}
pub(crate) fn observe_history(app: &tauri::AppHandle, item: &Item) {
    if !["queued", "running", "succeeded", "failed"].contains(&item.phase.as_str()) {
        return;
    }
    let run = app.state::<Recovery>().snapshot().run_id;
    let mut entry = crate::job_history::Entry::new(
        "recording_recovery",
        &format!("{run}:{}", item.session_id),
        &item.meeting_title,
        Some(item.meeting_id.clone()),
        &item.phase,
    );
    entry.accepted_at = item.started_at_ms;
    entry.detail = item.error.clone();
    crate::job_history::record(entry);
}
pub(crate) fn run(app: &tauri::AppHandle) -> Result<(), String> {
    let recovery = app.state::<Recovery>();
    let _reservation = recovery.reserve()?;
    run_reserved(app, None)
}
pub(crate) fn save_closed(app: &tauri::AppHandle, session: &str) -> Result<(), String> {
    let recovery = app.state::<Recovery>();
    let _reservation = recovery.reserve()?;
    run_reserved(app, Some(session))?;
    let snapshot = recovery.snapshot();
    let item = snapshot
        .items
        .iter()
        .find(|item| item.session_id == session)
        .ok_or("音声の保存対象が見つかりません")?;
    if item.phase != "succeeded" {
        return Err(item
            .error
            .clone()
            .unwrap_or_else(|| "音声を保存できませんでした".into()));
    }
    Ok(())
}
fn run_reserved(app: &tauri::AppHandle, session: Option<&str>) -> Result<(), String> {
    let progress_app = app.clone();
    let _observations =
        crate::operation_observation::Scope::enter("recording_recovery", move |value| {
            publish(&progress_app, |s| {
                if let Some(item) = s.items.iter_mut().find(|i| i.phase == "running") {
                    item.progress = Some(value);
                }
            });
        });
    publish(app, |s| s.phase = "scanning".into());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.state::<crate::storage::Storage>()
            .recover_recordings_observed_for(
                session,
                &|items| {
                    publish(app, |s| {
                        s.phase = "running".into();
                        if session.is_some() {
                            s.items.retain(|prior| {
                                !items.iter().any(|item| item.session_id == prior.session_id)
                            });
                            s.items.extend(items);
                        } else {
                            s.items = items;
                        }
                    })
                },
                &|item| {
                    publish(app, |s| {
                        if let Some(prior) =
                            s.items.iter_mut().find(|p| p.session_id == item.session_id)
                        {
                            *prior = item;
                        }
                    })
                },
            )
    }))
    .unwrap_or_else(|_| Err("録音回復の処理が停止しました。音声を保持しています".into()));
    let error = result.err();
    if let Some(error) = &error {
        crate::internal_logs::report_error("recovery", error);
    }
    publish(app, |s| {
        if let Some(error) = &error {
            for item in &mut s.items {
                if matches!(item.phase.as_str(), "queued" | "running") {
                    item.phase = "failed".into();
                    item.error = Some(error.clone());
                }
            }
        }
        s.phase = if error.is_some() {
            "failed"
        } else {
            "completed"
        }
        .into();
        s.error = error.clone();
    });
    let s = app.state::<Recovery>().snapshot();
    let ids: std::collections::BTreeSet<_> = s
        .items
        .iter()
        .filter(|i| i.phase == "succeeded")
        .map(|i| i.meeting_id.clone())
        .collect();
    if !ids.is_empty() {
        for item in s.items.iter().filter(|item| item.phase == "succeeded") {
            crate::recorder::clear_recovered_error(app, &item.session_id);
        }
        let _ = app.emit("recordings-recovered", ids.into_iter().collect::<Vec<_>>());
        let _ = crate::analysis_worker::AnalysisController::kick(app);
    }
    error.map_or(Ok(()), Err)
}
#[tauri::command]
pub(crate) fn retry_recording_recovery(
    window: WebviewWindow,
    session_id: String,
) -> Result<Snapshot, mon_contracts::AppError> {
    let result: Result<Snapshot, String> =
        crate::recorder::RecorderController::with_transition(window.app_handle(), || {
            if window.label() != "main" {
                return Err("音声の保存はメイン画面だけで再試行できます".into());
            }
            if !window
                .state::<crate::startup_attention::Startup>()
                .ready_for_work()
            {
                return Err("起動処理の完了後に保存を再試行してください".into());
            }
            let recovery = window.state::<Recovery>();
            if window
                .state::<crate::session::SessionStore>()
                .snapshot()
                .is_active()
            {
                return Err("録音終了後に音声の保存を再試行してください".into());
            }
            let reservation = recovery.reserve()?;
            if !recovery.snapshot().items.iter().any(|item| {
                item.session_id == session_id && item.phase == "failed" && !item.no_audio
            }) {
                return Err("再試行できる音声の保存処理がありません".into());
            }
            publish(window.app_handle(), |s| {
                s.phase = "running".into();
                if let Some(item) = s.items.iter_mut().find(|i| i.session_id == session_id) {
                    item.phase = "queued".into();
                    item.error = None;
                    item.progress = None;
                }
            });
            // Transfer the reservation to the owned background worker. Its guard
            // releases the flag on normal completion and unwinding.
            std::mem::forget(reservation);
            let app = window.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let recovery = app.state::<Recovery>();
                let _reservation = Reservation(&recovery.1);
                if let Err(error) = run_reserved(&app, Some(&session_id)) {
                    crate::internal_logs::report_error("recording_save", &error);
                }
            });
            Ok(recovery.snapshot())
        });
    result.map_err(|message| {
        mon_contracts::AppError::from_command("retry_recording_recovery", message)
    })
}
#[tauri::command]
pub(crate) fn get_recording_recovery(
    window: WebviewWindow,
) -> Result<Snapshot, mon_contracts::AppError> {
    let result: Result<Snapshot, String> = (|| {
        if window.label() != "main" {
            return Err("録音回復の状態はメイン画面だけで確認できます".into());
        }
        Ok(window.state::<Recovery>().snapshot())
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_recording_recovery", message))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_inventory_is_independent_and_versioned() {
        let recovery = Recovery::default();
        let idle = recovery.snapshot();
        assert!(!recovery.busy());
        let running = recovery.update(|s| s.phase = "scanning".into());
        assert!(recovery.busy());
        assert_eq!(idle.run_id, running.run_id);
        let done = recovery.update(|s| s.phase = "completed".into());
        assert!(!recovery.busy());
        assert!(done.revision > running.revision);
    }
    #[test]
    fn recovery_reservation_serializes_retries_and_releases_on_unwind() {
        let recovery = Recovery::default();
        let first = recovery.reserve().unwrap();
        assert!(recovery.busy());
        assert!(recovery.reserve().is_err());
        drop(first);
        let _ = std::panic::catch_unwind(|| {
            let _guard = recovery.reserve().unwrap();
            panic!("synthetic worker failure");
        });
        assert!(!recovery.busy());
        assert!(recovery.reserve().is_ok());
    }
}
