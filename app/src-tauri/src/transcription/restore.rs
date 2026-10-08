//! Replay stored JSON through the native parser without executing whisper.
use super::*;
use crate::saved_artifact::{RestoreAssets, Role};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn restore_saved_results(
    job: &JobSnapshot,
    old: &JobSpec,
    new: &JobSpec,
    value: Value,
    model_names: &[String],
    assets: &mut impl RestoreAssets,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let decode = |spec: &JobSpec| {
        decode_parameters(spec, |snapshot| {
            if snapshot.meeting_id != spec.meeting_id {
                return Err("復元する用語の会議が一致しません".into());
            }
            snapshot.validate()
        })
    };
    let before = decode(old)?;
    let after = decode(new)?;
    if old.profile_snapshot != new.profile_snapshot
        || before.schema_version != after.schema_version
        || before.units != after.units
        || before.known_input != after.known_input
        || serde_json::to_value(&before.engine).map_err(|e| e.to_string())?
            != serde_json::to_value(&after.engine).map_err(|e| e.to_string())?
        || !(1..=64).contains(&before.engine.threads)
        || before.engine.prompt.chars().count() > 8000
        || job.kind != "transcription"
        || job.meeting_id != old.meeting_id
        || job.input_hash != old.input_hash
        || job.total_units != old.total_units
    {
        return Err("復元前後の文字起こし固定条件が一致しません".into());
    }
    let checkpoint = checked_checkpoint(job, Some(value))?;
    for output in checkpoint.outputs {
        if cancelled() {
            return Err("文字起こし成果物の復元が中断されました".into());
        }
        let unit = before
            .units
            .get(output.index)
            .ok_or("保存済み文字起こしの区間位置が不正です")?;
        let duration = |p: &Parameters| -> Result<i64, String> {
            let track = p
                .source
                .tracks
                .get(unit.track_index)
                .ok_or("固定トラックがありません")?;
            let frames = unit
                .window_end
                .checked_sub(unit.window_first)
                .ok_or("固定窓が不正です")?;
            i64::try_from(
                frames
                    .checked_mul(1_000_000)
                    .ok_or("固定窓の時間が範囲外です")?
                    / u64::from(track.sample_rate),
            )
            .map_err(|e| e.to_string())
        };
        let old_duration = duration(&before)?;
        if old_duration != duration(&after)? {
            return Err("復元で文字起こし音声時間が変更されています".into());
        }
        let bytes = assets.read(
            Role::TranscriptionResult,
            &output.json_asset_id,
            &output.json_hash,
        )?;
        if bytes.len() as u64 > Role::TranscriptionResult.max_bytes() {
            return Err("文字起こし成果物が容量上限を超えています".into());
        }
        let mut raw: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let name = raw["params"]["model"]
            .as_str()
            .ok_or("文字起こし結果のモデル名がありません")?;
        if !model_names.iter().any(|known| known == name) {
            return Err("文字起こし結果のモデル名が固定SHAの記録と一致しません".into());
        }
        let recognized = whisper::parse_output(&bytes, Path::new(name), old_duration)?;
        // Only machine-generated model metadata changes. Keep recognized text,
        // offsets and all other native metadata, including unknown fields.
        raw["params"]["model"] = json!(after.engine.model.path);
        let rebuilt = serde_json::to_vec(&raw).map_err(|e| e.to_string())?;
        if whisper::parse_output(&rebuilt, &after.engine.model.path, old_duration)? != recognized {
            return Err("再構成で文字起こし結果の本文・時刻が変化しています".into());
        }
        assets.write(
            Role::TranscriptionResult,
            &output.json_asset_id,
            &output.json_hash,
            &rebuilt,
        )?;
    }
    if cancelled() {
        return Err("文字起こし成果物の復元が中断されました".into());
    }
    Ok(())
}
