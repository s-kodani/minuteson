#[cfg(any(target_os = "macos", windows))]
use crate::bootstrap;
use crate::data_controller::commands::data_app;
use crate::generation::commands::minutes_app;
use crate::recording_confirmation_window;
use crate::storage::{self, Meeting, Storage};
use tauri::{Emitter, Manager, State, WebviewWindow};

#[tauri::command]
pub(crate) fn get_vocabulary_book(
    window: WebviewWindow,
    meeting_id: Option<String>,
) -> Result<storage::VocabularyBook, mon_contracts::AppError> {
    let result: Result<storage::VocabularyBook, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read vocabulary".into());
        }
        window
            .state::<Storage>()
            .vocabulary_book(meeting_id.as_deref())
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_vocabulary_book", message))
}

#[tauri::command]
pub(crate) fn get_meeting_vocabulary(
    window: WebviewWindow,
    meeting_id: String,
) -> Result<storage::VocabularyView, mon_contracts::AppError> {
    let result: Result<storage::VocabularyView, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can read vocabulary".into());
        }
        window.state::<Storage>().vocabulary_view(&meeting_id)
    })();
    result
        .map_err(|message| mon_contracts::AppError::from_command("get_meeting_vocabulary", message))
}

#[tauri::command]
pub(crate) fn save_vocabulary_book(
    window: WebviewWindow,
    meeting_id: Option<String>,
    expected_lock_version: i64,
    payload: Vec<storage::VocabularyEntry>,
) -> Result<storage::VocabularyBook, mon_contracts::AppError> {
    let result: Result<storage::VocabularyBook, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can edit vocabulary".into());
        }
        window.state::<Storage>().save_vocabulary_book(
            meeting_id.as_deref(),
            expected_lock_version,
            payload,
        )
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("save_vocabulary_book", message))
}

#[tauri::command]
pub(crate) fn get_ui_preferences(
    window: WebviewWindow,
) -> Result<storage::UiPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::UiPreferencesSnapshot, String> = (|| {
        if !matches!(
            window.label(),
            "main" | "recorder" | recording_confirmation_window::LABEL
        ) {
            return Err("Only application windows can read presentation settings".into());
        }
        window.state::<Storage>().ui_preferences()
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("get_ui_preferences", message))
}

#[tauri::command]
pub(crate) fn get_model_download_guidance(
    window: WebviewWindow,
) -> Result<storage::model_download_guidance::Guidance, mon_contracts::AppError> {
    let result = (|| {
        let app = minutes_app(&window)?;
        app.state::<Storage>().model_download_guidance()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_model_download_guidance", message)
    })
}

#[tauri::command]
pub(crate) fn dismiss_model_download_guidance(
    window: WebviewWindow,
) -> Result<storage::model_download_guidance::Guidance, mon_contracts::AppError> {
    let result = (|| {
        let app = minutes_app(&window)?;
        app.state::<Storage>().dismiss_model_download_guidance()
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("dismiss_model_download_guidance", message)
    })
}

#[tauri::command]
pub(crate) fn save_ui_preferences(
    window: WebviewWindow,
    expected_lock_version: i64,
    payload: storage::UiPreferences,
) -> Result<storage::UiPreferencesSnapshot, mon_contracts::AppError> {
    let result: Result<storage::UiPreferencesSnapshot, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can edit presentation settings".into());
        }
        let saved = window
            .state::<Storage>()
            .save_ui_preferences(expected_lock_version, payload)?;
        // A hint only: every window reloads Storage. A missed event is covered by
        // periodic reads; event failure must not hide an already committed setting.
        let _ = window
            .app_handle()
            .emit("ui-preferences-changed", saved.lock_version);
        Ok(saved)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("save_ui_preferences", message))
}

#[tauri::command]
pub(crate) async fn list_meetings(
    window: WebviewWindow,
) -> Result<Vec<Meeting>, mon_contracts::AppError> {
    let result: Result<Vec<Meeting>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can list meetings".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().list_meetings())
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("list_meetings", message))
}

#[tauri::command]
pub(crate) async fn get_meeting_list_diagnostics(
    window: WebviewWindow,
) -> Result<storage::startup_diagnostics::MeetingListDiagnostics, mon_contracts::AppError> {
    let result: Result<_, String> = async move {
        let app = data_app(&window)?.clone();
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<Storage>().meeting_list_diagnostics()
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| {
        mon_contracts::AppError::from_command("get_meeting_list_diagnostics", message)
    })
}

#[tauri::command]
pub(crate) fn get_storage_write_state(
    window: WebviewWindow,
) -> Result<storage::startup_diagnostics::StorageWriteState, mon_contracts::AppError> {
    data_app(&window)
        .map(|app| {
            let storage = app.state::<Storage>();
            let mut state = storage.storage_write_state();
            #[cfg(any(target_os = "macos", windows))]
            if state.salvage_available {
                state.salvage_available = app
                    .state::<bootstrap::Bootstrap>()
                    .require_ready_salvage(&storage)
                    .is_ok();
            }
            state
        })
        .map_err(|message| {
            mon_contracts::AppError::from_command("get_storage_write_state", message)
        })
}

#[tauri::command]
pub(crate) async fn get_meeting(
    window: WebviewWindow,
    id: String,
) -> Result<Option<Meeting>, mon_contracts::AppError> {
    let result: Result<Option<Meeting>, String> = async move {
        if window.label() != "main" {
            return Err("Only the main window can read meetings".into());
        }

        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || app.state::<Storage>().get_meeting(&id))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_meeting", message))
}

#[tauri::command]
pub(crate) fn rename_meeting(
    window: WebviewWindow,
    storage: State<'_, Storage>,
    id: String,
    title: String,
    expected_lock_version: i64,
) -> Result<Meeting, mon_contracts::AppError> {
    let result: Result<Meeting, String> = (|| {
        if window.label() != "main" {
            return Err("Only the main window can rename meetings".into());
        }
        storage.rename_meeting(&id, &title, expected_lock_version)
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("rename_meeting", message))
}
