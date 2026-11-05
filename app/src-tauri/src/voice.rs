//! Meeting-independent voice identities. Embeddings never establish authentication.
use crate::diarization::{embedding, sherpa::SherpaConfig};
use crate::storage::{JobKind, JobSnapshot, Storage};
use serde::{Deserialize, Serialize};
use std::time::Duration;
pub(crate) mod result;

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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MatchingPolicy {
    source: String,
    version: String,
    pub min_query_duration_us: i64,
}
impl MatchingPolicy {
    pub(crate) fn fixed() -> Self {
        Self {
            source: "user_settings".into(),
            version: "user_thresholds_v1".into(),
            min_query_duration_us: 3_000_000,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resident_runtime: Option<crate::local_engine::resident::Snapshot>,
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matching_options: Option<crate::storage::voice_preferences::VoiceMatchingPreferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matching_policy: Option<MatchingPolicy>,
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
    if !cfg!(any(target_os = "macos", windows)) {
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
    let engine = e.capture(crate::diarization::sherpa::CaptureMode::Transient, &|| {
        false
    })?;
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
    if plan.schema_version != crate::current_formats::VOICE
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
    storage.clear_completion_progress(job_id);
    struct ProgressScope<'a>(&'a Storage, &'a str);
    impl Drop for ProgressScope<'_> {
        fn drop(&mut self) {
            self.0.clear_completion_progress(self.1);
        }
    }
    let _progress_scope = ProgressScope(storage, job_id);
    let speaker_count = plan
        .windows
        .iter()
        .map(|w| &w.speaker_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let total = plan.windows.len() + speaker_count + 1;
    let update = |phase: &str,
                  completed: usize,
                  phase_completed: usize,
                  phase_total: usize|
     -> Result<(), String> {
        progress(storage.set_completion_progress(
            job_id,
            crate::storage::ExecutionProgress {
                phase: phase.into(),
                completed,
                total,
                phase_completed,
                phase_total,
            },
        )?);
        Ok(())
    };
    let result = (|| {
        update("preparing", 0, 0, 0)?;
        storage.check_voice_plan(&spec, &plan)?;
        crate::storage::model_profiles::jobs::verify(
            &spec.profile_snapshot,
            crate::storage::model_profiles::jobs::Execution::Sherpa(&plan.engine),
            &cancelled,
        )?;
        let source = storage.recorded_analysis_input(&spec.meeting_id)?.0;
        let mut feature_session = crate::speaker_features::Session::new(
            storage,
            &spec.meeting_id,
            &plan.engine,
            &plan.extractor,
            &source,
            plan.resident_runtime.as_ref(),
            crate::internal_logs::Context {
                category: if spec.kind == JobKind::VoiceRegistration {
                    "voice_registration"
                } else {
                    "voice_matching"
                }
                .into(),
                meeting_id: Some(spec.meeting_id.clone()),
                job_id: Some(job_id.into()),
                operation_id: Some(spec.operation_id.clone()),
            },
            &cancelled,
        )?;
        let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
        let mut features = Vec::new();
        for (i, window) in plan.windows.iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let dir = temporary.path().join(i.to_string());
            if let Some(vector) = feature_session.extract(window, &dir, &cancelled)? {
                features.push((window.clone(), normalize(&vector)?));
            }
            update("references", i + 1, i + 1, plan.windows.len())?;
        }
        feature_session.finish(&cancelled)?;
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
