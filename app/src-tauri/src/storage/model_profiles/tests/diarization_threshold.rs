use super::*;

fn tuning(f: &Fixture, index: usize, threshold: u32) -> Result<ApplyPreview, String> {
    let package = &f.packages[index];
    crate::model_application::preview_with_tuning(
        &f.storage,
        &f.models(),
        ProfileName::Standard,
        &package.id,
        &package.record.review_hash,
        &package.check,
        None,
        Some(threshold),
        &crate::test_support::never_cancelled,
    )
}

#[test]
fn automatic_merge_tuning_is_bound_atomic_and_keeps_started_jobs_and_other_settings() {
    let mut f = Fixture::new();
    let index = f.add(Engine::Sherpa, true, false);
    f.apply(index, ProfileName::Standard);
    let before = f.storage.diarization_preferences().unwrap();
    let fixed = f.storage.fixed_job_input(&f.baseline.id).unwrap();
    let calls = f.executed(index);
    let preview = tuning(&f, index, 1300).unwrap();
    let operation = Uuid::new_v4().to_string();
    let apply = || {
        crate::model_application::apply(
            &f.storage,
            &f.models(),
            &operation,
            &preview,
            &crate::test_support::never_cancelled,
        )
    };
    let first = apply().unwrap();
    assert_eq!(apply().unwrap(), first);
    let mut expected = before.payload.clone();
    expected.standard.cluster_threshold_milli = 1300;
    let after = f.storage.diarization_preferences().unwrap();
    assert_eq!(after.payload, expected);
    assert_eq!(after.lock_version, before.lock_version + 1);
    assert!(first
        .components
        .iter()
        .all(|c| c.settings_match && c.compatibility_current));
    assert_eq!(f.executed(index), calls);
    assert_eq!(f.storage.fixed_job_input(&f.baseline.id).unwrap(), fixed);
    let mut changed = preview.clone();
    changed.diarization_cluster_threshold_milli = Some(1500);
    if let Settings::Sherpa(p) = &mut changed.after {
        p.cluster_threshold_milli = 1500;
    }
    changed.review_digest.clear();
    changed.review_digest = digest(&changed).unwrap();
    assert!(crate::model_application::apply(
        &f.storage,
        &f.models(),
        &operation,
        &changed,
        &crate::test_support::never_cancelled
    )
    .is_err());
    let back = tuning(&f, index, before.payload.standard.cluster_threshold_milli).unwrap();
    crate::model_application::apply(
        &f.storage,
        &f.models(),
        &Uuid::new_v4().to_string(),
        &back,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        f.storage.diarization_preferences().unwrap().payload,
        before.payload
    );
}

#[test]
fn automatic_merge_tuning_rejects_unselected_models_forgery_stale_input_and_cancellation() {
    let mut f = Fixture::new();
    let index = f.add(Engine::Sherpa, true, false);
    assert!(tuning(&f, index, 1300).is_err());
    f.apply(index, ProfileName::Standard);
    for threshold in [2001, u32::MAX] {
        assert!(tuning(&f, index, threshold).is_err());
    }
    let other = f.add(Engine::Sherpa, true, false);
    assert!(tuning(&f, other, 1300).is_err());
    let whisper = f.add(Engine::Whisper, true, false);
    f.apply(whisper, ProfileName::Standard);
    assert!(tuning(&f, whisper, 1300).is_err());
    for mutation in [
        "threshold",
        "threads",
        "duration",
        "model",
        "stale",
        "cancel",
    ] {
        let mut preview = tuning(&f, index, 1300).unwrap();
        if let Settings::Sherpa(p) = &mut preview.after {
            match mutation {
                "threshold" => p.cluster_threshold_milli = 1500,
                "threads" => p.threads += 1,
                "duration" => p.min_duration_on_ms = 300,
                "model" => p.embedding_model_path.push_str("-other"),
                _ => (),
            }
        }
        preview.review_digest.clear();
        preview.review_digest = digest(&preview).unwrap();
        let before = f.storage.diarization_preferences().unwrap();
        if mutation == "stale" {
            let mut payload = before.payload.clone();
            payload.high_accuracy.threads += 1;
            f.storage
                .save_diarization_preferences(before.lock_version, payload)
                .unwrap();
        }
        let current = f.storage.diarization_preferences().unwrap();
        assert!(
            crate::model_application::apply(
                &f.storage,
                &f.models(),
                &Uuid::new_v4().to_string(),
                &preview,
                &|| mutation == "cancel"
            )
            .is_err(),
            "{mutation}"
        );
        assert_eq!(
            f.storage.diarization_preferences().unwrap().payload,
            current.payload
        );
    }
}
