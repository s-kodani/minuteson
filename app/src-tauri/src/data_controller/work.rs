use super::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

pub(super) struct Admission {
    app: AppHandle,
    lease: Option<crate::analysis_worker::data_maintenance::DataMaintenanceLease>,
}
enum DeletionLane {
    Meeting,
    Model,
    Audio,
}

impl Admission {
    pub(super) fn acquire(app: &AppHandle) -> Result<Self, String> {
        app.state::<Storage>().require_data_writable()?;
        Self::acquire_worker(app, None, DeletionLane::Meeting)
    }
    #[cfg(unix)]
    pub(super) fn acquire_deletion(
        app: &AppHandle,
        operation: Option<&str>,
    ) -> Result<Self, String> {
        deletion::authorize_review(&app.state::<Storage>(), operation)?;
        Self::acquire_worker(app, operation, DeletionLane::Meeting)
    }
    #[cfg(unix)]
    pub(super) fn acquire_model_deletion(
        app: &AppHandle,
        operation: Option<&str>,
    ) -> Result<Self, String> {
        model_deletion::authorize_review(&app.state::<Storage>(), operation)?;
        Self::acquire_worker(app, operation, DeletionLane::Model)
    }
    #[cfg(unix)]
    pub(super) fn acquire_audio_deletion(
        app: &AppHandle,
        operation: Option<&str>,
    ) -> Result<Self, String> {
        audio_deletion::authorize_review(&app.state::<Storage>(), operation)?;
        Self::acquire_worker(app, operation, DeletionLane::Audio)
    }
    fn acquire_worker(
        app: &AppHandle,
        deletion_operation: Option<&str>,
        lane: DeletionLane,
    ) -> Result<Self, String> {
        let importing = app
            .state::<crate::import_controller::ImportController>()
            .snapshot()?;
        let recording = app
            .state::<crate::session::SessionStore>()
            .snapshot()
            .is_active();
        let import_blocked = importing.busy || importing.recording_blocked;
        let analysis = app.state::<AnalysisController>();
        #[cfg(unix)]
        let lease = if let Some(operation) = deletion_operation {
            if matches!(lane, DeletionLane::Audio) {
                analysis.begin_audio_deletion_maintenance(
                    &app.state::<Storage>(),
                    operation,
                    recording,
                    import_blocked,
                    &|| false,
                )?
            } else if matches!(lane, DeletionLane::Model) {
                analysis.begin_model_deletion_maintenance(
                    &app.state::<Storage>(),
                    operation,
                    recording,
                    import_blocked,
                    &|| false,
                )?
            } else {
                analysis.begin_deletion_maintenance(
                    &app.state::<Storage>(),
                    operation,
                    recording,
                    import_blocked,
                    &|| false,
                )?
            }
        } else {
            analysis.begin_data_maintenance(recording, import_blocked, &|| false)?
        };
        #[cfg(not(unix))]
        let lease = {
            let _ = (deletion_operation, lane);
            analysis.begin_data_maintenance(recording, import_blocked, &|| false)?
        };
        Ok(Self {
            app: app.clone(),
            lease: Some(lease),
        })
    }
    pub(super) fn cancelled(&self) -> bool {
        self.lease.as_ref().is_none_or(|lease| lease.cancelled())
    }
    pub(super) fn proof(
        &self,
    ) -> Option<&crate::analysis_worker::data_maintenance::DataMaintenanceLease> {
        self.lease.as_ref()
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        drop(self.lease.take());
        if !self.app.state::<Storage>().restart_required() {
            let _ = AnalysisController::kick(&self.app);
        }
    }
}

#[cfg(unix)]
pub(crate) enum ModelTarget {
    Fresh { package: String, review: String },
    Pending(String),
}
#[cfg(unix)]
pub(crate) enum AudioTarget {
    Fresh(String),
    Pending(String),
}

pub(super) enum Work {
    Select(Kind, Result<Option<PathBuf>, String>, Option<Selection>),
    Request(Selection, Request),
    Recovery(recovery::Source, String, Option<Selection>),
    #[cfg(target_os = "macos")]
    PendingMove(Option<Selection>),
    #[cfg(unix)]
    Deletion(String, bool, Option<Selection>),
    #[cfg(unix)]
    ModelDeletion(PathBuf, ModelTarget, Option<Selection>),
    #[cfg(unix)]
    AudioDeletion(AudioTarget, Option<Selection>),
    #[cfg(unix)]
    PartialRecording(String, Option<Selection>),
}

#[derive(Default)]
pub(super) struct Completion {
    pub selection: Option<Selection>,
    pub backup: Option<BackupReceipt>,
    pub restore: Option<RestoreReceipt>,
    pub recovery: Option<recovery::Receipt>,
    pub relocation: Option<MoveReceipt>,
    #[cfg(unix)]
    pub deletion: Option<deletion::Receipt>,
    #[cfg(unix)]
    pub model_deletion: Option<model_deletion::Receipt>,
    #[cfg(unix)]
    pub audio_deletion: Option<audio_deletion::Receipt>,
    #[cfg(unix)]
    pub partial_recording: Option<partial_recording::Receipt>,
    pub restart_required: bool,
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
    #[cfg(test)]
    #[cfg(target_os = "macos")]
    pub(super) fn run_with_bootstrap(
        self,
        storage: &Storage,
        bootstrap: &crate::bootstrap::Bootstrap,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Completion {
        self.run_with_bootstrap_admitted(storage, bootstrap, cancelled, progress, None)
    }
    #[cfg(target_os = "macos")]
    pub(super) fn run_with_bootstrap_admitted(
        self,
        storage: &Storage,
        bootstrap: &crate::bootstrap::Bootstrap,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
        admission: Option<&crate::analysis_worker::data_maintenance::DataMaintenanceLease>,
    ) -> Completion {
        match self {
            Self::PendingMove(previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    bootstrap
                        .prepare_pending_move(storage, cancelled, progress)
                        .map(|p| {
                            Selection::Relocation(crate::bootstrap::movement::Prepared::Pending(
                                Box::new(p),
                            ))
                        })
                }))
                .unwrap_or_else(|_| {
                    Err("未完了移動の確認が予期せず停止しました。ファイルを保持します".into())
                });
                match result {
                    Ok(selection) => Completion {
                        selection: Some(selection),
                        ..Completion::default()
                    },
                    Err(error) => Completion {
                        error: Some(error),
                        cancelled: cancelled(),
                        ..Completion::default()
                    },
                }
            }
            Self::Select(Kind::Relocation, path, previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    if cancelled() {
                        return Err("保存先の選択を取り消しました".into());
                    }
                    let Some(parent) = path? else {
                        return Ok(None);
                    };
                    let prepared = bootstrap.prepare_move(storage, &parent, cancelled, progress)?;
                    if cancelled() {
                        return Err("保存先の確認を取り消しました".into());
                    }
                    Ok(Some(Selection::Relocation(prepared)))
                }))
                .unwrap_or_else(|_| {
                    Err("保存先の確認が予期せず停止しました。旧保存先を保持します".into())
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
            Self::Request(Selection::Relocation(mut prepared), request) => {
                let mut completion = Completion::default();
                let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                    match request.action {
                        Action::Refresh => {
                            prepared.refresh(bootstrap, storage, cancelled, progress)?
                        }
                        Action::Publish => {
                            completion.relocation = Some(prepared.publish(
                                bootstrap,
                                storage,
                                &request.confirmation_id,
                                cancelled,
                                progress,
                            )?)
                        }
                        Action::Keep => {
                            completion.relocation = Some(prepared.keep(
                                bootstrap,
                                storage,
                                &request.confirmation_id,
                                cancelled,
                            )?)
                        }
                        _ => return Err("確認対象と保存先変更の操作が一致しません".into()),
                    }
                    Ok(())
                }));
                completion.restart_required = storage.restart_required();
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => completion.error = Some(error),
                    Err(_) => {
                        completion.error = Some(
                            "保存先変更が予期せず停止しました。旧保存先と途中コピーを保持します"
                                .into(),
                        )
                    }
                }
                if completion.error.is_some() {
                    completion.attention = completion.restart_required
                        || bootstrap
                            .view()
                            .map_or(true, |v| v.pending_operation_id.is_some());
                    completion.cancelled = !completion.attention && cancelled();
                }
                completion.selection = Some(Selection::Relocation(prepared));
                completion
            }
            other => other.run_admitted(storage, cancelled, progress, admission),
        }
    }

    #[cfg(test)]
    pub(super) fn run(
        self,
        storage: &Storage,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
    ) -> Completion {
        self.run_admitted(storage, cancelled, progress, None)
    }
    pub(super) fn run_admitted(
        self,
        storage: &Storage,
        cancelled: &impl Fn() -> bool,
        progress: &impl Fn(&str),
        admission: Option<&crate::analysis_worker::data_maintenance::DataMaintenanceLease>,
    ) -> Completion {
        match self {
            #[cfg(unix)]
            Self::PartialRecording(session, previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    progress("verifying");
                    let lease =
                        admission.ok_or("部分救出の確認に必要な共有実行排他がありません")?;
                    partial_recording::Prepared::fresh(storage, &session, lease, cancelled)
                        .map(Selection::PartialRecording)
                }))
                .unwrap_or_else(|_| Err("部分救出の検査が停止しました。原音声は保持します".into()));
                match result {
                    Ok(selection) => Completion {
                        selection: Some(selection),
                        ..Completion::default()
                    },
                    Err(error) => Completion {
                        error: Some(error),
                        cancelled: cancelled(),
                        ..Completion::default()
                    },
                }
            }
            #[cfg(unix)]
            Self::AudioDeletion(target, previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    progress("verifying_audio_delete_scope");
                    let lease = admission.ok_or("音声削除確認に必要な共有実行排他がありません")?;
                    let p = match target {
                        AudioTarget::Fresh(meeting) => {
                            audio_deletion::Prepared::fresh(storage, &meeting, lease, cancelled)?
                        }
                        AudioTarget::Pending(id) => {
                            audio_deletion::Prepared::pending(storage, &id, lease, cancelled)?
                        }
                    };
                    Ok::<_, String>(Selection::AudioDeletion(p))
                }))
                .unwrap_or_else(|_| {
                    Err("音声削除対象の検査が停止しました。対象を保持します".into())
                });
                match result {
                    Ok(selection) => Completion {
                        selection: Some(selection),
                        restart_required: storage.restart_required(),
                        ..Completion::default()
                    },
                    Err(error) => Completion {
                        error: Some(error),
                        attention: storage.data_write_blocked(),
                        restart_required: storage.restart_required(),
                        cancelled: cancelled(),
                        ..Completion::default()
                    },
                }
            }
            #[cfg(unix)]
            Self::ModelDeletion(root, target, previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    progress("verifying_model_delete_scope");
                    let lease =
                        admission.ok_or("モデル削除確認に必要な共有実行排他がありません")?;
                    let p = match target {
                        ModelTarget::Fresh { package, review } => model_deletion::Prepared::fresh(
                            storage, &root, &package, &review, lease, cancelled,
                        )?,
                        ModelTarget::Pending(id) => model_deletion::Prepared::pending(
                            storage, &root, &id, lease, cancelled,
                        )?,
                    };
                    Ok::<_, String>(Selection::ModelDeletion(p))
                }))
                .unwrap_or_else(|_| {
                    Err("モデル削除対象の検査が停止しました。対象を保持します".into())
                });
                match result {
                    Ok(selection) => Completion {
                        selection: Some(selection),
                        restart_required: storage.restart_required(),
                        ..Completion::default()
                    },
                    Err(error) => Completion {
                        error: Some(error),
                        attention: storage.data_write_blocked(),
                        cancelled: cancelled(),
                        restart_required: storage.restart_required(),
                        ..Completion::default()
                    },
                }
            }
            #[cfg(unix)]
            Self::Deletion(id, pending, previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    progress("verifying_delete_scope");
                    let p = if pending {
                        deletion::Prepared::pending(storage, &id, cancelled)?
                    } else {
                        deletion::Prepared::fresh(storage, &id, cancelled)?
                    };
                    Ok::<_, String>(Selection::Deletion(p))
                }))
                .unwrap_or_else(|_| Err("削除対象の検査が停止しました。データを保持します".into()));
                match result {
                    Ok(selection) => Completion {
                        selection: Some(selection),
                        restart_required: storage.restart_required(),
                        ..Completion::default()
                    },
                    Err(error) => Completion {
                        error: Some(error),
                        attention: storage.data_write_blocked(),
                        cancelled: cancelled(),
                        restart_required: storage.restart_required(),
                        ..Completion::default()
                    },
                }
            }
            #[cfg(target_os = "macos")]
            Self::PendingMove(_) => {
                Completion::failed("起動設定から未完了移動を確認してください".into())
            }
            Self::Recovery(source, id, previous) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    drop(previous);
                    progress("verifying");
                    storage
                        .prepare_recovery(source, &id, cancelled)
                        .map(Selection::Recovery)
                }))
                .unwrap_or_else(|_| {
                    Err("要確認対象の検査が予期せず停止しました。ファイルは保持します".into())
                });
                match result {
                    Ok(selection) => Completion {
                        selection: Some(selection),
                        ..Completion::default()
                    },
                    Err(error) => Completion {
                        error: Some(error),
                        cancelled: cancelled(),
                        ..Completion::default()
                    },
                }
            }
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
                        Kind::Recovery => return Err("要確認一覧から対象を選択してください".into()),
                        Kind::Relocation => return Err("このOSの保存先変更は保留中です".into()),
                        Kind::Deletion => {
                            return Err("会議または未完了削除の一覧から確認してください".into())
                        }
                        Kind::ModelDeletion => {
                            return Err(
                                "モデルまたは未完了モデル削除の一覧から確認してください".into()
                            )
                        }
                        Kind::AudioDeletion => {
                            return Err(
                                "会議または音声削除の確認待ち一覧から選択してください".into()
                            )
                        }
                        Kind::PartialRecording => {
                            return Err("中断録音の一覧から選択してください".into())
                        }
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
                #[cfg(unix)]
                if let Selection::PartialRecording(prepared) = &mut selection {
                    let mut completion = Completion::default();
                    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                        let lease = admission.ok_or("部分救出に必要な共有実行排他がありません")?;
                        match request.action {
                            Action::Refresh => {
                                progress("verifying");
                                if let Some(receipt) = prepared.committed_result(storage)? {
                                    completion.partial_recording = Some(receipt);
                                } else {
                                    prepared.refresh(storage, lease, cancelled)?;
                                }
                            }
                            Action::Recover | Action::Keep => {
                                completion.partial_recording = Some(prepared.resolve(
                                    storage,
                                    lease,
                                    &request.confirmation_id,
                                    request.action == Action::Recover,
                                    cancelled,
                                    progress,
                                )?)
                            }
                            _ => return Err("部分救出の確認と操作が一致しません".into()),
                        }
                        Ok(())
                    }));
                    let panic = result.is_err();
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => completion.error = Some(error),
                        Err(_) => {
                            completion.error =
                                Some("部分救出が予期せず停止しました。保存結果を照合します".into())
                        }
                    }
                    if completion.partial_recording.is_none()
                        && request.action == Action::Recover
                        && completion.error.is_some()
                    {
                        match prepared.committed_result(storage) {
                            Ok(Some(receipt)) => {
                                completion.partial_recording = Some(receipt);
                                completion.error = None;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                completion.error = Some(error);
                                completion.attention = true;
                            }
                        }
                    }
                    // If a panic cannot be reconciled, require inspection, not
                    // an automatic second publication from an old confirmation.
                    completion.attention |= panic
                        && request.action == Action::Recover
                        && completion.partial_recording.is_none();
                    completion.cancelled = completion.partial_recording.is_none()
                        && !completion.attention
                        && cancelled();
                    completion.selection = Some(selection);
                    return completion;
                }
                #[cfg(unix)]
                if let Selection::AudioDeletion(prepared) = &mut selection {
                    let mut completion = Completion::default();
                    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                        let lease =
                            admission.ok_or("音声削除操作に必要な共有実行排他がありません")?;
                        match request.action {
                            Action::Refresh => {
                                progress("verifying_audio_delete_scope");
                                prepared.refresh(storage, lease, cancelled)?;
                            }
                            Action::ReviewRemaining => {
                                progress("verifying_audio_delete_scope");
                                prepared.review_remaining(
                                    storage,
                                    lease,
                                    &request.confirmation_id,
                                    cancelled,
                                )?;
                            }
                            Action::Publish | Action::DeleteIncludingUnregistered => {
                                completion.audio_deletion = Some(prepared.execute(
                                    storage,
                                    lease,
                                    &request.confirmation_id,
                                    request.action == Action::DeleteIncludingUnregistered,
                                    cancelled,
                                    progress,
                                )?);
                            }
                            Action::Keep => {
                                completion.audio_deletion = Some(prepared.keep(
                                    storage,
                                    lease,
                                    &request.confirmation_id,
                                    cancelled,
                                )?)
                            }
                            _ => return Err("音声削除確認と操作が一致しません".into()),
                        }
                        Ok(())
                    }));
                    match result {
                        Ok(Ok(())) => {}, Ok(Err(error)) => completion.error = Some(error),
                        Err(_) => completion.error = Some("音声削除が予期せず停止しました。既に消した音声は復元せず、保存済み進捗を確認してください".into()),
                    }
                    completion.restart_required = storage.restart_required();
                    completion.attention = completion.error.is_some()
                        && (storage.data_write_blocked() || prepared.preview().pending);
                    completion.cancelled =
                        completion.audio_deletion.is_none() && !completion.attention && cancelled();
                    completion.selection = Some(selection);
                    return completion;
                }
                #[cfg(unix)]
                if let Selection::ModelDeletion(prepared) = &mut selection {
                    let mut completion = Completion::default();
                    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                        let lease =
                            admission.ok_or("モデル削除操作に必要な共有実行排他がありません")?;
                        match request.action {
                            Action::Refresh => {
                                progress("verifying_model_delete_scope");
                                prepared.refresh(storage, lease, cancelled)?;
                            }
                            Action::ReviewRemaining => {
                                progress("verifying_model_delete_scope");
                                prepared.review_remaining(
                                    storage,
                                    lease,
                                    &request.confirmation_id,
                                    cancelled,
                                )?;
                            }
                            Action::Publish | Action::DeleteIncludingUnregistered => {
                                completion.model_deletion = Some(prepared.execute(
                                    storage,
                                    lease,
                                    &request.confirmation_id,
                                    request.action == Action::DeleteIncludingUnregistered,
                                    cancelled,
                                    progress,
                                )?);
                            }
                            Action::Keep => {
                                completion.model_deletion = Some(prepared.keep(
                                    storage,
                                    lease,
                                    &request.confirmation_id,
                                    cancelled,
                                )?)
                            }
                            _ => return Err("モデル削除確認と操作が一致しません".into()),
                        }
                        Ok(())
                    }));
                    match result {
                        Ok(Ok(())) => {},
                        Ok(Err(error)) => completion.error = Some(error),
                        Err(_) => completion.error = Some("モデル削除が予期せず停止しました。既に消したファイルは復元せず、保存済み進捗を確認してください".into()),
                    }
                    completion.restart_required = storage.restart_required();
                    completion.attention = completion.error.is_some()
                        && (storage.data_write_blocked() || prepared.preview().pending);
                    completion.cancelled =
                        completion.model_deletion.is_none() && !completion.attention && cancelled();
                    completion.selection = Some(selection);
                    return completion;
                }
                #[cfg(unix)]
                if let Selection::Deletion(prepared) = &mut selection {
                    let mut completion = Completion::default();
                    let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
                        match request.action {
                            Action::Refresh => prepared.refresh(storage, cancelled)?,
                            Action::ReviewRemaining => {
                                progress("verifying_delete_scope");
                                prepared.review_remaining(
                                    storage,
                                    admission.ok_or("再確認に必要な共有実行排他がありません")?,
                                    &request.confirmation_id,
                                    cancelled,
                                )?;
                            }
                            Action::Publish | Action::DeleteIncludingUnregistered => {
                                completion.deletion = Some(prepared.execute(
                                    storage,
                                    admission.ok_or("削除に必要な共有実行排他がありません")?,
                                    &request.confirmation_id,
                                    request.action == Action::DeleteIncludingUnregistered,
                                    cancelled,
                                    progress,
                                )?)
                            }
                            Action::Keep => {
                                completion.deletion = Some(
                                    prepared.keep(
                                        storage,
                                        admission
                                            .ok_or("保持・中止に必要な共有実行排他がありません")?,
                                        &request.confirmation_id,
                                        cancelled,
                                    )?,
                                )
                            }
                            _ => return Err("削除確認と操作が一致しません".into()),
                        }
                        Ok(())
                    }));
                    match result { Ok(Ok(()))=>{},Ok(Err(error))=>completion.error=Some(error),Err(_)=>completion.error=Some("削除処理が予期せず停止しました。既に消したデータは復元せず、保存済み進捗を確認してください".into()) }
                    completion.restart_required = storage.restart_required();
                    completion.attention = completion.error.is_some()
                        && (storage.data_write_blocked() || prepared.preview().pending);
                    completion.cancelled =
                        completion.deletion.is_none() && !completion.attention && cancelled();
                    completion.selection = Some(selection);
                    return completion;
                }
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
                        (Selection::Recovery(prepared), Action::Refresh) => {
                            let p = prepared.preview();
                            *prepared = storage.prepare_recovery(
                                p.item.source,
                                &p.item.operation_id,
                                cancelled,
                            )?;
                        }
                        (Selection::Recovery(prepared), Action::Recover | Action::Keep) => {
                            completion.recovery = Some(prepared.resolve(
                                storage,
                                &request.confirmation_id,
                                request.action == Action::Keep,
                                cancelled,
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
                            && completion.recovery.is_none()
                            && cancelled();
                    }
                }
                completion.selection = Some(selection);
                completion
            }
        }
    }
}
