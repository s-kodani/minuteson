//! Meeting-independent voice identities. Embeddings never establish authentication.
use crate::diarization::{embedding, sherpa::SherpaConfig};
use crate::local_engine::Fingerprint;
use crate::storage::{JobKind, JobSnapshot, Storage};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) const EXTRACTOR: &str = "sherpa-1.13.8-mono16k-normalized-v1";
pub(crate) const MAX_DIMENSION: usize = 4096;

pub(crate) fn normalize(values: &[f32]) -> Result<Vec<f32>, String> {
    if values.is_empty() || values.len() > MAX_DIMENSION || values.iter().any(|v| !v.is_finite()) {
        return Err("声の特徴量の次元・数値が不正です".into());
    }
    let norm = values
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm < 1e-12 {
        return Err("声の特徴量が空です".into());
    }
    Ok(values
        .iter()
        .map(|v| (f64::from(*v) / norm) as f32)
        .collect())
}
pub(crate) fn encode(values: &[f32]) -> Result<Vec<u8>, String> {
    Ok(normalize(values)?
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect())
}
pub(crate) fn decode(bytes: &[u8], dimension: usize) -> Result<Vec<f32>, String> {
    if dimension == 0 || dimension > MAX_DIMENSION || bytes.len() != dimension * 4 {
        return Err("保存された声の特徴量の長さが不正です".into());
    }
    let values: Vec<_> = bytes
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let normalized = normalize(&values)?;
    if values
        .iter()
        .zip(&normalized)
        .any(|(a, b)| (a - b).abs() > 1e-5)
    {
        return Err("保存された声の特徴量が正規化されていません".into());
    }
    Ok(normalized)
}
pub(crate) fn centroid(values: &[Vec<f32>]) -> Result<Vec<f32>, String> {
    let Some(first) = values.first() else {
        return Err("登録した声がありません".into());
    };
    let mut sum = vec![0.0f64; first.len()];
    for value in values {
        if value.len() != sum.len() {
            return Err("声の特徴量の次元が一致しません".into());
        }
        for (s, v) in sum.iter_mut().zip(normalize(value)?) {
            *s += f64::from(v);
        }
    }
    normalize(
        &sum.into_iter()
            .map(|v| (v / values.len() as f64) as f32)
            .collect::<Vec<_>>(),
    )
}
pub(crate) fn similarity(a: &[f32], b: &[f32]) -> Result<f64, String> {
    if a.len() != b.len() {
        return Err("声の特徴量の次元が一致しません".into());
    }
    Ok(normalize(a)?
        .iter()
        .zip(normalize(b)?)
        .map(|(a, b)| f64::from(*a) * f64::from(b))
        .sum::<f64>()
        .clamp(-1.0, 1.0))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Window {
    pub speaker_id: String,
    pub track_id: String,
    pub start_us: i64,
    pub end_us: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Plan {
    pub schema_version: u32,
    pub engine: SherpaConfig,
    pub extractor: embedding::ExtractorConfig,
    pub book_hash: String,
    pub working_hash: String,
    pub audio_manifest_hash: String,
    pub person_id: Option<String>,
    pub person_name: Option<String>,
    pub windows: Vec<Window>,
}

pub(crate) fn engine(storage: &Storage) -> Result<SherpaConfig, String> {
    if !cfg!(target_os = "macos") {
        return Err("声の登録・照合は現在macOSのみ対応しています".into());
    }
    let preferences = storage.diarization_preferences()?;
    let e = preferences.payload.standard;
    if e.executable_path.is_empty()
        || e.embedding_model_path.is_empty()
        || e.segmentation_model_path.is_empty()
    {
        return Err("ローカルAI設定で標準プロファイルの話者分離モデルを準備してください".into());
    }
    let capture = |path: &str| Fingerprint::capture(std::path::Path::new(path), &|| false);
    let engine = SherpaConfig {
        executable: capture(&e.executable_path)?,
        segmentation_model: capture(&e.segmentation_model_path)?,
        embedding_model: capture(&e.embedding_model_path)?,
        runtime_libraries: e
            .runtime_library_paths
            .iter()
            .map(|p| capture(p))
            .collect::<Result<_, _>>()?,
        threads: e.threads,
        cluster_threshold_milli: e.cluster_threshold_milli,
        window_shift_milli: e.window_shift_milli,
        min_duration_on_ms: e.min_duration_on_ms,
        min_duration_off_ms: e.min_duration_off_ms,
        speech_sensitivity_milli: e.speech_sensitivity_milli,
    };
    engine.validate_settings(None)?;
    Ok(engine)
}

pub(crate) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    let (spec, _) = storage.fixed_job_input(job_id)?;
    let plan: Plan =
        serde_json::from_value(spec.parameters["voice"].clone()).map_err(|e| e.to_string())?;
    if !matches!(plan.schema_version, 1 | 2)
        || !matches!(
            spec.kind,
            JobKind::VoiceRegistration | JobKind::VoiceMatching
        )
        || plan.windows.len() > 200
    {
        return Err("声の処理条件が不正です".into());
    }
    let cancelled = || {
        stopping()
            || storage
                .get_job(job_id)
                .map_or(true, |j| j.state != "running")
    };
    let result = (|| {
        storage.check_voice_plan(&spec, &plan)?;
        crate::storage::model_profiles::jobs::verify(
            &spec.profile_snapshot,
            crate::storage::model_profiles::jobs::Execution::Sherpa(&plan.engine),
            &cancelled,
        )?;
        let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
        let mut features = Vec::new();
        for (i, window) in plan.windows.iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let dir = temporary.path().join(i.to_string());
            std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
            let mut audio = storage.prepare_track_audio_cancellable(
                &spec.meeting_id,
                &window.track_id,
                window.start_us,
                window.end_us,
                &cancelled,
            )?;
            let wave = dir.join("input.wav");
            let frames = crate::transcription::wave::write_track(&mut audio, &wave, &cancelled)?;
            let fingerprint = Fingerprint::capture(&wave, &cancelled)?;
            let Some(output) = embedding::run(
                &plan.extractor,
                &plan.engine,
                &fingerprint,
                0,
                frames,
                &dir,
                Duration::from_secs(300),
                &cancelled,
            )?
            else {
                return Ok(None);
            };
            for span in &audio.spans {
                span.audio.check_unchanged()?;
            }
            if let Some(vector) = output.vector(0, frames)? {
                features.push((window.clone(), normalize(&vector)?));
            }
            // Resume starts this bounded extraction again; no partially extracted vectors survive.
            progress(storage.get_job(job_id)?);
        }
        if cancelled() {
            return Ok(None);
        }
        storage.check_voice_plan(&spec, &plan)?;
        plan.engine.verify(&cancelled)?;
        let job = storage.publish_voice_result(&spec, &plan, features, job_id)?;
        progress(job);
        Ok(Some(spec.meeting_id.clone()))
    })();
    if storage.get_job(job_id)?.state == "succeeded" {
        return result;
    }
    if cancelled() {
        let job = storage.get_job(job_id)?;
        if matches!(job.state.as_str(), "running" | "pausing") {
            if job.state == "running" {
                storage.request_job_pause(job_id, "exit")?;
            }
            progress(storage.save_job_checkpoint(
                job_id,
                job.completed_units,
                job.completed_units,
                serde_json::json!({"voiceResult":false}),
            )?);
        }
        return Ok(None);
    }
    result
}

pub(crate) fn retry(
    storage: &Storage,
    job_id: &str,
    operation: &str,
) -> Result<JobSnapshot, String> {
    let job = storage.get_job(job_id)?;
    let (spec, checkpoint) = storage.fixed_job_input(job_id)?;
    let plan: Plan =
        serde_json::from_value(spec.parameters["voice"].clone()).map_err(|e| e.to_string())?;
    storage.check_voice_plan(&spec, &plan)?;
    storage.enqueue_retry_job(&job, &spec, checkpoint.as_ref(), operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vectors_reject_corruption_and_preserve_f32_bytes() {
        let bytes = encode(&[3., 4.]).unwrap();
        assert_eq!(decode(&bytes, 2).unwrap(), vec![0.6, 0.8]);
        assert!(decode(&bytes, 3).is_err());
        assert!(encode(&[f32::NAN]).is_err());
        assert!(encode(&[0., 0.]).is_err());
        assert!(decode(&[0; 8], 2).is_err());
        assert!(normalize(&vec![1.; 4097]).is_err());
    }
    #[test]
    fn centroid_and_cosine_do_not_treat_opposite_or_different_dimensions_as_identity() {
        assert!(centroid(&[vec![1., 0.], vec![-1., 0.]]).is_err());
        assert_eq!(
            centroid(&[vec![3., 0.], vec![2., 0.]]).unwrap(),
            vec![1., 0.]
        );
        assert_eq!(similarity(&[1., 0.], &[0., 1.]).unwrap(), 0.);
        assert!(similarity(&[1.], &[1., 0.]).is_err());
    }
}

// Called only after a newly analyzed meeting becomes current, never after
// enrollment or registry changes (old meetings are explicitly re-matched).
pub(crate) fn enqueue_automatic(storage: &Storage, meeting: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") || storage.voice_people()?.iter().all(|p| p.samples.is_empty()) {
        return Ok(());
    }
    let context = storage.voice_context(meeting)?;
    if context.windows.is_empty() || storage.has_voice_matching(&context.analysis_id)? {
        return Ok(());
    }
    storage.start_voice(crate::storage::voice::Start {
        meeting_id: meeting.into(),
        operation_id: uuid::Uuid::new_v4().to_string(),
        analysis_id: context.analysis_id,
        working_hash: context.working_hash,
        book_hash: context.book_hash,
        speaker_id: None,
        person_id: None,
        person_name: None,
        selected_windows: None,
    })?;
    Ok(())
}
