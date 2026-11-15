use super::{complete, policy, publish, Updates};
use mon_contracts::AppUpdateState as Phase;
use std::{path::PathBuf, time::Duration};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

pub(super) struct Pending {
    update: Update,
    bytes: Vec<u8>,
}

pub(super) async fn check_and_download(app: &AppHandle) {
    let result: Result<(), String> = async {
        if std::env::consts::ARCH != "aarch64" {
            return Err("このMacのCPUは自動更新の対象外です".into());
        }
        let updater = app
            .updater_builder()
            .timeout(policy::DEADLINE)
            .build()
            .map_err(|e| e.to_string())?;
        let Some(mut update) = updater.check().await.map_err(|e| e.to_string())? else {
            complete(app, Ok((false, None)));
            return Ok(());
        };
        policy::validate_download(
            &update.version,
            update.download_url.as_str(),
            std::env::consts::ARCH,
        )?;
        complete(app, Ok((true, Some(update.version.clone()))));
        transfer(app, Phase::Downloading, 0, None, None);
        // Downloads may exceed the short metadata deadline. Never install in this task.
        update.timeout = Some(Duration::from_secs(30 * 60));
        let mut downloaded = 0u64;
        let mut last_publish = std::time::Instant::now();
        let bytes = update
            .download(
                |chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    if last_publish.elapsed() >= Duration::from_millis(250) {
                        transfer(app, Phase::Downloading, downloaded, total, None);
                        last_publish = std::time::Instant::now();
                    }
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())?;
        if app
            .state::<Updates>()
            .stopped
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(());
        }
        let size = bytes.len() as u64;
        *app.state::<Updates>()
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Pending { update, bytes });
        transfer(app, Phase::Ready, size, Some(size), None);
        Ok(())
    }
    .await;
    if let Err(error) = result {
        crate::internal_logs::report_error("app_update", &error);
        transfer(
            app,
            Phase::Failed,
            0,
            None,
            Some("更新を取得できませんでした。接続を確認して再試行してください。".into()),
        );
    }
}

fn transfer(app: &AppHandle, phase: Phase, bytes: u64, total: Option<u64>, error: Option<String>) {
    let state = app.state::<Updates>();
    if state.stopped.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    state
        .model
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .transfer(phase, bytes, total, error);
    publish(app);
}

pub(crate) fn require_ready(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<Updates>();
    if state.snapshot().state != Phase::Ready
        || state
            .pending
            .lock()
            .map_err(|_| "更新状態を取得できません")?
            .is_none()
    {
        return Err("更新のダウンロードが完了してから再起動してください".into());
    }
    installation_path()?;
    Ok(())
}

fn installation_path() -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let app = executable
        .ancestors()
        .nth(3)
        .ok_or("アプリの配置先を確認できません")?;
    // Exclude mounted DMGs, translocated apps and arbitrary temporary copies.
    let user_apps = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Applications"));
    if app.parent() != Some(std::path::Path::new("/Applications"))
        && app.parent() != user_apps.as_deref()
    {
        return Err("アプリをApplicationsフォルダーに移動してから更新してください".into());
    }
    let meta = std::fs::symlink_metadata(app).map_err(|e| e.to_string())?;
    let parent = app.parent().ok_or("配置先を確認できません")?;
    let writable = |path: &std::path::Path| {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::CString::new(path.as_os_str().as_bytes())
            .is_ok_and(|p| unsafe { libc::access(p.as_ptr(), libc::W_OK) == 0 })
    };
    if meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || !writable(app)
        || !writable(parent)
    {
        return Err("アプリの更新に必要な書き込み権限がありません。書き込み可能なApplicationsフォルダーに配置してください".into());
    }
    Ok(app.to_path_buf())
}

pub(crate) fn require_idle(app: &AppHandle) -> Result<(), String> {
    let (analysis, model) = app.state::<crate::AnalysisController>().close_activity()?;
    if app.state::<crate::SessionStore>().snapshot().is_active()
        || app.state::<crate::ImportController>().snapshot()?.busy
        || app.state::<crate::DataController>().snapshot()?.busy
        || app.state::<crate::exports::ExportController>().active()
        || app.state::<crate::recording_recovery::Recovery>().busy()
        || analysis
        || model
    {
        return Err("録音・解析・保存などの処理が完了してから更新してください".into());
    }
    app.state::<crate::capture_test::CaptureTests>()
        .require_exit_ready()
}

/// Called only after the normal close barrier has sealed admission and stopped workers.
pub(crate) fn install(app: &AppHandle) -> Result<(), String> {
    installation_path()?;
    let state = app.state::<Updates>();
    let pending = state
        .pending
        .lock()
        .map_err(|_| "更新状態を取得できません")?
        .take()
        .ok_or("取得済みの更新がありません")?;
    transfer(
        app,
        Phase::Installing,
        pending.bytes.len() as u64,
        Some(pending.bytes.len() as u64),
        None,
    );
    if let Err(error) = pending.update.install(&pending.bytes) {
        crate::internal_logs::report_error("app_update", &error.to_string());
        *state.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(pending);
        let status = state.snapshot();
        transfer(
            app,
            Phase::Ready,
            status.downloaded_bytes,
            status.total_bytes,
            Some(
                "更新を適用できませんでした。配置先と書き込み権限を確認して再試行してください。"
                    .into(),
            ),
        );
        return Err("更新を適用できませんでした。アプリを終了せずに保持しています".into());
    }
    Ok(())
}
