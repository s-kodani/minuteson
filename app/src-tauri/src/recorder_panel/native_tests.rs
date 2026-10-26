//! Real HWNDs and WebViews, with an in-memory recording phase only. No capture
//! helper, microphone, meeting storage, or production application setup runs.
use super::*;
use std::time::{Duration, Instant};
use tauri::{WebviewUrl, WebviewWindowBuilder, WindowEvent};

// The application binary gets this dependency from Tauri's resource manifest.
// A libtest binary normally does not link the GUI builder; this native test
// does, so it also needs Common Controls v6 (including TaskDialogIndirect).
macro_rules! common_controls_manifest {
    ($directive:literal) => {
        #[used]
        #[link_section = ".drectve"]
        static COMMON_CONTROLS: [u8; $directive.len()] = *$directive;
    };
}
common_controls_manifest!(b" /MANIFESTDEPENDENCY:\"type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'\" ");

#[allow(deprecated)]
fn pump(app: &mut tauri::App, duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        app.run_iteration(|_, _| {});
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "opens isolated native test windows; run explicitly on a Windows desktop"]
fn minimized_restore_does_not_reopen_or_reactivate_the_panel() {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let temp = tempfile::tempdir().unwrap();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    let mut app = tauri::Builder::default()
        .any_thread()
        .manage(PanelVisibility::default())
        .manage(SessionStore::default())
        .manage(StopDecisionStore::default())
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::Focused(focused) = event {
                    main_focus_changed(window.app_handle(), *focused);
                }
            }
        })
        .build(context)
        .unwrap();
    let handle = app.handle().clone();
    let mut windows = Vec::new();
    for label in ["main", "recorder", "other"] {
        windows.push(
            WebviewWindowBuilder::new(
                &app,
                label,
                WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .title(format!("MON isolated panel test: {label}"))
            .data_directory(temp.path().join("webview"))
            .visible(false)
            .inner_size(260.0, 160.0)
            .always_on_top(label == "recorder")
            .build()
            .unwrap(),
        );
    }
    let (main, panel, other) = (&windows[0], &windows[1], &windows[2]);
    handle.state::<SessionStore>().update(|s| {
        s.phase = RecordingPhase::Recording;
        s.session_id = Some(uuid::Uuid::new_v4().to_string());
        s.meeting_id = Some(uuid::Uuid::new_v4().to_string());
    });
    let recording = handle.state::<SessionStore>().snapshot();
    other.show().unwrap();
    other.set_focus().unwrap();
    pump(&mut app, Duration::from_millis(200));
    let foreground = unsafe { GetForegroundWindow() };
    synchronize(&handle).unwrap();
    assert!(panel.is_visible().unwrap());
    assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    assert!(
        other.is_focused().unwrap(),
        "automatic panel display stole focus"
    );

    for _ in 0..10 {
        crate::residency::restore(&handle).unwrap();
        pump(&mut app, Duration::from_millis(200));
        assert!(main.is_focused().unwrap());
        assert!(!panel.is_visible().unwrap());
        main.minimize().unwrap();
        pump(&mut app, Duration::from_millis(200));
        assert!(main.is_minimized().unwrap());
        assert!(panel.is_visible().unwrap());
        // Same entry point as the recorder's open_main_window command.
        crate::residency::restore(&handle).unwrap();
        // A delayed blur must not override the actual restored focus.
        main_focus_changed(&handle, false);
        pump(&mut app, Duration::from_millis(300));
        assert!(!main.is_minimized().unwrap());
        assert!(main.is_focused().unwrap());
        assert!(!panel.is_visible().unwrap());
        assert_eq!(handle.state::<SessionStore>().snapshot(), recording);
    }

    // The non-minimized app-switch case, dismissal, and rearming still work.
    other.set_focus().unwrap();
    pump(&mut app, Duration::from_millis(200));
    assert!(panel.is_visible().unwrap());
    assert!(other.is_focused().unwrap());
    crate::recording_confirmation_window::hide_recorder(&handle).unwrap();
    pump(&mut app, Duration::from_millis(200));
    assert!(!panel.is_visible().unwrap());
    crate::residency::restore(&handle).unwrap();
    pump(&mut app, Duration::from_millis(200));
    other.set_focus().unwrap();
    pump(&mut app, Duration::from_millis(200));
    assert!(panel.is_visible().unwrap());

    // Native restore failure must release suppression, without hiding the
    // original error or permanently disabling subsequent panel display.
    assert_eq!(
        with_main_restore(&handle, || {
            assert!(!panel.is_visible().unwrap());
            main_focus_changed(&handle, true);
            main_focus_changed(&handle, false);
            Err("synthetic native failure".into())
        }),
        Err("synthetic native failure".into())
    );
    pump(&mut app, Duration::from_millis(200));
    assert!(panel.is_visible().unwrap());

    // A popup from main hides the panel; cancellation allows it again.
    let (decision, _) = handle
        .state::<StopDecisionStore>()
        .request(&recording, "main", "synthetic recording".into())
        .unwrap();
    synchronize(&handle).unwrap();
    assert!(!panel.is_visible().unwrap());
    handle
        .state::<StopDecisionStore>()
        .dismiss(decision.id.as_deref().unwrap())
        .unwrap();
    synchronize(&handle).unwrap();
    assert!(panel.is_visible().unwrap());
    let (_, _) = handle
        .state::<StopDecisionStore>()
        .request(&recording, "recorder", "synthetic recording".into())
        .unwrap();
    synchronize(&handle).unwrap();
    assert!(panel.is_visible().unwrap());
    crate::recording_confirmation_window::close_requested(&handle);
    pump(&mut app, Duration::from_millis(200));
    assert!(panel.is_visible().unwrap());
    assert!(
        panel.is_focused().unwrap(),
        "explicit cancellation cannot focus panel"
    );
    crate::residency::restore(&handle).unwrap();
    pump(&mut app, Duration::from_millis(200));
    assert!(
        !panel.is_visible().unwrap(),
        "Tauri cache prevented hiding the panel"
    );

    main_focus_changed(&handle, false);
    handle
        .state::<SessionStore>()
        .update(|s| s.phase = RecordingPhase::Stopping);
    pump(&mut app, Duration::from_millis(200));
    assert!(!panel.is_visible().unwrap());
    for window in windows.into_iter().rev() {
        window.destroy().unwrap();
    }
    pump(&mut app, Duration::from_millis(100));
    app.cleanup_before_exit();
}
