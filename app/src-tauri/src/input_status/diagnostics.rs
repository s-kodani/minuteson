//! Local-only read failures and recovery. Never affects capture or durability.
use crate::internal_logs::{self, Context};
use tauri::WebviewWindow;

pub(super) fn input_transitions(
    current: &super::Publication,
    previous: Option<&super::Publication>,
) {
    for (name, history, prior) in [
        (
            "microphone",
            current.microphone_history.as_ref(),
            previous.and_then(|p| p.microphone_history.as_ref()),
        ),
        (
            "system",
            current.system_history.as_ref(),
            previous.and_then(|p| p.system_history.as_ref()),
        ),
    ] {
        let Some(history) = history else { continue };
        let through = prior.map_or(0, |h| h.last_sequence);
        for event in history
            .events
            .iter()
            .filter(|event| event.sequence > through)
        {
            internal_logs::detail(
                Context {
                    category: "recording_inputs".into(),
                    operation_id: Some(current.owner_id.clone()),
                    ..Context::default()
                },
                &event.kind,
                "info",
                std::time::Duration::ZERO,
                &format!(
                    "input={name} sequence={} epoch={} reason={}",
                    event.sequence,
                    event.status.source_epoch,
                    event.status.reason.as_deref().unwrap_or("none")
                ),
            );
        }
    }
}

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
            "live-status"
                | "live-journal"
                | "live-journal-notice"
                | "live-levels"
                | "test-levels"
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

fn ui_stage(stage: &str, window_label: &str) -> String {
    match stage {
        "live-levels" | "test-levels" => format!("ui-{stage}-{window_label}"),
        _ => format!("ui-{stage}"),
    }
}

#[tauri::command]
pub(crate) fn record_input_diagnostic_log(
    window: WebviewWindow,
    stage: String,
    session_id: Option<String>,
    message: Option<String>,
) -> Result<(), mon_contracts::AppError> {
    let result: Result<(), String> = (|| {
        validate(
            window.label(),
            &stage,
            session_id.as_deref(),
            message.as_deref(),
        )?;
        read_result(
            &ui_stage(&stage, window.label()),
            session_id.as_deref(),
            None,
            message.as_deref(),
        );
        Ok(())
    })();
    result.map_err(|message| {
        mon_contracts::AppError::from_command("record_input_diagnostic_log", message)
    })
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
        assert!(validate("main", "live-levels", Some(owner), None).is_ok());
        assert!(validate("recorder", "test-levels", Some(owner), None).is_ok());
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

    #[test]
    fn level_stages_include_window_label_for_dedup() {
        let owner = "73500000-0000-4000-8000-000000000001";
        let main = ui_stage("live-levels", "main");
        let recorder = ui_stage("live-levels", "recorder");
        assert_eq!(main, "ui-live-levels-main");
        assert_eq!(recorder, "ui-live-levels-recorder");
        assert_ne!(format!("{main}:{owner}"), format!("{recorder}:{owner}"));
        assert_eq!(ui_stage("live-status", "main"), "ui-live-status");
    }
}
