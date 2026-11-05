use super::*;
use crate::local_engine::Fingerprint;
use std::collections::BTreeMap;

fn proof(path: &str, index: usize) -> Fingerprint {
    Fingerprint {
        path: path.into(),
        size: (index + 1) as u64,
        sha256: format!("{index:064x}"),
    }
}

fn fixture(id: &str) -> (AsrConfig, Value) {
    let descriptor = asr::find(id).unwrap().descriptor();
    let files = descriptor
        .slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let name = match *slot {
                "decoder" => "decoder.onnx",
                "joiner" => "joiner.onnx",
                "library" => "sherpa-library",
                "tokens" => "tokens.txt",
                _ => unreachable!(),
            };
            (
                slot.to_string(),
                proof(&format!("/source/models/{name}"), index + 2),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let model = proof("/source/models/encoder.onnx", 1);
    let models = std::iter::once(("model".to_owned(), model.path.clone()))
        .chain(
            files
                .iter()
                .map(|(slot, file)| (slot.clone(), file.path.clone())),
        )
        .collect::<BTreeMap<_, _>>();
    let config = AsrConfig {
        executable: proof("/source/worker", 0),
        model,
        vad_model: None,
        vad_options: None,
        threads: 1,
        use_gpu: false,
        prompt: "ミライ".into(),
        backend: Some(asr::BackendBinding {
            id: id.into(),
            files,
        }),
    };
    let result = json!({
        "schemaVersion": 1, "backendId": id, "models": models, "durationUs": 1_000_000,
        "segments": [{"startUs":100_000,"endUs":900_000,"text":"ミライに連絡してください"}],
        "tokens": [{"startUs":100_000,"text":"ミライ"}],
        "ignoredHotwords": ["未対応"],
    });
    (config, result)
}

fn unavailable(mut config: AsrConfig) -> AsrConfig {
    config.executable.path = format!("mon-unavailable:{}", config.executable.sha256).into();
    config.model.path = format!("mon-unavailable:{}", config.model.sha256).into();
    for file in config.backend.as_mut().unwrap().files.values_mut() {
        file.path = format!("mon-unavailable:{}", file.sha256).into();
    }
    config
}

#[test]
fn asr_saved_result_rebinds_all_verified_aliases_and_roundtrips_without_models() {
    let (original, mut raw) = fixture(asr::TRANSDUCER);
    let aliases = std::iter::once(("model", &original.model))
        .chain(
            original
                .backend
                .as_ref()
                .unwrap()
                .files
                .values()
                .map(|file| ("artifact", file)),
        )
        .map(|(role, file)| {
            (
                (role.to_owned(), file.size, file.sha256.clone()),
                file.path.file_name().unwrap().to_str().unwrap().to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let aliases = |file: &Fingerprint, role: &str| {
        aliases
            .get(&(role.to_owned(), file.size, file.sha256.clone()))
            .cloned()
            .into_iter()
            .collect()
    };
    for path in raw["models"].as_object_mut().unwrap().values_mut() {
        *path = json!(path.as_str().unwrap().rsplit('/').next().unwrap());
    }
    let portable = unavailable(original);
    let bytes = serde_json::to_vec(&raw).unwrap();
    let (before, after) = restore_asr_result(
        &bytes,
        &portable,
        &portable,
        &["encoder.onnx".into()],
        1_000_000,
        &aliases,
    )
    .unwrap();
    assert_eq!(before, after);
    let rebuilt: Value = serde_json::from_slice(&after).unwrap();
    assert_eq!(
        rebuilt["models"].as_object().unwrap().len(),
        raw["models"].as_object().unwrap().len()
    );
    for path in rebuilt["models"].as_object().unwrap().values() {
        assert!(path.as_str().unwrap().starts_with("mon-unavailable:"));
    }
    for field in ["segments", "tokens", "ignoredHotwords"] {
        assert_eq!(rebuilt[field], raw[field]);
    }
    let (_, again) = restore_asr_result(
        &after,
        &portable,
        &portable,
        &["encoder.onnx".into()],
        1_000_000,
        &aliases,
    )
    .unwrap();
    assert_eq!(
        again, after,
        "rebackup/rerestore must retain path-free results"
    );
}

#[test]
fn asr_saved_result_refuses_unbound_dependencies_wrong_backend_and_invalid_times() {
    let (config, raw) = fixture(asr::TRANSDUCER);
    for corruption in [
        "dependency",
        "missing-slot",
        "extra-slot",
        "backend",
        "time",
    ] {
        let mut value = raw.clone();
        match corruption {
            "dependency" => value["models"]["decoder"] = json!("foreign.onnx"),
            "missing-slot" => {
                value["models"].as_object_mut().unwrap().remove("tokens");
            }
            "extra-slot" => value["models"]["extra"] = json!("undeclared.onnx"),
            "backend" => value["backendId"] = json!(asr::WHISPER),
            "time" => value["segments"][0]["endUs"] = json!(2_000_000),
            _ => unreachable!(),
        }
        assert!(
            restore_asr_result(
                &serde_json::to_vec(&value).unwrap(),
                &config,
                &config,
                &[],
                1_000_000,
                &|_, _| vec![]
            )
            .is_err(),
            "{corruption}"
        );
    }
    let portable = unavailable(config);
    let mut redacted = raw;
    for path in redacted["models"].as_object_mut().unwrap().values_mut() {
        *path = json!(path.as_str().unwrap().rsplit('/').next().unwrap());
    }
    assert!(
        restore_asr_result(
            &serde_json::to_vec(&redacted).unwrap(),
            &portable,
            &portable,
            &["encoder.onnx".into()],
            1_000_000,
            &|_, _| vec![]
        )
        .is_err(),
        "a portable auxiliary basename needs exact size/SHA/role metadata"
    );
}
