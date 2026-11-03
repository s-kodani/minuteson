use super::*;
use crate::transcription::vad::VadOptions;
fn tuning(f: &Fixture, index: usize, options: VadOptions) -> Result<ApplyPreview, String> {
    let p = &f.packages[index];
    crate::model_application::preview_with_vad(
        &f.storage,
        &f.models(),
        ProfileName::Standard,
        &p.id,
        &p.record.review_hash,
        &p.check,
        None,
        None,
        Some(options),
        &|| false,
    )
}
#[test]
fn vad_tuning_is_atomic_and_keeps_fixed_jobs_and_other_settings() {
    let mut f = Fixture::new();
    let i = f.add(Engine::Whisper, true, false);
    let options = VadOptions {
        enabled: false,
        threshold_milli: 750,
        min_speech_duration_ms: 450,
        ..Default::default()
    };
    assert!(tuning(&f, i, options).is_err());
    f.apply(i, ProfileName::Standard);
    let before = f.storage.analysis_preferences().unwrap();
    let fixed = f.storage.fixed_job_input(&f.baseline.id).unwrap();
    let calls = f.executed(i);
    let preview = tuning(&f, i, options).unwrap();
    let id = Uuid::new_v4().to_string();
    let apply =
        || crate::model_application::apply(&f.storage, &f.models(), &id, &preview, &|| false);
    let first = apply().unwrap();
    assert_eq!(apply().unwrap(), first);
    let mut expected = before.payload;
    expected.standard.vad_options = Some(options);
    let after = f.storage.analysis_preferences().unwrap();
    assert_eq!(after.payload, expected);
    assert_eq!(after.lock_version, before.lock_version + 1);
    assert_eq!(f.storage.fixed_job_input(&f.baseline.id).unwrap(), fixed);
    assert_eq!(f.executed(i), calls);
    let next = f.add(Engine::Whisper, true, false);
    f.apply(next, ProfileName::Standard);
    assert_eq!(
        f.storage
            .analysis_preferences()
            .unwrap()
            .payload
            .standard
            .vad_options,
        Some(options)
    );
    let exported = crate::storage::model_profiles::portable::export(
        &f.storage.connection.lock().unwrap(),
        &|| false,
    )
    .unwrap();
    let json = serde_json::to_value(&exported).unwrap();
    assert!(json.to_string().contains("\"enabled\":false"));
    let decoded: crate::storage::model_profiles::portable::Catalog =
        serde_json::from_value(json.clone()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), json);
    // New jobs seal both the disabled state and all numbers. A real archive
    // restore keeps that state in portable history without selecting models.
    let p = f.storage.analysis_preferences().unwrap().payload.standard;
    let job = crate::transcription::StartRequest {
        meeting_id: f.meeting.clone(),
        operation_id: Uuid::new_v4().to_string(),
        executable_path: p.executable_path.into(),
        model_path: p.model_path.into(),
        vad_model_path: p.vad_model_path.map(Into::into),
        vad_options: p.vad_options,
        threads: p.threads,
        use_gpu: p.use_gpu,
        profile: "standard".into(),
        prompt: String::new(),
        expected_vocabulary: None,
    }
    .enqueue(&f.storage)
    .unwrap();
    f.storage.finish_job(&job.id, 0, "cancelled", None).unwrap();
    let frozen = f.storage.fixed_job_input(&job.id).unwrap();
    assert_eq!(
        frozen.0.parameters["engine"]["vadOptions"]["enabled"],
        false
    );
    let on = tuning(
        &f,
        next,
        VadOptions {
            enabled: true,
            ..options
        },
    )
    .unwrap();
    crate::model_application::apply(
        &f.storage,
        &f.models(),
        &Uuid::new_v4().to_string(),
        &on,
        &|| false,
    )
    .unwrap();
    assert_eq!(f.storage.fixed_job_input(&job.id).unwrap(), frozen);
    let mut changed = preview.clone();
    changed
        .whisper_vad_options
        .as_mut()
        .unwrap()
        .threshold_milli = 800;
    changed.review_digest.clear();
    changed.review_digest = digest(&changed).unwrap();
    assert!(crate::model_application::apply(
        &f.storage,
        &f.models(),
        &Uuid::new_v4().to_string(),
        &changed,
        &|| false
    )
    .is_err());
}
#[test]
fn vad_tuning_rejects_foreign_engine_invalid_ranges_and_cancellation() {
    let mut f = Fixture::new();
    let i = f.add(Engine::Sherpa, true, false);
    f.apply(i, ProfileName::Standard);
    assert!(tuning(&f, i, Default::default()).is_err());
    let i = f.add(Engine::Whisper, true, false);
    f.apply(i, ProfileName::Standard);
    assert!(tuning(
        &f,
        i,
        VadOptions {
            min_speech_duration_ms: 2000,
            max_speech_duration_s: 1,
            ..Default::default()
        }
    )
    .is_err());
    let preview = tuning(&f, i, Default::default()).unwrap();
    let before = f.storage.analysis_preferences().unwrap();
    assert!(crate::model_application::apply(
        &f.storage,
        &f.models(),
        &Uuid::new_v4().to_string(),
        &preview,
        &|| true
    )
    .is_err());
    assert_eq!(
        f.storage.analysis_preferences().unwrap().payload,
        before.payload
    );
}
