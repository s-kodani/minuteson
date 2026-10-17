use super::*;
use crate::saved_artifact::{RestoreAssets, Role};
use serde_json::Value;

#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_saved_results(
    job: &JobSnapshot,
    old: &JobSpec,
    new: &JobSpec,
    value: Value,
    old_initial: &str,
    new_initial: &str,
    assets: &mut impl RestoreAssets,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    if old.parameters["schemaVersion"] == 2 {
        return known::restore_saved_results(
            job,
            old,
            new,
            value,
            old_initial,
            new_initial,
            assets,
            cancelled,
        );
    }
    let before: Envelope =
        serde_json::from_value(old.parameters.clone()).map_err(|e| e.to_string())?;
    let after: Envelope =
        serde_json::from_value(new.parameters.clone()).map_err(|e| e.to_string())?;
    for (spec, envelope, initial) in [(old, &before, old_initial), (new, &after, new_initial)] {
        if spec.kind != JobKind::Diarization
            || envelope.schema_version != 1
            || envelope.request.meeting_id != spec.meeting_id
            || envelope.request.operation_id != initial
            || spec.total_units != Some(envelope.diarization.tracks.len() as i64)
        {
            return Err("復元する話者分離の固定計画・由来が不正です".into());
        }
        validate_tracks(&envelope.diarization.source, &envelope.diarization.tracks)?;
        managed::validate(
            &spec.profile_snapshot,
            Execution::Sherpa(&envelope.diarization.engine),
        )?;
        for track in &envelope.diarization.tracks {
            envelope
                .diarization
                .engine
                .validate_settings(track.num_speakers)?;
        }
    }
    if before.diarization.resident_runtime != after.diarization.resident_runtime
        || before.diarization.engine != after.diarization.engine
        || old.profile_snapshot != new.profile_snapshot
        || old.total_units != new.total_units
        || job.kind != "diarization"
        || job.meeting_id != old.meeting_id
        || job.input_hash != old.input_hash
        || job.total_units != old.total_units
    {
        return Err("復元前後の話者分離固定条件が一致しません".into());
    }
    let checkpoint = checked_checkpoint(job, Some(value))?;
    for output in checkpoint.outputs {
        if cancelled() {
            return Err("話者分離成果物の復元が中断されました".into());
        }
        let bytes = assets.read(
            Role::DiarizationResult,
            &output.result_asset_id,
            &output.result_hash,
        )?;
        if bytes.len() as u64 > Role::DiarizationResult.max_bytes() {
            return Err("話者分離成果物が容量上限を超えています".into());
        }
        let result: ResultFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let track = before
            .diarization
            .source
            .tracks
            .get(output.index)
            .ok_or("保存済み話者分離の位置が範囲外です")?;
        if result.track_id != track.id {
            return Err("保存済み話者分離結果がチェックポイントのトラックと一致しません".into());
        }
        let rebuilt = restore_result(&bytes, old, new)?;
        assets.write(
            Role::DiarizationResult,
            &output.result_asset_id,
            &output.result_hash,
            &rebuilt,
        )?;
    }
    if cancelled() {
        return Err("話者分離成果物の復元が中断されました".into());
    }
    Ok(())
}
