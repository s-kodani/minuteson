//! Rereads startup recovery review against the live database without reopening storage.
#![cfg(any(target_os = "macos", windows))]
use serde::Serialize;
use tauri::{Manager, WebviewWindow};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecheckRecoveryReviewResult {
    updated: bool,
    restart_required: bool,
    review_required: bool,
    reasons: Vec<String>,
}

fn require_ready_main(window: &WebviewWindow, command: &str) -> Result<(), String> {
    window
        .state::<crate::startup_attention::Startup>()
        .admit_command(window.label(), command)
}

#[tauri::command]
pub(crate) fn recheck_startup_recovery(
    window: WebviewWindow,
) -> Result<RecheckRecoveryReviewResult, mon_contracts::AppError> {
    let result: Result<RecheckRecoveryReviewResult, String> = (|| {
        require_ready_main(&window, "recheck_startup_recovery")?;
        let app = window.app_handle();
        let bootstrap = app.state::<crate::bootstrap::Bootstrap>();
        let updated = bootstrap.recheck_recovery_review()?;
        let view = bootstrap
            .recovery_view()
            .map_err(|error| error.to_string())?;
        let (review_required, reasons) = view
            .as_ref()
            .map(|recovery| (recovery.review_required, recovery.reasons.clone()))
            .unwrap_or((false, vec![]));
        Ok(RecheckRecoveryReviewResult {
            updated,
            restart_required: updated,
            review_required,
            reasons,
        })
    })();
    result.map_err(|message| mon_contracts::AppError::from_command("recheck_startup_recovery", message))
}

#[tauri::command]
pub(crate) fn restart_after_recovery_review(
    window: WebviewWindow,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        require_ready_main(&window, "restart_after_recovery_review")?;
        crate::startup_instance::prepare_restart()?;
        crate::startup_instance::restart(window.app_handle());
        Ok(())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("restart_after_recovery_review", message)
    })
}