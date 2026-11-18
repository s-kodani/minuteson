//! Display-only startup recovery inventory. Never grants storage authority.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager, WebviewWindow};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
    pub origin: String,
    pub target_session_id: Option<String>,
    pub items: Vec<Item>,
    pub error: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            run_id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            phase: "idle".into(),
            origin: "startup".into(),
            target_session_id: None,
            items: vec![],
            error: None,
        }
    }
}
#[derive(Default)]
pub(crate) struct Recovery(Mutex<Snapshot>, AtomicBool);
struct Reservation<'a>(&'a Recovery);
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut snapshot = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        self.0 .1.store(false, Ordering::Release);
        // Readers may have observed terminal items while the worker still
        // owned its reservation. Publish a newer version once it truly ends.
        if snapshot.phase == "completed" {
            snapshot.revision += 1;
        }
    }
}
impl Recovery {
    fn reserve(&self) -> Result<Reservation<'_>, String> {
        self.1
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "音声の保存処理が進行中です".to_string())?;
        Ok(Reservation(self))
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        let current = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut snapshot = current.clone();
        if self.1.load(Ordering::Acquire) && snapshot.phase == "completed" {
            snapshot.phase = "running".into();
        }
        snapshot
    }
    pub(crate) fn busy(&self) -> bool {
        self.1.load(Ordering::Acquire)
            || matches!(self.snapshot().phase.as_str(), "scanning" | "running")
    }
    #[cfg(test)]
    fn update(&self, change: impl FnOnce(&mut Snapshot)) -> Snapshot {
        let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut s);
        s.revision += 1;
        s.clone()
    }
    pub(crate) fn require_exit_ready(
        &self,
        expected: Option<&crate::application_close::RecordingSaveExpectation>,
    ) -> Result<(), String> {
        if self.busy() {
            return Err(
                "録音の保存が完了していません。完了を確認してから終了してください。".into(),
            );
        }
        if let Some(expected) = expected {
            expected.require_complete(&self.snapshot())?;
        }
        Ok(())
    }
}
fn publish(app: &tauri::AppHandle, change: impl FnOnce(&mut Snapshot)) {
    let recovery = app.state::<Recovery>();
    // Serialize state changes and lifecycle publication together. Another
    // producer cannot enqueue an older outer state after an inner result.
    let s = {
        let mut current = recovery.0.lock().unwrap_or_else(|e| e.into_inner());
        let previous = current.clone();
        change(&mut current);
        current.revision += 1;
        for entry in history_changes(&previous, &current) {
            crate::job_history::record(entry);
        }
        current.clone()
    };
    let _ = app.emit_to("main", "recording-recovery-changed", s);
}
fn history_changes(previous: &Snapshot, current: &Snapshot) -> Vec<crate::job_history::Entry> {
    current
        .items
        .iter()
        .filter_map(|item| {
            if !["queued", "running", "succeeded", "failed", "skipped"]
                .contains(&item.phase.as_str())
            {
                return None;
            }
            let prior = previous
                .items
                .iter()
                .find(|p| p.session_id == item.session_id);
            if prior.is_some_and(|prior| {
                prior.phase == item.phase
                    && prior.error == item.error
                    && prior.started_at_ms == item.started_at_ms
                    && prior.meeting_title == item.meeting_title
            }) {
                return None;
            }
            let mut entry = crate::job_history::Entry::new(
                "recording_save",
                &item.session_id,
                &item.meeting_title,
                Some(item.meeting_id.clone()),
                if item.phase == "skipped" {
                    "cancelled"
                } else {
                    &item.phase
                },
            );
            entry.accepted_at = item.started_at_ms;
            entry.detail = if item.phase == "skipped" {
                Some("保存対象の録音または音声が削除されたため、録音の保存を終了しました。".into())
            } else {
                item.error.clone()
            };
            Some(entry)
        })
        .collect()
}

/// The recording controller delegates presentation and history from stopping
/// onward. The finalizer owns terminal results; delayed outer notifications
/// cannot replace one with a stale running/error state.
pub(crate) fn observe_recorder(
    app: &tauri::AppHandle,
    previous: &crate::session::RecordingSnapshot,
    current: &crate::session::RecordingSnapshot,
) {
    use crate::session::RecordingPhase;
    let Some(session) = current.session_id.as_ref().or(previous.session_id.as_ref()) else {
        return;
    };
    let Some(meeting) = current.meeting_id.as_ref().or(previous.meeting_id.as_ref()) else {
        return;
    };
    if !matches!(
        current.phase,
        RecordingPhase::Stopping | RecordingPhase::Saving | RecordingPhase::Error
    ) || !matches!(
        current.phase,
        RecordingPhase::Stopping | RecordingPhase::Saving
    ) && !matches!(
        previous.phase,
        RecordingPhase::Stopping | RecordingPhase::Saving
    ) {
        return;
    }
    let title = app
        .state::<crate::storage::Storage>()
        .get_meeting(meeting)
        .ok()
        .flatten()
        .map(|m| m.title)
        .unwrap_or_else(|| "録音した会議".into());
    publish(app, |s| {
        observe_recorder_state(s, session, meeting, &title, current)
    });
}

fn observe_recorder_state(
    s: &mut Snapshot,
    session: &str,
    meeting: &str,
    title: &str,
    current: &crate::session::RecordingSnapshot,
) {
    use crate::session::RecordingPhase;
    if s.items.iter().any(|i| {
        i.session_id == session && ["succeeded", "failed", "skipped"].contains(&i.phase.as_str())
    }) {
        return;
    }
    s.origin = "stop".into();
    s.target_session_id = Some(session.into());
    let failed = current.phase == RecordingPhase::Error;
    s.phase = if failed { "completed" } else { "running" }.into();
    s.error = None;
    let now = chrono::Utc::now().timestamp_millis();
    if let Some(item) = s.items.iter_mut().find(|i| i.session_id == session) {
        item.phase = if failed { "failed" } else { "running" }.into();
        item.error = current.error_message.clone();
        if current.progress.is_some() {
            item.progress = current.progress.clone();
        }
    } else {
        s.items.push(Item {
            session_id: session.into(),
            meeting_id: meeting.into(),
            meeting_title: title.into(),
            phase: if failed { "failed" } else { "running" }.into(),
            started_at_ms: Some(now),
            error: current.error_message.clone(),
            progress: current.progress.clone(),
            no_audio: false,
        });
    }
}
pub(crate) fn run(app: &tauri::AppHandle) -> Result<(), String> {
    let recovery = app.state::<Recovery>();
    let _reservation = recovery.reserve()?;
    run_reserved(app, None, "startup")
}
pub(crate) fn save_closed(app: &tauri::AppHandle, session: &str) -> Result<(), String> {
    let recovery = app.state::<Recovery>();
    let _reservation = recovery.reserve()?;
    run_reserved(app, Some(session), "stop")?;
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
fn run_reserved(app: &tauri::AppHandle, session: Option<&str>, origin: &str) -> Result<(), String> {
    let progress_app = app.clone();
    let _observations =
        crate::operation_observation::Scope::enter("recording_save", move |value| {
            publish(&progress_app, |s| {
                if let Some(item) = s.items.iter_mut().find(|i| i.phase == "running") {
                    item.progress = Some(value);
                }
            });
        });
    publish(app, |s| {
        s.origin = origin.into();
        s.target_session_id = session.map(str::to_owned);
        s.phase = "scanning".into();
        s.error = None;
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.state::<crate::storage::Storage>()
            .recover_recordings_observed_for(
                session,
                &|mut items| {
                    publish(app, |s| {
                        s.phase = "running".into();
                        for item in &mut items {
                            if let Some(prior) =
                                s.items.iter().find(|p| p.session_id == item.session_id)
                            {
                                item.started_at_ms = prior.started_at_ms.or(item.started_at_ms);
                                if origin == "stop" {
                                    item.phase = "running".into();
                                }
                            }
                        }
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
                &|mut item| {
                    publish(app, |s| {
                        if let Some(prior) =
                            s.items.iter_mut().find(|p| p.session_id == item.session_id)
                        {
                            item.started_at_ms = prior.started_at_ms.or(item.started_at_ms);
                            if item.phase == "running" {
                                item.progress = prior.progress.clone();
                            }
                            *prior = item;
                        }
                    })
                },
            )
    }))
    .unwrap_or_else(|_| Err("録音の保存処理が停止しました。音声を保持しています".into()));
    let error = result.err().or_else(|| {
        let snapshot = app.state::<Recovery>().snapshot();
        session
            .filter(|id| {
                !snapshot.items.iter().any(|item| {
                    item.session_id == *id
                        && ["succeeded", "failed", "skipped"].contains(&item.phase.as_str())
                })
            })
            .map(|_| "録音の保存対象を確認できません。元音声は保持しています".to_string())
    });
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
        .filter(|i| i.phase == "succeeded" && session.is_none_or(|id| id == i.session_id))
        .map(|i| i.meeting_id.clone())
        .collect();
    if !ids.is_empty() {
        crate::internal_logs::transition(
            crate::internal_logs::Context {
                category: "recovery".into(),
                ..Default::default()
            },
            "succeeded",
            "info",
            &format!("前回の音声保存 {} 件を検証して復旧しました。", ids.len()),
        );
        for item in s.items.iter().filter(|item| {
            item.phase == "succeeded" && session.is_none_or(|id| id == item.session_id)
        }) {
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
            window
                .state::<crate::analysis_worker::AnalysisController>()
                .require_not_exiting()?;
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
                s.origin = "retry".into();
                s.target_session_id = Some(session_id.clone());
                s.error = None;
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
                let _reservation = Reservation(&recovery);
                if let Err(error) = run_reserved(&app, Some(&session_id), "retry") {
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
            return Err("録音の保存状態はメイン画面だけで確認できます".into());
        }
        Ok(window.state::<Recovery>().snapshot())
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_recording_recovery", message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(session: &str, phase: &str) -> Item {
        Item {
            session_id: session.into(),
            meeting_id: format!("meeting-{session}"),
            meeting_title: "会議".into(),
            phase: phase.into(),
            started_at_ms: Some(100),
            error: None,
            progress: None,
            no_audio: false,
        }
    }

    #[test]
    fn lifecycle_uses_one_session_history_and_progress_does_not_republish_another_result() {
        let initial = Snapshot::default();
        let mut running = initial.clone();
        running.items = vec![item("one", "running"), item("two", "queued")];
        let records = history_changes(&initial, &running);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id, "recording_save:one");
        assert!(records.iter().all(|entry| entry.kind == "recording_save"));
        let mut failed = running.clone();
        failed.items[0].phase = "failed".into();
        failed.items[0].error = Some("保存できません".into());
        assert_eq!(history_changes(&running, &failed).len(), 1);
        let mut next = failed.clone();
        next.items[1].progress = Some(crate::operation_observation::Progress {
            stage: "verifying_files".into(),
            completed: 1,
            total: None,
            unit: "bytes",
        });
        assert!(history_changes(&failed, &next).is_empty());
        next.items[0].phase = "queued".into();
        next.items[0].error = None;
        let retry = history_changes(&failed, &next);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].id, records[0].id);
        assert_eq!(retry[0].accepted_at, Some(100));
    }

    #[test]
    fn recorder_stopping_starts_the_same_save_and_late_outer_states_do_not_replace_the_result() {
        let mut snapshot = Snapshot::default();
        let mut recording = crate::session::RecordingSnapshot {
            phase: crate::session::RecordingPhase::Stopping,
            session_id: Some("one".into()),
            meeting_id: Some("meeting-one".into()),
            ..Default::default()
        };
        observe_recorder_state(&mut snapshot, "one", "meeting-one", "会議", &recording);
        assert_eq!(snapshot.origin, "stop");
        assert_eq!(snapshot.target_session_id.as_deref(), Some("one"));
        assert_eq!(snapshot.items.len(), 1);
        let started = snapshot.items[0].started_at_ms;
        recording.phase = crate::session::RecordingPhase::Saving;
        observe_recorder_state(&mut snapshot, "one", "meeting-one", "会議", &recording);
        assert_eq!(snapshot.items[0].started_at_ms, started);
        snapshot.items[0].phase = "succeeded".into();
        snapshot.phase = "completed".into();
        let completed = snapshot.clone();
        recording.phase = crate::session::RecordingPhase::Error;
        recording.error_message = Some("遅延した外側の失敗".into());
        observe_recorder_state(&mut snapshot, "one", "meeting-one", "会議", &recording);
        assert_eq!(snapshot.items, completed.items);
        assert!(history_changes(&completed, &snapshot).is_empty());
    }

    #[test]
    fn exit_waits_for_the_actual_worker_even_after_its_terminal_snapshot_is_published() {
        let recovery = Recovery::default();
        let reservation = recovery.reserve().unwrap();
        recovery.update(|snapshot| snapshot.phase = "completed".into());
        let pending = recovery.snapshot();
        assert_eq!(pending.phase, "running");
        assert!(recovery.require_exit_ready(None).is_err());
        drop(reservation);
        let complete = recovery.snapshot();
        assert_eq!(complete.phase, "completed");
        assert!(complete.revision > pending.revision);
        assert!(recovery.require_exit_ready(None).is_ok());
    }

    #[test]
    fn skipped_inventory_settles_the_same_queued_history_as_cancelled() {
        let mut queued = Snapshot::default();
        queued.items = vec![item("deleted", "queued")];
        let mut skipped = queued.clone();
        skipped.items[0].phase = "skipped".into();
        let records = history_changes(&queued, &skipped);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "recording_save:deleted");
        assert_eq!(records[0].state, "cancelled");
        assert_eq!(records[0].accepted_at, Some(100));
        assert!(records[0].finished_at.is_some());
        assert!(records[0]
            .detail
            .as_ref()
            .is_some_and(|reason| reason.contains("削除")));
        assert!(history_changes(&skipped, &skipped).is_empty());
    }
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
