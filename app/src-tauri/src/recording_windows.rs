//! Recording companions exist only for a recording. Creation/destruction runs
//! on a blocking worker under RecorderController's transition, never in a
//! synchronous Windows command or event callback.
use crate::recorder::RecorderController;
use crate::session::{RecordingSnapshot, SessionStore};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder, WindowEvent};

const LABELS: [&str; 2] = ["recorder", "recording-confirmation"];
const DESTROY_WAIT: Duration = Duration::from_secs(5);

#[cfg(all(test, windows))]
mod native_tests;

#[derive(Default)]
pub(crate) struct RecordingWindows(Mutex<Vec<Companion>>);

struct Companion {
    window: WebviewWindow,
    destroyed: mpsc::Receiver<()>,
    closing: bool,
}

impl Companion {
    fn attach(window: WebviewWindow) -> Self {
        let (send, destroyed) = mpsc::channel();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                // The UI callback must never wait on the lifecycle/recorder
                // mutex held by the worker waiting for this notification.
                let _ = send.send(());
            }
        });
        Self {
            window,
            destroyed,
            closing: false,
        }
    }

    fn destroy(&mut self) -> Result<(), String> {
        if !self.closing {
            self.window.destroy().map_err(|error| error.to_string())?;
            self.closing = true;
        }
        // destroy() enqueues native work. Keep the label reserved until the
        // Destroyed event confirms Tauri has removed the old WebView/listeners.
        // A timeout leaves this entry closing, so a retry cannot reuse it.
        self.destroyed
            .recv_timeout(DESTROY_WAIT)
            .map_err(|error| format!("録音補助画面の終了を確認できません: {error}"))
    }
}

fn appearance(window: &WebviewWindow) -> Result<(), String> {
    let window = window.clone();
    let app = window.app_handle().clone();
    // This is queued before capture can publish its first session change.
    // Native creation is the admission boundary; cosmetic initialization must
    // not hold the recording transition waiting for the UI event loop.
    app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        let result = crate::rounded_windows::configure_window(&window);
        #[cfg(windows)]
        let result = crate::windows_appearance::configure_window(&window);
        #[cfg(not(any(target_os = "macos", windows)))]
        let result: Result<(), String> = Ok(());
        if let Err(error) = result {
            report_error(&error);
        }
    })
    .map_err(|error| error.to_string())
}

fn destroy_all(windows: &mut Vec<Companion>) -> Result<(), String> {
    let mut failures = Vec::new();
    let mut index = 0;
    while index < windows.len() {
        match windows[index].destroy() {
            Ok(()) => {
                windows.remove(index);
            }
            Err(error) => {
                failures.push(error);
                index += 1;
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// Called after input admission, before a draft or capture helper is created.
/// The caller owns the recording transition on a blocking thread.
pub(crate) fn prepare(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<RecordingWindows>();
    let mut windows = state
        .0
        .lock()
        .map_err(|_| "録音補助画面の状態を取得できません")?;
    // A previous native destruction may have timed out. Finish observing it
    // before creating another window with either of the same labels.
    if windows.iter().any(|window| window.closing) {
        destroy_all(&mut windows)?;
    }
    let result = (|| {
        for label in LABELS {
            if windows.iter().any(|entry| entry.window.label() == label) {
                continue;
            }
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|config| config.label == label)
                .ok_or("録音補助画面の設定がありません")?;
            let window = WebviewWindowBuilder::from_config(app, config)
                .map_err(|error| error.to_string())?
                .visible(false)
                .focused(false)
                .build()
                .map_err(|error| error.to_string())?;
            windows.push(Companion::attach(window.clone()));
            appearance(&window)?;
        }
        Ok(())
    })();
    if result.is_err() {
        if let Err(error) = destroy_all(&mut windows) {
            report_error(&error);
        }
    }
    result
}

fn same_terminal(current: &RecordingSnapshot, expected: &RecordingSnapshot) -> bool {
    !current.is_active()
        && current.revision == expected.revision
        && current.session_id == expected.session_id
}

/// The caller owns the transition. No active capture or newer terminal state
/// may be affected by cleanup queued for a previous recording/start attempt.
pub(crate) fn release_if_current(
    app: &AppHandle,
    expected: &RecordingSnapshot,
) -> Result<(), String> {
    if !same_terminal(&app.state::<SessionStore>().snapshot(), expected)
        || RecorderController::has_capture(app)?
    {
        return Ok(());
    }
    let state = app.state::<RecordingWindows>();
    let mut windows = state
        .0
        .lock()
        .map_err(|_| "録音補助画面の状態を取得できません")?;
    destroy_all(&mut windows)
}

pub(crate) fn terminal_published(app: &AppHandle, snapshot: &RecordingSnapshot) {
    if snapshot.is_active() {
        return;
    }
    let handle = app.clone();
    let expected = snapshot.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) =
            RecorderController::with_transition(&handle, || release_if_current(&handle, &expected))
        {
            report_error(&error);
        }
    });
}

fn report_error(error: &str) {
    crate::internal_logs::phase("recording_windows", None, None, "failed", Some(error));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::RecordingPhase;

    #[test]
    fn companions_are_templates_in_normal_and_qa_configuration() {
        for source in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.qa.conf.json"),
        ] {
            let config: tauri::Config = serde_json::from_str(source).unwrap();
            for label in LABELS {
                let window = config
                    .app
                    .windows
                    .iter()
                    .find(|w| w.label == label)
                    .unwrap();
                assert!(!window.create);
                assert!(!window.visible);
            }
            assert!(
                config
                    .app
                    .windows
                    .iter()
                    .find(|w| w.label == "main")
                    .unwrap()
                    .create
            );
        }
    }

    #[test]
    fn old_cleanup_cannot_release_new_recording_or_newer_terminal_state() {
        let expected = RecordingSnapshot {
            revision: 9,
            phase: RecordingPhase::Error,
            session_id: Some("previous".into()),
            ..Default::default()
        };
        assert!(same_terminal(&expected, &expected));
        for phase in [
            RecordingPhase::Preparing,
            RecordingPhase::Recording,
            RecordingPhase::Stopping,
            RecordingPhase::Saving,
        ] {
            let mut current = expected.clone();
            current.phase = phase;
            assert!(!same_terminal(&current, &expected));
        }
        let mut current = expected.clone();
        current.revision += 1;
        assert!(!same_terminal(&current, &expected));
        current = expected.clone();
        current.session_id = Some("next".into());
        assert!(!same_terminal(&current, &expected));
    }
}
