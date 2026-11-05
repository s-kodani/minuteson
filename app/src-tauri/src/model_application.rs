//! No engine execution. Both preview and apply re-read the latest successful
//! trial, current model files, and trial artifacts before touching preferences.
use crate::storage::{
    model_profiles::{ApplyPreview, ProfileName, ProfileView},
    Storage,
};
use std::path::Path;

#[cfg(test)]
pub(crate) fn preview(
    storage: &Storage,
    models_root: &Path,
    name: ProfileName,
    package: &str,
    review: &str,
    check: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<ApplyPreview, String> {
    preview_with_threads(
        storage,
        models_root,
        name,
        package,
        review,
        check,
        None,
        cancelled,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn preview_with_threads(
    storage: &Storage,
    models_root: &Path,
    name: ProfileName,
    package: &str,
    review: &str,
    check: &str,
    diarization_threads: Option<u32>,
    cancelled: &impl Fn() -> bool,
) -> Result<ApplyPreview, String> {
    preview_with_tuning(
        storage,
        models_root,
        name,
        package,
        review,
        check,
        diarization_threads,
        None,
        cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn preview_with_tuning(
    storage: &Storage,
    models_root: &Path,
    name: ProfileName,
    package: &str,
    review: &str,
    check: &str,
    diarization_threads: Option<u32>,
    diarization_cluster_threshold_milli: Option<u32>,
    cancelled: &impl Fn() -> bool,
) -> Result<ApplyPreview, String> {
    preview_with_vad(
        storage,
        models_root,
        name,
        package,
        review,
        check,
        diarization_threads,
        diarization_cluster_threshold_milli,
        None,
        cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn preview_with_vad(
    storage: &Storage,
    models_root: &Path,
    name: ProfileName,
    package: &str,
    review: &str,
    check: &str,
    diarization_threads: Option<u32>,
    diarization_cluster_threshold_milli: Option<u32>,
    whisper_vad_options: Option<crate::transcription::vad::VadOptions>,
    cancelled: &impl Fn() -> bool,
) -> Result<ApplyPreview, String> {
    let (record, directory) = crate::model_compatibility::verify_success(
        storage,
        models_root,
        check,
        package,
        review,
        cancelled,
    )?;
    let result = storage.preview_model_profile_with_vad(
        name,
        &record,
        &directory,
        check,
        diarization_threads,
        diarization_cluster_threshold_milli,
        whisper_vad_options,
    )?;
    if cancelled() {
        return Err("モデル適用の事前確認を取り消しました".into());
    }
    Ok(result)
}
pub(crate) fn apply(
    storage: &Storage,
    models_root: &Path,
    operation: &str,
    preview: &ApplyPreview,
    cancelled: &impl Fn() -> bool,
) -> Result<ProfileView, String> {
    let (record, directory) = crate::model_compatibility::verify_success(
        storage,
        models_root,
        &preview.check_operation_id,
        &preview.package_id,
        &preview.review_hash,
        cancelled,
    )?;
    storage.apply_model_profile(operation, preview, &record, &directory, cancelled)
}
