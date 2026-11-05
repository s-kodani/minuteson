//! Adjust the merged configuration before creating any native windows. QA's
//! windows array replaces the base array, so a platform JSON override is not enough.
pub(crate) fn configure(config: &mut tauri::Config) {
    if let Some(main) = config
        .app
        .windows
        .iter_mut()
        .find(|window| window.label == "main")
    {
        main.decorations = false;
    }
    for window in &mut config.app.windows {
        if matches!(window.label.as_str(), "recorder" | "recording-confirmation") {
            // Tao's undecorated shadows add a non-client frame and invisible
            // resize insets. Floating cards already paint their own borders.
            window.decorations = false;
            window.shadow = false;
            window.transparent = true;
            window.background_color = Some(tauri::utils::config::Color(0, 0, 0, 0));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn normal_and_qa_remove_native_frames_from_floating_cards() {
        for source in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.qa.conf.json"),
        ] {
            let mut config: tauri::Config = serde_json::from_str(source).unwrap();
            let original = config.app.windows.clone();
            super::configure(&mut config);
            for (window, mut expected) in config.app.windows.iter().zip(original) {
                if expected.label == "main" {
                    expected.decorations = false;
                }
                if matches!(
                    expected.label.as_str(),
                    "recorder" | "recording-confirmation"
                ) {
                    expected.decorations = false;
                    expected.shadow = false;
                    expected.transparent = true;
                    expected.background_color = Some(tauri::utils::config::Color(0, 0, 0, 0));
                }
                assert_eq!(
                    serde_json::to_value(window).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
        }
    }
}
