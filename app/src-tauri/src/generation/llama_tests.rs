use super::*;
use serde_json::json;

#[test]
fn incomplete_output_is_not_repaired_or_published_and_has_a_recoverable_message() {
    for source in [
        br#"{"markdown":"unfinished"#.as_slice(),
        br#"{"markdown":"unfinished\"#.as_slice(),
    ] {
        let error = parse_output(source).unwrap_err();
        assert!(error.contains("minutes_output_incomplete"));
        assert!(error.contains("保存していません"));
    }
    assert!(parse_output(br#"{"markdown":oops}"#)
        .unwrap_err()
        .contains("JSONが不正"));
}

#[test]
fn markdown_bound_counts_decoded_unicode_without_truncating_text() {
    let schema = crate::generation::markdown::bounded_schema(2048);
    assert_eq!(schema["properties"]["markdown"]["maxLength"], 480);
    assert!(validate_markdown_bound(&json!({"markdown":"あ".repeat(480)}), &schema).is_ok());
    assert!(
        validate_markdown_bound(&json!({"markdown":"あ".repeat(481)}), &schema)
            .unwrap_err()
            .contains("minutes_output_limit")
    );
    assert!(validate_markdown_bound(
        &json!({"markdown":"old saved body".repeat(1000)}),
        &crate::generation::markdown::schema()
    )
    .is_ok());
}

#[test]
fn literal_markdown_whitespace_preserves_text_without_accepting_other_invalid_json() {
    let source = "{\n\"markdown\":\"# 会議\n\n- 点検日\t確認\r\n[区間](audio:00:00:01-00:00:02)\"\n}\n[end of text]";
    assert_eq!(
        parse_output(source.as_bytes()).unwrap(),
        json!({"markdown":"# 会議\n\n- 点検日\t確認\r\n[区間](audio:00:00:01-00:00:02)"})
    );
    assert_eq!(
        parse_output(br#"{"markdown":"escaped \\n and \"quotes\""}"#).unwrap(),
        json!({"markdown":"escaped \\n and \"quotes\""})
    );
    for source in [
        "{\"markdown\":\"a\0b\"}",
        "{\"markdown\":\"a\u{1b}b\"}",
        "{\"markdown\":\"a\n",
        "{\"markdown\":\"a\n\",\"markdown\":\"b\"}",
        "{\"markdown\":\"a\n\"}{}",
        "log\n{\"markdown\":\"a\n\"}",
        "{\"markdown\":\"a\\\nb\"}",
    ] {
        assert!(parse_output(source.as_bytes()).is_err(), "{source:?}");
    }
}

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
        "--fit\noff",
        "--device\nnone",
        "--no-kv-offload",
        "--no-op-offload",
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
fn explicit_gpu_selection_is_not_silently_overridden_by_cpu_flags() {
    let (dir, mut config) = fake("printf '%s\\n' \"$@\" > args.log\nprintf '{\"items\":[]}'");
    config.gpu_layers = 3;
    let path = unit(dir.path(), "gpu-argument-contract-only");
    run_unit(
        &config,
        &path,
        "fixed",
        "input",
        &json!({}),
        Duration::from_secs(5),
        &|| false,
    )
    .unwrap();
    let args = std::fs::read_to_string(path.join("args.log")).unwrap();
    assert!(args.contains("--gpu-layers\n3") && args.contains("--fit\noff"));
    for cpu_only in ["--device", "--no-kv-offload", "--no-op-offload"] {
        assert!(!args.contains(cpu_only), "{cpu_only}: {args}");
    }
    // A fake CLI checks the contract only; this is not a real GPU trial.
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
        let pause_requested = std::cell::Cell::new(None::<Instant>);
        let result = run_unit(
            &config,
            &path,
            "fixed",
            "input",
            &json!({}),
            // Startup under parallel-test load is not cancellation latency.
            // Keep the timeout test short, but let the pause fixture become
            // ready before requesting its stop.
            if pause {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(250)
            },
            &|| {
                let ready = pause
                    && std::fs::read_to_string(path.join("child.pid"))
                        .ok()
                        .and_then(|s| s.parse::<u32>().ok())
                        .is_some_and(|pid| pid > 0);
                if ready && pause_requested.get().is_none() {
                    pause_requested.set(Some(Instant::now()));
                }
                ready
            },
        );
        if pause {
            assert!(result.unwrap().is_none());
            assert!(pause_requested.get().unwrap().elapsed() < Duration::from_secs(3));
        } else {
            assert!(result.unwrap_err().contains("制限時間"));
            assert!(started.elapsed() < Duration::from_secs(3));
        }
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
    let mut schema = crate::generation::result::schema(&input, &ids).unwrap();
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
