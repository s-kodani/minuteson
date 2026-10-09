//! Read-only explanation beside the small recorder WebView. Never stops capture.
use crate::recording_confirmation::StopDecisionStore;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

pub const LABEL: &str = "recorder-info";

fn may_open(origin: &str, recorder_visible: bool, confirming: bool) -> bool {
    origin == "recorder" && recorder_visible && !confirming
}

fn may_restore(explicit: bool, recorder_visible: bool, confirming: bool) -> bool {
    explicit && recorder_visible && !confirming
}

// Physical coordinates: adjacent below, or above when the lower edge is full.
// Clamp both axes to the current monitor's work area (including negative origins).
fn placement(
    origin: (i32, i32),
    area: (u32, u32),
    panel: (i32, i32, u32, u32),
    scale: f64,
) -> Result<(i32, i32, u32, u32), String> {
    if area.0 == 0 || area.1 == 0 || !scale.is_finite() || scale <= 0.0 {
        return Err("録音パネルの表示領域を確認できません".into());
    }
    let width = ((360.0 * scale).round() as u32).max(1).min(area.0);
    let height = ((340.0 * scale).round() as u32).max(1).min(area.1);
    // Malformed but finite monitor scales must not overflow coordinate sums.
    let gap = (8.0 * scale).round().clamp(1.0, 128.0) as i64;
    let left = i64::from(origin.0);
    let top = i64::from(origin.1);
    let right = left + i64::from(area.0);
    let bottom = top + i64::from(area.1);
    let below = i64::from(panel.1) + i64::from(panel.3) + gap;
    let y = if below + i64::from(height) <= bottom {
        below
    } else {
        i64::from(panel.1) - gap - i64::from(height)
    };
    let x = i64::from(panel.0).clamp(left, right - i64::from(width));
    let y = y.clamp(top, bottom - i64::from(height));
    Ok((
        x.try_into().map_err(|_| "表示位置が範囲外です")?,
        y.try_into().map_err(|_| "表示位置が範囲外です")?,
        width,
        height,
    ))
}

pub fn hide(app: &AppHandle, explicit: bool) -> Result<(), String> {
    let popup = app
        .get_webview_window(LABEL)
        .ok_or("録音の説明画面がありません")?;
    let was_visible = popup.is_visible().map_err(|e| e.to_string())?;
    popup.hide().map_err(|e| e.to_string())?;
    let recorder = app.get_webview_window("recorder");
    let visible = recorder
        .as_ref()
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let confirming = app.state::<StopDecisionStore>().snapshot().visible();
    let restore = was_visible && may_restore(explicit, visible, confirming);
    if restore {
        recorder
            .as_ref()
            .unwrap()
            .set_focus()
            .map_err(|e| e.to_string())?;
    }
    if let Some(recorder) = recorder {
        let _ = recorder.emit("recorder-info-closed", restore);
    }
    Ok(())
}

#[tauri::command]
pub fn get_recorder_info(window: WebviewWindow) -> Result<bool, String> {
    if window.label() != "recorder" {
        return Err("録音パネルから説明の状態を確認してください".into());
    }
    window
        .app_handle()
        .get_webview_window(LABEL)
        .ok_or("録音の説明画面がありません")?
        .is_visible()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn show_recorder_info(window: WebviewWindow) -> Result<bool, String> {
    let app = window.app_handle();
    if !may_open(
        window.label(),
        window.is_visible().unwrap_or(false),
        app.state::<StopDecisionStore>().snapshot().visible(),
    ) {
        return Err("録音パネルから説明を開いてください。停止確認中は説明を開けません".into());
    }
    let popup = app
        .get_webview_window(LABEL)
        .ok_or("録音の説明画面がありません")?;
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("録音パネルのモニターを確認できません")?;
    let area = monitor.work_area();
    let panel = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let (x, y, width, height) = placement(
        (area.position.x, area.position.y),
        (area.size.width, area.size.height),
        (panel.x, panel.y, size.width, size.height),
        monitor.scale_factor(),
    )?;
    popup
        .set_size(PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    popup
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    popup.show().map_err(|e| e.to_string())?;
    if let Err(error) = popup.set_focus() {
        let _ = hide(app, false);
        return Err(format!("録音の説明画面に移れません: {error}"));
    }
    Ok(true)
}

#[tauri::command]
pub fn close_recorder_info(window: WebviewWindow) -> Result<bool, String> {
    if !matches!(window.label(), "recorder" | LABEL) {
        return Err("録音の説明画面から閉じてください".into());
    }
    hide(window.app_handle(), true)?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_visible_recorder_can_open_without_a_stop_confirmation() {
        assert!(may_open("recorder", true, false));
        for label in ["main", LABEL, "recording-confirmation", "unknown"] {
            assert!(!may_open(label, true, false));
        }
        assert!(!may_open("recorder", false, false));
        assert!(!may_open("recorder", true, true));
    }

    #[test]
    fn only_explicit_close_restores_focus_never_over_confirmation() {
        assert!(may_restore(true, true, false));
        assert!(!may_restore(false, true, false));
        assert!(!may_restore(true, false, false));
        assert!(!may_restore(true, true, true));
    }

    #[test]
    fn placement_uses_both_edges_negative_monitors_and_scaled_work_areas() {
        assert_eq!(
            placement((0, 0), (1920, 1040), (1536, 24, 360, 116), 1.0).unwrap(),
            (1536, 148, 360, 340)
        );
        assert_eq!(
            placement((-1920, -1080), (1920, 1040), (-360, -200, 360, 116), 1.0).unwrap(),
            (-360, -548, 360, 340)
        );
        assert_eq!(
            placement((0, 0), (1000, 800), (900, 100, 720, 232), 2.0).unwrap(),
            (280, 0, 720, 680)
        );
        assert_eq!(
            placement((10, 20), (200, 100), (999, 999, 360, 116), 2.0).unwrap(),
            (10, 20, 200, 100)
        );
        assert!(placement((0, 0), (0, 100), (0, 0, 360, 116), 1.0).is_err());
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(placement((0, 0), (1000, 800), (0, 0, 360, 116), scale).is_err());
        }
        assert_eq!(
            placement((0, 0), (1000, 800), (0, 0, 360, 116), 1e300).unwrap(),
            (0, 0, 1000, 800)
        );
    }
}
