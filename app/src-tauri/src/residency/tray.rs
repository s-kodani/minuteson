use super::*;
use std::cell::RefCell;
use tray_icon::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
thread_local! { static TRAY: RefCell<Option<(TrayIcon,MenuItem)>> = const { RefCell::new(None) }; }
pub(super) fn present() -> bool {
    TRAY.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|(icon, _)| icon.rect().is_some())
    })
}
pub(super) fn create(app: &tauri::AppHandle) -> Result<(), String> {
    // Remove the old GUID before adding it again; dropping an old icon after
    // registration would delete the newly registered Shell icon as well.
    TRAY.with(|slot| {
        slot.borrow_mut().take();
    });
    let menu = Menu::new();
    let status = MenuItem::with_id("mon-resident-status", "待機中", false, None);
    let open = MenuItem::with_id("mon-resident-open", "メインを開く", true, None);
    let quit = MenuItem::with_id("mon-resident-quit", "終了", true, None);
    menu.append_items(&[&status, &PredefinedMenuItem::separator(), &open, &quit])
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let image = tauri::include_image!("icons/tray-template.png");
    #[cfg(windows)]
    let image = app
        .default_window_icon()
        .ok_or("常駐アイコンがありません")?;
    let icon = Icon::from_rgba(image.rgba().to_vec(), image.width(), image.height())
        .map_err(|e| e.to_string())?;
    let guid = {
        let state = app.state::<Residency>();
        let mut value = state.guid.lock().unwrap_or_else(|e| e.into_inner());
        value
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone()
    };
    let name = app.config().product_name.as_deref().unwrap_or("MON");
    let builder = TrayIconBuilder::new()
        .with_id("mon-resident")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_tooltip(format!("{name} — 待機中"))
        .with_menu_on_left_click(false);
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_as_template(true);
    #[cfg(windows)]
    let builder = builder.with_guid(
        uuid::Uuid::parse_str(&guid)
            .map_err(|e| e.to_string())?
            .as_u128(),
    );
    let icon = builder
        .build()
        .map_err(|e| format!("常駐アイコンを登録できません: {e}"))?;
    TRAY.with(|slot| *slot.borrow_mut() = Some((icon, status)));
    if !app
        .state::<Residency>()
        .menu_installed
        .swap(true, Ordering::SeqCst)
    {
        app.on_menu_event(|app, event| match event.id().as_ref() {
            "mon-resident-open" => {
                let _ = restore(app);
            }
            "mon-resident-quit" => super::quit(app),
            _ => {}
        });
    }
    let handle = app.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            let app = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let _ = restore(&app);
            });
        }
    }));
    Ok(())
}
pub(super) fn status(app: &tauri::AppHandle) -> String {
    let session = app.state::<crate::session::SessionStore>().snapshot();
    if app.state::<crate::startup_attention::Startup>().blocked() {
        "要確認・起動の復旧".into()
    } else {
        match session.phase {
            crate::session::RecordingPhase::Error => "要確認・録音エラー".into(),
            crate::session::RecordingPhase::Saving => "録音を保存中".into(),
            crate::session::RecordingPhase::Stopping => "録音停止中".into(),
            crate::session::RecordingPhase::Preparing => "録音準備中".into(),
            crate::session::RecordingPhase::Recording => format!(
                "録音中 {:02}:{:02}:{:02}",
                session.elapsed_ms / 3_600_000,
                session.elapsed_ms / 60_000 % 60,
                session.elapsed_ms / 1000 % 60
            ),
            _ => {
                let (analysis, model) = app
                    .state::<crate::analysis_worker::AnalysisController>()
                    .close_activity()
                    .unwrap_or((false, false));
                let import = app
                    .state::<crate::import_controller::ImportController>()
                    .snapshot()
                    .is_ok_and(|s| s.busy);
                let data = app
                    .state::<crate::data_controller::DataController>()
                    .snapshot()
                    .is_ok_and(|s| s.busy);
                let test = app
                    .state::<crate::capture_test::CaptureTests>()
                    .snapshot(None)
                    .is_ok_and(|s| s.is_some_and(|s| s.is_busy()));
                if app.state::<crate::recording_recovery::Recovery>().busy() {
                    "録音を回復中".into()
                } else if analysis || model || import || data || test {
                    "処理中".into()
                } else {
                    "待機中".into()
                }
            }
        }
    }
}
pub(super) fn refresh(app: &tauri::AppHandle) {
    // Stable native GUID also permits an independent Shell existence check.
    #[cfg(windows)]
    let missing = !present();
    #[cfg(target_os = "macos")]
    let missing = TRAY.with(|slot| slot.borrow().is_none());
    if missing {
        let _ = create(app);
    }
    let status = app
        .state::<Residency>()
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let name = app.config().product_name.as_deref().unwrap_or("MON");
    TRAY.with(|slot| {
        if let Some((icon, item)) = slot.borrow().as_ref() {
            item.set_text(&status);
            let tooltip = format!("{name} — {status}");
            #[cfg(windows)]
            if let Some(guid) = app
                .state::<Residency>()
                .guid
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_deref()
            {
                let _ = windows::set_tray_tooltip(icon, guid, &tooltip);
            }
            #[cfg(target_os = "macos")]
            let _ = icon.set_tooltip(Some(tooltip));
        }
    });
}
pub(super) fn remove() {
    TRAY.with(|slot| {
        slot.borrow_mut().take();
    });
}
