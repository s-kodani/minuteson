//! Exercise Tauri's resolved build output, not a second hand-written ACL model.
#[cfg(test)]
mod tests {
    use tauri::ipc::{Origin, RuntimeAuthority};
    use tauri::utils::{acl::resolved::Resolved, platform::Target};

    fn authority() -> RuntimeAuthority {
        let manifests = serde_json::from_slice(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/acl-manifests.json"
        )))
        .unwrap();
        let capabilities = serde_json::from_slice(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/capabilities.json"
        )))
        .unwrap();
        let resolved = Resolved::resolve(&manifests, capabilities, Target::current()).unwrap();
        assert!(resolved.has_app_acl);
        RuntimeAuthority::new(manifests, resolved)
    }

    #[test]
    fn auxiliary_windows_are_denied_data_writes_by_the_actual_tauri_authority() {
        let acl = authority();
        for command in [
            "start_recording",
            "start_audio_import",
            "start_model_download",
            "start_data_operation",
            "save_transcript_draft",
            "save_minutes_markdown",
            "save_ui_preferences",
            "get_internal_logs",
            "get_recording_recovery",
            "get_startup_instance",
            "get_residency_state",
            "hide_main_to_background",
            "acknowledge_startup_activation",
            "open_existing_instance",
            "recover_startup_instance",
            "retry_startup",
            "get_meeting_deletion_availability",
        ] {
            assert!(
                acl.resolve_access(command, "main", "main", &Origin::Local)
                    .is_some(),
                "{command}"
            );
            for label in [
                "recorder",
                "recorder-info",
                "recording-confirmation",
                "unknown",
            ] {
                assert!(
                    acl.resolve_access(command, label, label, &Origin::Local)
                        .is_none(),
                    "{label}: {command}"
                );
            }
        }
    }

    #[test]
    fn custom_titlebar_controls_keep_existing_panel_and_origin_boundaries() {
        let acl = authority();
        let remote = Origin::Remote {
            url: "https://example.invalid/".parse().unwrap(),
        };
        for command in [
            "plugin:window|minimize",
            "plugin:window|toggle_maximize",
            "plugin:window|start_dragging",
            "plugin:window|close",
        ] {
            assert!(
                acl.resolve_access(command, "main", "main", &Origin::Local)
                    .is_some(),
                "{command}"
            );
            assert!(
                acl.resolve_access(command, "main", "main", &remote)
                    .is_none(),
                "{command}"
            );
            for label in [
                "recorder",
                "recorder-info",
                "recording-confirmation",
                "unknown",
            ] {
                let allowed = label == "recorder" && command == "plugin:window|start_dragging";
                assert_eq!(
                    acl.resolve_access(command, label, label, &Origin::Local)
                        .is_some(),
                    allowed,
                    "{label}: {command}"
                );
            }
        }
    }

    #[test]
    fn panel_actions_and_remote_origins_keep_separate_permissions() {
        let acl = authority();
        for (label, command) in [
            ("recorder", "request_recording_confirmation"),
            ("recorder", "get_recording_input_status"),
            ("recorder", "open_main_window"),
            ("recorder-info", "close_recorder_info"),
            ("recording-confirmation", "decide_recording_confirmation"),
        ] {
            assert!(
                acl.resolve_access(command, label, label, &Origin::Local)
                    .is_some(),
                "{label}: {command}"
            );
        }
        let remote = Origin::Remote {
            url: "https://example.invalid/".parse().unwrap(),
        };
        for label in [
            "main",
            "recorder",
            "recorder-info",
            "recording-confirmation",
        ] {
            assert!(acl
                .resolve_access("get_ui_preferences", label, label, &Origin::Local)
                .is_some());
            assert!(acl
                .resolve_access("get_ui_preferences", label, label, &remote)
                .is_none());
            assert!(acl
                .resolve_access("plugin:shell|execute", label, label, &Origin::Local)
                .is_none());
            assert!(acl
                .resolve_access("plugin:fs|write_file", label, label, &Origin::Local)
                .is_none());
        }
        assert!(acl
            .resolve_access(
                "decide_recording_confirmation",
                "recorder-info",
                "recorder-info",
                &Origin::Local
            )
            .is_none());
    }
}
