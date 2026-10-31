//! Replay stored JSON through the selected ASR parser without executing models.
use super::*;
use crate::saved_artifact::{RestoreAssets, Role};
use serde_json::{json, Value};
use std::path::Path;

/// Bind every native ASR reference to the fixed model bytes before replacing
/// historical basenames with logical unavailable paths. Text and times are
/// validated by the same adapter used for normal result publication.
fn restore_asr_result(
    bytes: &[u8],
    before: &AsrConfig,
    after: &AsrConfig,
    model_names: &[String],
    duration: i64,
    aliases: &impl Fn(&crate::local_engine::Fingerprint, &str) -> Vec<String>,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let raw: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let models = raw
        .get("models")
        .and_then(Value::as_object)
        .ok_or("ASR結果のモデル参照がありません")?;
    let mut runtime = before.clone();
    let rebind = |slot: &str,
                  proof: &mut crate::local_engine::Fingerprint,
                  role: &str|
     -> Result<(), String> {
        let name = models
            .get(slot)
            .and_then(Value::as_str)
            .ok_or("ASR結果の必要ファイル参照がありません")?;
        let logical = proof.path.to_str().ok_or("固定モデルの参照が不正です")?;
        let portable = logical == format!("mon-unavailable:{}", proof.sha256);
        let original_basename = logical.rsplit(['/', '\\']).next().unwrap_or("");
        if name != logical
            && (portable || name != original_basename)
            && !aliases(proof, role).iter().any(|known| known == name)
            && !(slot == "model" && model_names.iter().any(|known| known == name))
        {
            return Err("ASR結果のファイル名が固定した容量・SHA・役割と一致しません".into());
        }
        proof.path = name.into();
        Ok(())
    };
    rebind("model", &mut runtime.model, "model")?;
    for (slot, proof) in &mut runtime
        .backend
        .as_mut()
        .ok_or("ASRバックエンドがありません")?
        .files
    {
        rebind(slot, proof, "artifact")?;
    }
    let adapter = asr::adapter(before)?;
    let recognized = adapter.parse_result(bytes, &runtime, duration)?;
    let logical_before = adapter.logical_output(bytes, &runtime, before, duration)?;
    let rebuilt = adapter.logical_output(&logical_before, before, after, duration)?;
    if adapter.parse_result(&logical_before, before, duration)? != recognized
        || adapter.parse_result(&rebuilt, after, duration)? != recognized
    {
        return Err("再構成でASR結果の本文・時刻が変化しています".into());
    }
    Ok((logical_before, rebuilt))
}

pub(crate) fn restore_parameters(
    meeting: &str,
    original: &Value,
    value: &mut Value,
    resolver: &impl crate::job_codec::Resolver,
) -> Result<(), String> {
    if let Some(source) = original.get("source") {
        let source: crate::storage::RecordedAudioPlan =
            serde_json::from_value(source.clone()).map_err(|e| e.to_string())?;
        let known: Option<crate::storage::KnownInput> = original
            .get("knownInput")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| e.to_string())?;
        value["source"] =
            serde_json::to_value(resolver.audio_plan(meeting, &source, known.as_ref())?)
                .map_err(|e| e.to_string())?;
    }
    Ok(())
}

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
        || before.recognition_policy != after.recognition_policy
        || before.resident_runtime != after.resident_runtime
        || before.acceleration_policy != after.acceleration_policy
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
    if before.recognition_policy == Some(RecognitionPolicy::AdapterV1)
        && checkpoint.pending_recovery.is_some()
    {
        return Err("ASRバックエンドに未対応の再認識状態があります".into());
    }
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
        let references = std::iter::once((&output.json_asset_id, &output.json_hash)).chain(
            output
                .recognition
                .as_ref()
                .and_then(|a| a.rejected.as_ref())
                .map(|r| (&r.json_asset_id, &r.json_hash)),
        );
        let mut rewritten = vec![];
        for (id, hash) in references {
            let bytes = assets.read(Role::TranscriptionResult, id, hash)?;
            if bytes.len() as u64 > Role::TranscriptionResult.max_bytes() {
                return Err("文字起こし成果物が容量上限を超えています".into());
            }
            if before.recognition_policy == Some(RecognitionPolicy::AdapterV1) {
                let (logical_before, rebuilt) = restore_asr_result(
                    &bytes,
                    &before.engine,
                    &after.engine,
                    model_names,
                    old_duration,
                    &|proof, role| assets.model_aliases(role, proof.size, &proof.sha256),
                )?;
                rewritten.push((id, hash, logical_before, rebuilt));
                continue;
            }
            let mut raw: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let name = raw["params"]["model"]
                .as_str()
                .ok_or("文字起こし結果のモデル名がありません")?;
            if !model_names.iter().any(|known| known == name) {
                return Err("文字起こし結果のモデル名が固定SHAの記録と一致しません".into());
            }
            let recognized = whisper::parse_output(&bytes, Path::new(name), old_duration)?;
            recovery::rebind_json(&mut raw, model_names, &before.engine.model.path)?;
            raw["params"]["model"] = json!(before.engine.model.path);
            let logical_before = serde_json::to_vec(&raw).map_err(|e| e.to_string())?;
            recovery::rebind_json(
                &mut raw,
                &[before.engine.model.path.to_string_lossy().into_owned()],
                &after.engine.model.path,
            )?;
            raw["params"]["model"] = json!(after.engine.model.path);
            let rebuilt = serde_json::to_vec(&raw).map_err(|e| e.to_string())?;
            if whisper::parse_output(&rebuilt, &after.engine.model.path, old_duration)?
                != recognized
            {
                return Err("再構成で文字起こし結果の本文・時刻が変化しています".into());
            }
            rewritten.push((id, hash, logical_before, rebuilt));
        }
        checked_output(
            &before,
            &output,
            &rewritten[0].2,
            rewritten.get(1).map(|v| v.2.as_slice()),
            old_duration,
        )?;
        checked_output(
            &after,
            &output,
            &rewritten[0].3,
            rewritten.get(1).map(|v| v.3.as_slice()),
            old_duration,
        )?;
        for (id, hash, _, rebuilt) in rewritten {
            assets.write(Role::TranscriptionResult, id, hash, &rebuilt)?;
        }
    }
    if let Some(pending) = checkpoint.pending_recovery {
        let bytes = assets.read(
            Role::TranscriptionResult,
            &pending.reference.json_asset_id,
            &pending.reference.json_hash,
        )?;
        let mut state: recovery::State =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let unit = before
            .units
            .get(pending.index)
            .ok_or("再認識の保存位置がありません")?;
        let rate = u64::from(before.source.tracks[unit.track_index].sample_rate);
        let first = (unit.core_first - unit.window_first) * 16_000 / rate;
        let end = (unit.core_end - unit.window_first) * 16_000 / rate;
        let duration = ((unit.window_end - unit.window_first) * 1_000_000 / rate) as i64;
        state.rebind_models(model_names, &before.engine.model.path)?;
        state.validate_input(first, end, duration, &before.engine.model.path)?;
        state.rebind_models(
            &[before.engine.model.path.to_string_lossy().into_owned()],
            &after.engine.model.path,
        )?;
        assets.write(
            Role::TranscriptionResult,
            &pending.reference.json_asset_id,
            &pending.reference.json_hash,
            &serde_json::to_vec(&state).map_err(|e| e.to_string())?,
        )?;
    }
    if cancelled() {
        return Err("文字起こし成果物の復元が中断されました".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
