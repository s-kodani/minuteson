use super::*;
#[cfg(windows)]
#[test]
#[ignore = "requires SHA-verified real models in MON_WINDOWS_PARITY_ASSETS"]
fn windows_real_llama_vulkan_trial_and_cpu_fallback() {
    use crate::local_engine::Fingerprint;
    let index: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var_os("MON_WINDOWS_PARITY_ASSETS").unwrap()).unwrap(),
    )
    .unwrap();
    let value = index
        .as_object()
        .unwrap()
        .values()
        .filter(|v| v["role"] == "llm")
        .min_by_key(|v| v["bytes"].as_u64().unwrap())
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    let snapshot = crate::local_engine::resident::prepare_installed(&path, "llama")
        .unwrap()
        .unwrap();
    let config = LlamaConfig {
        executable: Fingerprint::capture(
            &crate::windows_runtime::resources()
                .unwrap()
                .join("llama/llama-completion.exe"),
            &|| false,
        )
        .unwrap(),
        model: Fingerprint::capture(Path::new(value["path"].as_str().unwrap()), &|| false).unwrap(),
        threads: 2,
        context_tokens: 2048,
        prediction_tokens: 64,
        gpu_layers: 0,
        seed: 42,
    };
    {
        let scope = crate::local_engine::resident::Scope::enter(&path, Some(&snapshot)).unwrap();
        let directory = path.join("gpu");
        std::fs::create_dir(&directory).unwrap();
        let resolved = resolve(Policy::AutoVulkanV1, &config, 2048, &directory, &|| false)
            .unwrap()
            .unwrap();
        resolved
            .validate(Policy::AutoVulkanV1, &config, 2048)
            .unwrap();
        eprintln!(
            "Actual Vulkan trial: {}",
            serde_json::to_string(&resolved).unwrap()
        );
        if matches!(resolved, Resolved::Vulkan { .. }) {
            let directory = path.join("gpu-followup");
            std::fs::create_dir(&directory).unwrap();
            let mut selected = config.clone();
            let device = resolved.configure(&mut selected);
            assert_eq!(llama::run_unit_observed_on_device(&selected,&directory,SYSTEM,PROMPT,&json!({"type":"object","properties":{"ok":{"type":"boolean","const":true}},"required":["ok"],"additionalProperties":false}),Duration::from_secs(60),&||false,&|_|{},device).unwrap().unwrap(),json!({"ok":true}));
            let log = std::fs::read_to_string(directory.join("stderr.log")).unwrap();
            assert!(log.contains("reusing model"));
            assert!(!log.contains("mon_model_load: llama=1"));
        }
        drop(scope);
    }
    let mut cpu = snapshot;
    cpu.gpu = None;
    let _scope = crate::local_engine::resident::Scope::enter(&path, Some(&cpu)).unwrap();
    let directory = path.join("forced-failure");
    std::fs::create_dir(&directory).unwrap();
    let resolved = resolve(Policy::AutoVulkanV1, &config, 2048, &directory, &|| false)
        .unwrap()
        .unwrap();
    assert!(matches!(
        resolved,
        Resolved::Cpu {
            reason: CpuReason::TrialFailed
        }
    ));
    let directory = path.join("cpu");
    std::fs::create_dir(&directory).unwrap();
    assert_eq!(llama::run_unit_observed_on_device(&config,&directory,SYSTEM,PROMPT,&json!({"type":"object","properties":{"ok":{"type":"boolean","const":true}},"required":["ok"],"additionalProperties":false}),Duration::from_secs(60),&||false,&|_|{},None).unwrap().unwrap(),json!({"ok":true}));
}
fn config() -> LlamaConfig {
    use crate::local_engine::Fingerprint;
    LlamaConfig {
        executable: Fingerprint {
            path: "/unavailable/llama-completion".into(),
            size: 33472,
            sha256: "e3d5205729cb4c1e176d5c8e39796aefbe361a2300d0593586db9503dd0d63d1".into(),
        },
        model: Fingerprint {
            path: "/unavailable/model.gguf".into(),
            size: 5_027_783_488,
            sha256: "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785".into(),
        },
        threads: 4,
        context_tokens: 8192,
        prediction_tokens: 2048,
        gpu_layers: 0,
        seed: 1,
    }
}
fn choice() -> Resolved {
    let c = config();
    Resolved::Metal { proof:Proof { schema_version:2, engine_sha256:c.executable.sha256,model_sha256:c.model.sha256,context_tokens:40960,gpu_layers:256,device:DEVICE.into(),result:"{\"ok\":true}\n[end of text]".into(),log:format!("llama_prepare_model_devices: using device MTL0 (Apple M1 Max)\nload_tensors: offloaded 256/256 layers to GPU\n") } }
}
#[test]
fn exact_bytes_and_separate_positive_gpu_proof_are_required() {
    let c = config();
    let valid = choice();
    assert!(Policy::AutoMetalV2.accepts(&c));
    valid.validate(Policy::AutoMetalV2, &c, 40960).unwrap();
    let mut other = c.clone();
    other.model.sha256 = "a".repeat(64);
    assert!(Policy::AutoMetalV2.accepts(&other));
    assert!(valid.validate(Policy::AutoMetalV2, &other, 40960).is_err());
    assert!(valid.validate(Policy::AutoMetalV2, &c, 8192).is_err());
    for kind in [
        "cpu-log",
        "device",
        "layers",
        "truncated",
        "duplicate",
        "oversized",
        "engine",
    ] {
        let mut invalid = choice();
        let Resolved::Metal { proof } = &mut invalid else {
            unreachable!()
        };
        match kind {
            "cpu-log" => {
                proof.log = "found device MTL0\nload_tensors: offloaded 0/37 layers to GPU".into()
            }
            "device" => proof.device = "MTL1".into(),
            "layers" => proof.gpu_layers = 1,
            "truncated" => proof.result = "{\"ok\":".into(),
            "duplicate" => proof.result = "{\"ok\":false,\"ok\":true}".into(),
            "oversized" => proof.log.push_str(&"x".repeat(LOG_LIMIT)),
            _ => proof.engine_sha256 = "b".repeat(64),
        }
        assert!(
            invalid.validate(Policy::AutoMetalV2, &c, 40960).is_err(),
            "{kind}"
        );
    }
}
#[test]
fn pinned_choices_roundtrip_without_paths_or_implicit_cpu_switches() {
    let valid = choice();
    let value = serde_json::to_value(&valid).unwrap();
    let restored: Resolved = serde_json::from_value(value).unwrap();
    let mut runtime = config();
    assert_eq!(restored.configure(&mut runtime), Some("MTL0"));
    assert_eq!(runtime.gpu_layers, 256);
    assert_eq!(config().gpu_layers, 0);
    for reason in [CpuReason::Memory, CpuReason::TrialFailed] {
        let cpu = Resolved::Cpu { reason };
        cpu.validate(Policy::AutoMetalV2, &config(), 40960).unwrap();
        let mut runtime = config();
        assert!(cpu.configure(&mut runtime).is_none());
        assert_eq!(runtime.gpu_layers, 0);
    }
    assert!(resolve(
        Policy::AutoMetalV2,
        &config(),
        40960,
        Path::new("/unavailable"),
        &|| true
    )
    .unwrap()
    .is_none());
}

#[test]
fn every_catalog_model_and_other_model_bytes_are_eligible_for_auto_metal() {
    let choices: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../model-catalog/macos.json")).unwrap();
    let mut count = 0;
    for choice in choices
        .iter()
        .filter(|c| c["recipe"]["manifest"]["engine"] == "llama.cpp")
    {
        let mut c = config();
        let files = choice["recipe"]["manifest"]["files"].as_array().unwrap();
        let model = files.iter().find(|f| f["role"] == "llm").unwrap();
        c.model.sha256 = model["expectedHash"].as_str().unwrap().into();
        c.model.size = model["expectedSize"].as_u64().unwrap();
        assert!(Policy::AutoMetalV2.accepts(&c), "{}", choice["label"]);
        if cfg!(target_os = "macos") {
            assert_eq!(policy(&c), Some(Policy::AutoMetalV2));
        }
        count += 1;
    }
    assert_eq!(count, 5);
    let mut c = config();
    c.model.sha256 = "a".repeat(64);
    c.model.size = 123;
    c.executable.sha256 = "b".repeat(64);
    c.executable.size = 456;
    assert!(Policy::AutoMetalV2.accepts(&c));
    let mut proof = choice();
    let Resolved::Metal { proof: p } = &mut proof else {
        unreachable!()
    };
    p.schema_version = 2;
    p.model_sha256 = c.model.sha256.clone();
    p.engine_sha256 = c.executable.sha256.clone();
    proof.validate(Policy::AutoMetalV2, &c, 40960).unwrap();
    let Resolved::Metal { proof: p } = &mut proof else {
        unreachable!()
    };
    p.schema_version = 1;
    assert!(proof.validate(Policy::AutoMetalV2, &c, 40960).is_err());
    c.gpu_layers = 12;
    assert!(!Policy::AutoMetalV2.accepts(&c));
}

#[cfg(unix)]
fn automatic_config(root: &Path, script: &str) -> LlamaConfig {
    use std::os::unix::fs::PermissionsExt;
    let executable = root.join("engine");
    let model = root.join("model.gguf");
    std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&model, b"different model bytes").unwrap();
    let mut c = config();
    c.executable = crate::local_engine::Fingerprint::capture(
        &executable,
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    c.model =
        crate::local_engine::Fingerprint::capture(&model, &crate::test_support::never_cancelled)
            .unwrap();
    c
}
#[test]
#[cfg(unix)]
fn auto_trial_pins_actual_model_layers_and_works_below_the_old_memory_floor() {
    let root = tempfile::tempdir().unwrap();
    let c = automatic_config(
        root.path(),
        r#"printf '%s\n' "$@" > args.log
printf '%s\n' 'using device MTL0 (Apple GPU)' 'load_tensors: offloaded 29/29 layers to GPU' >&2
printf '%s' '{"ok":true}'"#,
    );
    let resolved = resolve(
        Policy::AutoMetalV2,
        &c,
        8192,
        root.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .unwrap();
    resolved.validate(Policy::AutoMetalV2, &c, 8192).unwrap();
    let mut runtime = c.clone();
    assert_eq!(resolved.configure(&mut runtime), Some("MTL0"));
    assert_eq!(runtime.gpu_layers, 29);
    let value = serde_json::to_value(resolved).unwrap();
    assert_eq!(value["proof"]["schemaVersion"], 2);
    let args = std::fs::read_to_string(root.path().join("args.log")).unwrap();
    assert!(args.contains("--gpu-layers\n256\n"));
    assert!(!root.path().join("gpu-trial-1").exists());
}
#[test]
#[cfg(unix)]
fn auto_trial_reduces_layers_after_gpu_failure_and_pins_the_successful_partial_offload() {
    let root = tempfile::tempdir().unwrap();
    let c = automatic_config(
        root.path(),
        r#"while [ "$1" != '--gpu-layers' ]; do shift; done
shift
printf '%s\n' 'using device MTL0 (Apple GPU)' >&2
if [ "$1" = 256 ]; then printf '%s\n' 'failed to allocate buffer' >&2; exit 1; fi
[ "$1" = 32 ] || exit 2
printf '%s\n' 'load_tensors: offloaded 32/37 layers to GPU' >&2
printf '%s' '{"ok":true}'"#,
    );
    let resolved = resolve(
        Policy::AutoMetalV2,
        &c,
        8192,
        root.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .unwrap();
    let mut runtime = c.clone();
    assert_eq!(resolved.configure(&mut runtime), Some("MTL0"));
    assert_eq!(runtime.gpu_layers, 32);
    resolved.validate(Policy::AutoMetalV2, &c, 8192).unwrap();
    assert!(root.path().join("gpu-trial-1/performance.json").is_file());
    assert!(!root.path().join("gpu-trial-2").exists());
}
#[test]
#[cfg(unix)]
fn auto_trial_without_device_uses_cpu_and_cancellation_never_pins_cpu() {
    let root = tempfile::tempdir().unwrap();
    let c = automatic_config(root.path(), "printf 'no GPU device' >&2; exit 1");
    let resolved = resolve(
        Policy::AutoMetalV2,
        &c,
        8192,
        root.path(),
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        resolved,
        Resolved::Cpu {
            reason: CpuReason::TrialFailed
        }
    ));
    assert!(!root.path().join("gpu-trial-1").exists());
    let root = tempfile::tempdir().unwrap();
    let c = automatic_config(
        root.path(),
        "touch stopped; printf 'using device MTL0\\n' >&2; /bin/sleep 2; exit 1",
    );
    assert!(resolve(Policy::AutoMetalV2, &c, 8192, root.path(), &|| root
        .path()
        .join("stopped")
        .exists(),)
    .unwrap()
    .is_none());
    assert!(!root.path().join("gpu-trial-1").exists());
}
