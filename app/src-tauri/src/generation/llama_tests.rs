use super::*;
use serde_json::json;

#[test]
fn output_is_one_complete_object_not_a_log_or_truncated_completion() {
    for good in [r#"{"items":[]}"#, " \n{\"items\":[]}\n [end of text]\n"] {
        assert_eq!(parse_output(good.as_bytes()).unwrap(), json!({"items":[]}));
    }
    for bad in [
        "",
        "[]",
        "null",
        "{",
        "{\"x\":1}{}",
        "log\n{}",
        "{}log",
        "```json\n{}\n```",
        "<think>reason</think>{}",
        "{}[end of text]oops",
        "{}[end of text][end of text]",
        "{\"items\":[],\"items\":[]}",
        "{\"items\":[{\"body\":\"a\",\"body\":\"b\"}]}",
    ] {
        assert!(parse_output(bad.as_bytes()).is_err(), "{bad}");
    }
    assert!(parse_output(&vec![b' '; MAX_OUTPUT_BYTES as usize + 1]).is_err());
}

#[cfg(unix)]
fn fake(script: &str) -> (tempfile::TempDir, LlamaConfig) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("llama-completion");
    let model = dir.path().join("model.gguf");
    std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&model, b"test GGUF identity").unwrap();
    let config = LlamaConfig {
        executable: Fingerprint::capture(&executable, &|| false).unwrap(),
        model: Fingerprint::capture(&model, &|| false).unwrap(),
        threads: 2,
        context_tokens: 2048,
        prediction_tokens: 256,
        gpu_layers: 0,
        seed: 1,
    };
    (dir, config)
}

#[cfg(unix)]
fn unit(dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::create_dir(&path).unwrap();
    path
}

#[test]
#[cfg(unix)]
fn fixed_arguments_inputs_and_existing_artifacts_are_protected() {
    let (dir, config) = fake("printf '%s\\n' \"$@\" > args.log\n/usr/bin/env > env.log\nprintf '{\"items\":[]}\\n [end of text]\\n'");
    let path = unit(dir.path(), "unit");
    assert_eq!(
        run_unit(
            &config,
            &path,
            "固定指示",
            "入力\\n",
            &json!({"type":"object"}),
            Duration::from_secs(5),
            &|| false
        )
        .unwrap(),
        Some(json!({"items":[]}))
    );
    assert_eq!(
        std::fs::read_to_string(path.join("prompt.txt")).unwrap(),
        "入力\\n"
    );
    let args = std::fs::read_to_string(path.join("args.log")).unwrap();
    for fixed in [
        "--offline",
        "--no-context-shift",
        "--single-turn",
        "--json-schema-file",
        "--no-escape",
        "--ctx-size\n2048",
        "--predict\n256",
        "--gpu-layers\n0",
        "--temp\n0",
    ] {
        assert!(args.contains(fixed), "{fixed}: {args}");
    }
    let env = std::fs::read_to_string(path.join("env.log")).unwrap();
    assert!(!env.contains("LLAMA_ARG_") && !env.contains("HF_TOKEN="));
    let saved = std::fs::read(path.join("result.json")).unwrap();
    assert!(run_unit(
        &config,
        &path,
        "fixed",
        "input",
        &json!({}),
        Duration::from_secs(5),
        &|| false
    )
    .is_err());
    assert_eq!(std::fs::read(path.join("result.json")).unwrap(), saved);
    let mut invalid = config.clone();
    invalid.prediction_tokens = invalid.context_tokens;
    assert!(invalid.verify(&|| false).is_err());
    std::fs::write(&config.model.path, b"changed model").unwrap();
    let second = unit(dir.path(), "changed");
    assert!(run_unit(
        &config,
        &second,
        "fixed",
        "input",
        &json!({}),
        Duration::from_secs(5),
        &|| false
    )
    .is_err());
    assert!(!second.join("result.json").exists());
}

#[test]
#[cfg(unix)]
fn failure_pause_timeout_and_log_overflow_never_return_a_result_and_reap_child() {
    for script in [
        "exit 3",
        "printf '{'",
        "/usr/bin/head -c 5000000 /dev/zero >&2",
    ] {
        let (dir, config) = fake(script);
        let path = unit(dir.path(), "bad");
        assert!(run_unit(
            &config,
            &path,
            "fixed",
            "input",
            &json!({}),
            Duration::from_secs(5),
            &|| false
        )
        .is_err());
    }
    for pause in [true, false] {
        let (dir, config) = fake("printf '%s' \"$$\" > child.pid\nwhile :; do :; done");
        let path = unit(dir.path(), "running");
        let started = Instant::now();
        let result = run_unit(
            &config,
            &path,
            "fixed",
            "input",
            &json!({}),
            Duration::from_millis(250),
            &|| pause && path.join("child.pid").exists(),
        );
        if pause {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.unwrap_err().contains("制限時間"));
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        let pid = std::fs::read_to_string(path.join("child.pid")).unwrap();
        assert!(!Command::new("/bin/kill")
            .args(["-0", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
    }
}

#[test]
#[cfg(unix)]
fn running_engine_cannot_change_fixed_model_or_prompt() {
    for target in ["../model.gguf", "prompt.txt"] {
        let (dir, config) = fake(&format!("printf 'changed' > {target}\nprintf '{{}}'"));
        let path = unit(dir.path(), "changed");
        assert!(run_unit(
            &config,
            &path,
            "fixed",
            "input",
            &json!({}),
            Duration::from_secs(5),
            &|| false
        )
        .is_err());
    }
}

#[test]
#[ignore = "requires MON_LLAMA_COMPLETION and MON_LLAMA_MODEL"]
fn real_llama_completion_runs_local_schema_constrained_json() {
    let config = LlamaConfig {
        executable: Fingerprint::capture(
            Path::new(&std::env::var("MON_LLAMA_COMPLETION").unwrap()),
            &|| false,
        )
        .unwrap(),
        model: Fingerprint::capture(
            Path::new(&std::env::var("MON_LLAMA_MODEL").unwrap()),
            &|| false,
        )
        .unwrap(),
        threads: 4,
        context_tokens: 4096,
        prediction_tokens: 256,
        gpu_layers: 0,
        seed: 1,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut input = crate::minutes_document::tests::input();
    input.analysis.utterances[0].effective_text = "点検日を確認することに決まりました。".into();
    let ids = vec![input.analysis.utterances[0].id.clone()];
    let mut schema = crate::generation::result::schema(&ids).unwrap();
    // A one-item synthetic protocol test, not a recall/quality evaluation.
    schema["properties"]["items"]["minItems"] = json!(1);
    schema["properties"]["items"]["maxItems"] = json!(1);
    let result = run_unit(
        &config,
        dir.path(),
        "会議の明示的な決定事項1件だけをJSONで抽出してください。担当者・期限は推測せずnullにしてください。根拠IDには入力の発言IDを使い、schemaVersionは1です。",
        "発言ID: utterance。発言: 点検日を確認することに決まりました。担当者名・話者ID・日付・期限の表現はすべてnullで返してください。",
        &schema,
        Duration::from_secs(180),
        &|| false,
    )
    .unwrap()
    .unwrap();
    let document = crate::generation::result::document(result, &input, &ids).unwrap();
    assert_eq!(document.items.len(), 1);
    assert_eq!(document.evidence[0].utterance_id, ids[0]);
    assert_eq!(
        document.evidence[0].quote_text,
        input.analysis.utterances[0].effective_text
    );
    assert!(document
        .markdown(&input)
        .unwrap()
        .contains("発言ID: utterance"));
    assert!(dir.path().join("result.json").is_file());
    println!(
        "real llama.cpp constrained JSON: CLI SHA {}, model SHA {}, CPU only, context {}",
        config.executable.sha256, config.model.sha256, config.context_tokens
    );
}
