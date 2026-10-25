//! Startup-only, off-UI-thread cleanup using the same exclusive admission as
//! foreground deletion. Required ownership/consent journals remain internal.
use super::*;
use work::Admission;

pub(super) fn run(app: &AppHandle) -> Result<(), String> {
    // A foreground operation may have begun while the startup worker waited.
    // Its selection/replay capability must not be replaced or disposed here.
    let state = app.state::<DataController>();
    let Some((snapshot, cancel)) = reserve(&state)? else {
        return Ok(());
    };
    DataController::emit(app, snapshot.clone());
    // No data-controller mutex remains held during filesystem work or events.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_items(app, &|| cancel.load(Ordering::Acquire))
    }))
    .unwrap_or_else(|_| Err("自動整理を停止しました。未確認のデータは保持しています".into()));
    if let Some(next) = state.finish(
        snapshot
            .token
            .as_deref()
            .ok_or("整理の識別子がありません")?,
        Completion {
            restart_required: app.state::<Storage>().restart_required(),
            error: result.as_ref().err().cloned(),
            ..Completion::default()
        },
    ) {
        DataController::emit(app, next);
    }
    let _ = app.emit("background-cleanup-finished", ());
    result
}

fn reserve(state: &DataController) -> Result<Option<(Snapshot, Arc<AtomicBool>)>, String> {
    let mut guard = state
        .state
        .lock()
        .map_err(|_| "データ操作の状態を確認できません")?;
    if guard.closing
        || guard.selection.is_some()
        || guard.snapshot.phase != "idle"
        || guard.snapshot.token.is_some()
    {
        return Ok(None);
    }
    guard.snapshot = Snapshot {
        revision: guard.snapshot.revision + 1,
        token: Some(Uuid::new_v4().to_string()),
        kind: Some(Kind::Deletion),
        phase: "verifying".into(),
        busy: true,
        ..Snapshot::default()
    };
    guard.cancel = Arc::new(AtomicBool::new(false));
    Ok(Some((guard.snapshot.clone(), guard.cancel.clone())))
}

fn run_items(app: &AppHandle, check: &impl Fn() -> bool) -> Result<(), String> {
    let storage = app.state::<Storage>();
    let root = storage.native_models_root().to_owned();
    let mut errors = Vec::new();
    for item in deletion::listing(&storage)?.items {
        if check() {
            break;
        }
        let result =
            Admission::acquire_deletion(app, Some(&item.operation_id)).and_then(|admission| {
                crate::storage::deletion::automatic::meeting(
                    &storage,
                    admission.proof().ok_or("整理の実行権限を確認できません")?,
                    &item.operation_id,
                    check,
                )
            });
        if let Err(error) = result {
            errors.push(error);
        }
    }
    for item in audio_deletion::listing(&storage)?
        .items
        .into_iter()
        .filter(|item| item.review_allowed)
    {
        if check() {
            break;
        }
        let result = Admission::acquire_audio_deletion(app, Some(&item.operation_id)).and_then(
            |admission| {
                crate::storage::deletion::automatic::audio(
                    &storage,
                    admission.proof().ok_or("整理の実行権限を確認できません")?,
                    &item.operation_id,
                    check,
                )
            },
        );
        if let Err(error) = result {
            errors.push(error);
        }
    }
    for item in model_deletion::listing(&storage, &root)?
        .items
        .into_iter()
        .filter(|item| item.review_allowed)
    {
        if check() {
            break;
        }
        let result = Admission::acquire_model_deletion(app, Some(&item.operation_id)).and_then(
            |admission| {
                crate::storage::deletion::automatic::model(
                    &storage,
                    &root,
                    admission.proof().ok_or("整理の実行権限を確認できません")?,
                    &item.operation_id,
                    check,
                )
            },
        );
        if let Err(error) = result {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "中断した処理の一部を自動整理できませんでした。対象データは保持しています: {}",
            errors.join(" / ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_reservation_is_nonblocking_and_does_not_replace_foreground_work() {
        let controller = DataController::default();
        let (snapshot, cancel) = reserve(&controller).unwrap().unwrap();
        assert!(controller.state.try_lock().is_ok());
        assert!(controller.snapshot().unwrap().busy);
        assert!(controller.begin(Kind::Backup).is_err());
        assert!(reserve(&controller).unwrap().is_none());
        cancel.store(true, Ordering::Release);
        controller
            .finish(snapshot.token.as_deref().unwrap(), Completion::default())
            .unwrap();
        assert!(!controller.snapshot().unwrap().busy);
        assert_eq!(controller.snapshot().unwrap().phase, "cancelled");
        let foreground = DataController::default();
        let (old, _, _) = foreground.begin(Kind::Backup).unwrap();
        assert!(reserve(&foreground).unwrap().is_none());
        assert_eq!(foreground.snapshot().unwrap().token, old.token);
        let closing = DataController::default();
        closing.state.lock().unwrap().closing = true;
        assert!(reserve(&closing).unwrap().is_none());
    }
}
