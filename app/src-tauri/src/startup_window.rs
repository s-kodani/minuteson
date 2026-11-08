//! Fit the main window once, before its first display. Restoring a resident
//! instance must preserve the user's subsequent size and position.
use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

pub(crate) fn initialize(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        crate::internal_logs::report_error("startup_window", "メイン画面がありません");
        return;
    };
    if let Err(error) = fit(&window) {
        crate::internal_logs::report_error("startup_window", &error);
    }
    // Even missing display information or a native resize failure must not
    // leave the application hidden, including on startup diagnostic paths.
    if let Err(error) = window.show() {
        crate::internal_logs::report_error("startup_window", &error.to_string());
    }
}

fn fit(window: &WebviewWindow) -> Result<(), String> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("起動画面のモニターを確認できません")?;
    let config = window
        .app_handle()
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == "main")
        .ok_or("メイン画面の設定がありません")?;
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let outer = window.outer_size().map_err(|e| e.to_string())?;
    let window_scale = window.scale_factor().map_err(|e| e.to_string())?;
    let frame = PhysicalSize::new(
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    )
    .to_logical::<f64>(window_scale);
    let area = monitor.work_area();
    let layout = fitted_layout(
        area.size,
        monitor.scale_factor(),
        frame,
        LogicalSize::new(config.width, config.height),
        LogicalSize::new(
            config.min_width.unwrap_or(0.0),
            config.min_height.unwrap_or(0.0),
        ),
    )?;
    // Lower constraints first, otherwise the OS clamps small-screen sizes.
    window
        .set_min_size(Some(layout.minimum))
        .and_then(|_| window.set_size(layout.size))
        .map_err(|e| e.to_string())?;
    // Query the real outer size after resizing instead of assuming all OSes
    // have the same titlebar/frame or logical-to-physical rounding.
    let outer = window.outer_size().map_err(|e| e.to_string())?;
    window
        .set_position(centered_position(area.position, area.size, outer))
        .map_err(|e| e.to_string())
}

#[derive(Debug, PartialEq)]
struct Layout {
    size: LogicalSize<f64>,
    minimum: LogicalSize<f64>,
}

fn fitted_layout(
    work_area: PhysicalSize<u32>,
    scale: f64,
    frame: LogicalSize<f64>,
    preferred: LogicalSize<f64>,
    minimum: LogicalSize<f64>,
) -> Result<Layout, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err("画面の表示倍率を確認できません".into());
    }
    let dimension = |available: u32, frame: f64, preferred: f64| {
        // Integer arithmetic keeps the physical budget strictly within 90%.
        let budget = (u64::from(available) * 9 / 10) as f64 / scale;
        let size = (budget - frame).floor().min(preferred);
        if size.is_finite() && size >= 1.0 {
            Ok(size)
        } else {
            Err("起動画面の作業領域を確認できません".to_string())
        }
    };
    let size = LogicalSize::new(
        dimension(work_area.width, frame.width, preferred.width)?,
        dimension(work_area.height, frame.height, preferred.height)?,
    );
    Ok(Layout {
        minimum: LogicalSize::new(
            minimum.width.min(size.width),
            minimum.height.min(size.height),
        ),
        size,
    })
}

fn centered_position(
    origin: PhysicalPosition<i32>,
    work_area: PhysicalSize<u32>,
    outer: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let coordinate = |origin: i32, available: u32, size: u32| {
        (i64::from(origin) + i64::from(available.saturating_sub(size)) / 2)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    PhysicalPosition::new(
        coordinate(origin.x, work_area.width, outer.width),
        coordinate(origin.y, work_area.height, outer.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(width: u32, height: u32, scale: f64, frame: LogicalSize<f64>) -> Layout {
        fitted_layout(
            PhysicalSize::new(width, height),
            scale,
            frame,
            LogicalSize::new(1280.0, 820.0),
            LogicalSize::new(960.0, 640.0),
        )
        .unwrap()
    }

    #[test]
    fn large_displays_keep_the_preferred_size_and_normal_minimum() {
        let layout = layout(1920, 1080, 1.0, LogicalSize::new(0.0, 28.0));
        assert_eq!(layout.size, LogicalSize::new(1280.0, 820.0));
        assert_eq!(layout.minimum, LogicalSize::new(960.0, 640.0));
    }

    #[test]
    fn small_displays_reserve_the_native_frame_and_relax_only_needed_minimums() {
        let small = layout(1024, 768, 1.0, LogicalSize::new(16.0, 39.0));
        assert_eq!(small.size, LogicalSize::new(905.0, 652.0));
        assert_eq!(small.minimum, LogicalSize::new(905.0, 640.0));
        let tiny = layout(800, 600, 1.0, LogicalSize::new(0.0, 28.0));
        assert_eq!(tiny.size, LogicalSize::new(720.0, 512.0));
        assert_eq!(tiny.minimum, tiny.size);
    }

    #[test]
    fn dpi_changes_physical_pixels_without_changing_the_logical_layout() {
        for scale in [1.0, 1.5, 2.0] {
            let layout = layout(
                (1280.0 * scale) as u32,
                (800.0 * scale) as u32,
                scale,
                LogicalSize::new(0.0, 28.0),
            );
            assert_eq!(layout.size, LogicalSize::new(1152.0, 692.0));
            assert_eq!(layout.minimum, LogicalSize::new(960.0, 640.0));
        }
    }

    #[test]
    fn fractional_dimensions_round_down_inside_the_physical_budget() {
        let layout = layout(1366, 768, 1.5, LogicalSize::new(10.0, 26.0));
        assert_eq!(layout.size, LogicalSize::new(809.0, 434.0));
        assert_eq!(layout.minimum, layout.size);
    }

    #[test]
    fn invalid_display_information_does_not_produce_a_resize() {
        for (size, scale) in [
            (PhysicalSize::new(0, 768), 1.0),
            (PhysicalSize::new(1024, 0), 1.0),
            (PhysicalSize::new(1024, 768), 0.0),
            (PhysicalSize::new(1024, 768), -1.0),
            (PhysicalSize::new(1024, 768), f64::NAN),
            (PhysicalSize::new(1024, 768), f64::INFINITY),
        ] {
            assert!(fitted_layout(
                size,
                scale,
                LogicalSize::new(0.0, 28.0),
                LogicalSize::new(1280.0, 820.0),
                LogicalSize::new(960.0, 640.0),
            )
            .is_err());
        }
    }

    #[test]
    fn centering_uses_the_work_area_origin_including_negative_monitors() {
        for origin in [
            PhysicalPosition::new(0, 24),
            PhysicalPosition::new(-1920, -1080),
        ] {
            assert_eq!(
                centered_position(
                    origin,
                    PhysicalSize::new(1920, 1040),
                    PhysicalSize::new(1280, 848)
                ),
                PhysicalPosition::new(origin.x + 320, origin.y + 96),
            );
        }
    }

    #[test]
    fn normal_and_qa_main_windows_wait_for_sizing_before_display() {
        for source in [
            include_str!("../tauri.conf.json"),
            include_str!("../tauri.qa.conf.json"),
        ] {
            let config: tauri::Config = serde_json::from_str(source).unwrap();
            let main = config
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .unwrap();
            assert!(!main.visible);
            assert!(main.resizable);
            assert_eq!((main.width, main.height), (1280.0, 820.0));
            assert_eq!(
                (main.min_width, main.min_height),
                (Some(960.0), Some(640.0))
            );
        }
    }
}
