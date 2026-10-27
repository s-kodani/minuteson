//! Local-only read failures and recovery. Never affects capture or durability.
use crate::internal_logs::{self, Context};
use tauri::WebviewWindow;

pub(crate) fn read_result(
    stage: &str,
    owner: Option<&str>,
    meeting: Option<&str>,
    error: Option<&str>,
) {
    internal_logs::transition(
        Context {
            category: "recording_diagnostics".into(),
            meeting_id: meeting.map(str::to_owned),
            operation_id: Some(format!("{stage}:{}", owner.unwrap_or("all"))),
            ..Context::default()
        },
        if error.is_some() {
            "failed"
        } else {
            "succeeded"
        },
        if error.is_some() { "warn" } else { "info" },
        &format!(
            "stage={stage} recording={} {}",
            owner.unwrap_or("all"),
            error.unwrap_or("read confirmed")
        ),
    );
}

fn validate(
    label: &str,
    stage: &str,
    owner: Option<&str>,
    message: Option<&str>,
) -> Result<(), String> {
    if !matches!(label, "main" | "recorder")
        || !matches!(
            stage,
            "live-journal"
                | "live-journal-notice"
                | "archived-page"
                | "archived-rows"
                | "archived-validation"
                | "archived-window"
                | "archived-subscribe"
        )
        || owner.is_some_and(|id| !crate::input_levels::valid_owner(id))
        || message.is_some_and(|text| text.len() > 16 * 1024)
    {
        return Err("録音の確認ログの対象または内容が不正です".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn record_input_diagnostic_log(
    window: WebviewWindow,
    stage: String,
    session_id: Option<String>,
    message: Option<String>,
) -> Result<(), String> {
    validate(
        window.label(),
        &stage,
        session_id.as_deref(),
        message.as_deref(),
    )?;
    read_result(
        &format!("ui-{stage}"),
        session_id.as_deref(),
        None,
        message.as_deref(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_logging_only_accepts_recording_windows_and_bounded_known_stages() {
        let owner = "73500000-0000-4000-8000-000000000001";
        assert!(validate(
            "main",
            "live-journal",
            Some(owner),
            Some("Access denied (os error 5)")
        )
        .is_ok());
        assert!(validate("recorder", "live-journal", Some(owner), None).is_ok());
        for (label, stage, id) in [
            ("recorder-info", "live-journal", Some(owner)),
            ("main", "arbitrary", None),
            ("main", "live-journal", Some("bad")),
        ] {
            assert!(validate(label, stage, id, None).is_err());
        }
        assert!(validate(
            "main",
            "archived-page",
            None,
            Some(&"x".repeat(16 * 1024 + 1))
        )
        .is_err());
    }
}
