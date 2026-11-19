//! Isolated companion WebViews only: no capture helper, microphone, meeting
//! database or normal application setup is used by this native lifetime check.
use super::*;
use crate::session::RecordingPhase;
use std::time::Instant;
use tauri::WebviewUrl;

#[allow(deprecated)]
fn worker(
    app: &mut tauri::App,
    action: impl FnOnce(AppHandle) -> Result<(), String> + Send + 'static,
) {
    let handle = app.handle().clone();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(action(handle));
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.run_iteration(|_, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
        match receive.try_recv() {
            Ok(result) => return result.unwrap(),
            Err(mpsc::TryRecvError::Disconnected) => panic!("companion worker disconnected"),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        assert!(
            Instant::now() < deadline,
            "companion lifecycle worker stalled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "creates isolated hidden native WebViews; run explicitly on a Windows desktop"]
fn companions_are_absent_idle_and_recreated_after_destroyed() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context
        .config_mut()
        .app
        .windows
        .retain(|window| LABELS.contains(&window.label.as_str()));
    for window in &mut context.config_mut().app.windows {
        window.create = false;
        window.url = WebviewUrl::External("about:blank".parse().unwrap());
        window.data_directory = Some(temp.path().join(&window.label));
    }
    crate::window_chrome::configure(context.config_mut());
    let mut app = tauri::Builder::default()
        .any_thread()
        .manage(RecordingWindows::default())
        .manage(RecorderController::default())
        .manage(SessionStore::default())
        .build(context)
        .unwrap();
    assert!(app.webview_windows().is_empty());
    for cycle in 0..3 {
        worker(&mut app, |handle| {
            RecorderController::with_transition(&handle, || prepare(&handle))
        });
        assert_eq!(app.webview_windows().len(), 2);
        for label in LABELS {
            assert!(!app.get_webview_window(label).unwrap().is_visible().unwrap());
        }
        let previous = app.state::<SessionStore>().snapshot();
        app.state::<SessionStore>().update(|snapshot| {
            snapshot.phase = RecordingPhase::Recording;
            snapshot.session_id = Some(format!("cycle-{cycle}"));
        });
        // A delayed cleanup from before this recording cannot close its cards.
        worker(&mut app, move |handle| {
            RecorderController::with_transition(&handle, || release_if_current(&handle, &previous))
        });
        assert_eq!(app.webview_windows().len(), 2);
        let terminal = app.state::<SessionStore>().update(|snapshot| {
            snapshot.phase = RecordingPhase::Idle;
            snapshot.session_id = None;
        });
        worker(&mut app, move |handle| {
            RecorderController::with_transition(&handle, || release_if_current(&handle, &terminal))
        });
        assert!(app.webview_windows().is_empty());
        assert!(app.state::<RecordingWindows>().0.lock().unwrap().is_empty());
    }
    app.cleanup_before_exit();
}
