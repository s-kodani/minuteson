use super::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

pub(super) struct Admission {
    app: AppHandle,
    lease: Option<crate::analysis_worker::data_maintenance::DataMaintenanceLease>,
}

impl Admission {
    pub(super) fn acquire(app: &AppHandle) -> Result<Self, String> {
        let importing = app
            .state::<crate::import_controller::ImportController>()
            .snapshot()?;
        let lease = app.state::<AnalysisController>().begin_data_maintenance(
            app.state::<crate::session::SessionStore>()
                .snapshot()
                .is_active(),
            importing.busy || importing.recording_blocked,
            &|| false,
        )?;
        Ok(Self {
            app: app.clone(),
            lease: Some(lease),
        })
    }
    pub(super) fn cancelled(&self) -> bool {
        self.lease.as_ref().is_none_or(|lease| lease.cancelled())
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        drop(self.lease.take());
        let _ = AnalysisController::kick(&self.app);
    }
}

pub(super) enum Work {
    Select(Kind, Result<Option<PathBuf>, String>, Option<Selection>),
    Request(Selection, Request),
}

#[derive(Default)]
pub(super) struct Completion {
    pub selection: Option<Selection>,
    pub backup: Option<BackupReceipt>,
    pub restore: Option<RestoreReceipt>,
    pub error: Option<String>,
    pub attention: bool,
    pub cancelled: bool,
}

impl Completion {
    pub(super) fn failed(error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }
}

impl Work {
    pub(super) fn run(
        self,
        storage: &Storage,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Completion {
        match self {
            Self::Select(kind, path, previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    if cancelled() {
                        return Err("データの確認を取り消しました".into());
                    }
                    let Some(path) = path? else {
                        return Ok(None);
                    };
                    progress("verifying");
                    let selection = match kind {
                        Kind::Backup => {
                            Selection::Backup(storage.prepare_backup_destination(&path, cancelled)?)
                        }
                        Kind::Restore => Selection::Restore(
                            storage.prepare_additive_restore(&path, cancelled, progress)?,
                        ),
                    };
                    if cancelled() {
                        return Err("データの確認を取り消しました".into());
                    }
                    Ok(Some(selection))
                }))
                .unwrap_or_else(|_| {
                    Err("バックアップの確認が予期せず停止しました。選び直してください".into())
                });
                match result {
                    Ok(selection) => Completion {
                        selection,
                        ..Completion::default()
                    },
                    Err(error) => Completion {
                        error: Some(error),
                        cancelled: cancelled(),
                        ..Completion::default()
                    },
                }
            }
            Self::Request(mut selection, request) => {
                let mut completion = Completion::default();
                let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                    match (&mut selection, request.action) {
                        (Selection::Backup(prepared), Action::Publish) => {
                            match prepared.publish(storage, &request.confirmation_id, cancelled) {
                                Ok(receipt) => completion.backup = Some(receipt),
                                Err(error) => {
                                    // Renamed but not fully acknowledged is terminal: never
                                    // retry an overwrite from this same operation.
                                    completion.attention = error.published;
                                    return Err(error.message);
                                }
                            }
                        }
                        (Selection::Restore(prepared), Action::Refresh) => {
                            prepared.refresh(storage, cancelled, progress)?;
                        }
                        (Selection::Restore(prepared), Action::Publish) => {
                            completion.restore = Some(prepared.publish(
                                storage,
                                &request.confirmation_id,
                                cancelled,
                                progress,
                            )?);
                        }
                        _ => return Err("このデータ操作は再確認できません".into()),
                    }
                    Ok(())
                }));
                match result {
                    Ok(Err(error)) => {
                        completion.error = Some(error);
                        completion.cancelled = cancelled();
                    }
                    Err(_) => {
                        completion.error = Some("データ処理が予期せず停止しました。公開済みの可能性があります。結果を確認してください".into());
                        // Restore can query its durable batch on retry. Backup
                        // panic may have crossed rename before caching an outcome.
                        completion.attention = matches!(selection, Selection::Backup(_));
                    }
                    Ok(Ok(())) => {
                        completion.cancelled = completion.backup.is_none()
                            && completion.restore.is_none()
                            && cancelled();
                    }
                }
                completion.selection = Some(selection);
                completion
            }
        }
    }
}
