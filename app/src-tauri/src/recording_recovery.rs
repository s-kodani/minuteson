//! Display-only startup recovery inventory. Never grants storage authority.
use serde::Serialize;
use std::sync::Mutex;
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
pub(crate) struct Recovery(Mutex<Snapshot>);
impl Recovery {
    pub(crate) fn snapshot(&self) -> Snapshot {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub(crate) fn busy(&self) -> bool {
        matches!(self.snapshot().phase.as_str(), "scanning" | "running")
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
    let _ = app.emit_to("main", "recording-recovery-changed", s);
}
pub(crate) fn run(app: &tauri::AppHandle) {
    publish(app, |s| s.phase = "scanning".into());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.state::<crate::storage::Storage>()
            .recover_recordings_observed(
                &|items| {
                    publish(app, |s| {
                        s.phase = "running".into();
                        s.items = items;
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
        s.error = error;
    });
    let s = app.state::<Recovery>().snapshot();
    let ids: std::collections::BTreeSet<_> = s
        .items
        .iter()
        .filter(|i| i.phase == "succeeded")
        .map(|i| i.meeting_id.clone())
        .collect();
    if !ids.is_empty() {
        let _ = app.emit("recordings-recovered", ids.into_iter().collect::<Vec<_>>());
        let _ = crate::analysis_worker::AnalysisController::kick(app);
    }
}
#[tauri::command]
pub(crate) fn get_recording_recovery(window: WebviewWindow) -> Result<Snapshot, String> {
    if window.label() != "main" {
        return Err("録音回復の状態はメイン画面だけで確認できます".into());
    }
    Ok(window.state::<Recovery>().snapshot())
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
}
