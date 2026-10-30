use super::*;
use serde_json::json;

#[test]
fn additional_models_use_only_their_own_sampling_settings() {
    for (size, sha, expected) in [
        (
            4_977_171_584,
            "85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87",
            vec![
                "--temp", "1.0", "--top-p", "0.95", "--top-k", "64", "--min-p", "0",
            ],
        ),
        (
            1_099_627_808,
            "3793e1e6e2266919d6e015287221b41100175a98eddaba92e111aed3b05c93c4",
            vec!["--temp", "0.1", "--top-k", "50", "--repeat-penalty", "1.1"],
        ),
        (
            2_491_874_688,
            "01999f17c39cc3074afae5e9c539bc82d45f2dd7faa3917c66cbef76fce8c0c2",
            vec!["--temp", "0"],
        ),
    ] {
        let mut model = Fingerprint {
            path: "/not-run/model.gguf".into(),
            size,
            sha256: sha.into(),
        };
        assert_eq!(model_arguments(&model), expected);
        model.size += 1;
        assert_eq!(model_arguments(&model), &["--temp", "0"]);
    }
}

#[test]
fn model_qwen3_and_qwen35_exact_bytes_use_non_thinking_template_and_recommended_sampling() {
    for (size, sha256) in [
        (
            5_027_783_488,
            "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785",
        ),
        (
            2_497_280_256,
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
        ),
        (
            2_740_937_888,
            "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
        ),
        (
            5_680_522_464,
            "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
        ),
    ] {
        let mut model = Fingerprint {
            path: "/not-run/model.gguf".into(),
            size,
            sha256: sha256.into(),
        };
        let args = model_arguments(&model);
        assert!(args.windows(2).any(|a| a == ["--temp", "0.7"]));
        assert!(args.windows(2).any(|a| a == ["--presence-penalty", "1.5"]));
        assert!(args.contains(&"--chat-template"));
        assert!(args.last().unwrap().contains("<think>\\n\\n</think>"));
        model.size -= 1;
        assert_eq!(model_arguments(&model), &["--temp", "0"]);
        model.size += 1;
        model.sha256 = "a".repeat(64);
        assert_eq!(model_arguments(&model), &["--temp", "0"]);
    }
}

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
fn explicit_schema_bound_counts_decoded_unicode_without_deriving_a_token_budget() {
    let schema = json!({"type":"object","properties":{"markdown":{"type":"string","maxLength":2}}});
    assert!(validate_markdown_bound(&json!({"markdown":"あい"}), &schema).is_ok());
    assert!(
        validate_markdown_bound(&json!({"markdown":"あいう"}), &schema)
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
        executable: Fingerprint::capture(&executable, &crate::test_support::never_cancelled)
            .unwrap(),
        model: Fingerprint::capture(&model, &crate::test_support::never_cancelled).unwrap(),
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
            &crate::test_support::never_cancelled
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
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert_eq!(std::fs::read(path.join("result.json")).unwrap(), saved);
    let mut invalid = config.clone();
    invalid.prediction_tokens = invalid.context_tokens;
    assert!(invalid
        .verify(&crate::test_support::never_cancelled)
        .is_err());
    std::fs::write(&config.model.path, b"changed model").unwrap();
    let second = unit(dir.path(), "changed");
    assert!(run_unit(
        &config,
        &second,
        "fixed",
        "input",
        &json!({}),
        Duration::from_secs(5),
        &crate::test_support::never_cancelled
    )
    .is_err());
    assert!(!second.join("result.json").exists());
}

#[test]
#[cfg(unix)]
fn long_context_input_reaches_completion_without_truncation() {
    let (dir, mut config) = fake("printf '%s\\n' \"$@\" > args.log\nprintf '{\"items\":[]}'");
    config.context_tokens = 262144;
    config.prediction_tokens = 32768;
    let path = unit(dir.path(), "long-context");
    let prompt = "会議の発言。".repeat(5000);
    assert!(prompt.len() > 64 * 1024);
    run_unit(
        &config,
        &path,
        "fixed",
        &prompt,
        &json!({}),
        Duration::from_secs(5),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("prompt.txt")).unwrap(),
        prompt
    );
    let args = std::fs::read_to_string(path.join("args.log")).unwrap();
    assert!(args.contains("--ctx-size\n262144") && args.contains("--predict\n32768"));
    config.context_tokens += 1;
    assert!(config.validate().is_err());
}

#[test]
#[cfg(unix)]
fn explicit_gpu_selection_is_not_silently_overridden_by_cpu_flags() {
    let (dir, mut config) = fake(
        "printf '%s\\n' \"$@\" > args.log\nprintf '%s\\n' 'llama_prepare_model_devices: using device MTL0 (Apple GPU)' 'load_tensors: offloaded 3/3 layers to GPU' >&2\nprintf '{\"items\":[]}'",
    );
    config.gpu_layers = 3;
    let path = unit(dir.path(), "gpu-argument-contract-only");
    run_unit(
        &config,
        &path,
        "fixed",
        "input",
        &json!({}),
        Duration::from_secs(5),
        &crate::test_support::never_cancelled,
    )
    .unwrap();
    let args = std::fs::read_to_string(path.join("args.log")).unwrap();
    assert!(args.contains("--gpu-layers\n3") && args.contains("--fit\noff"));
    assert!(args.contains("--device\nMTL0"));
    for cpu_only in ["--device\nnone", "--no-kv-offload", "--no-op-offload"] {
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
            &crate::test_support::never_cancelled
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
            assert!(result.unwrap_err().contains("minutes_generation_stalled"));
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
            &crate::test_support::never_cancelled
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
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        model: Fingerprint::capture(
            Path::new(&std::env::var("MON_LLAMA_MODEL").unwrap()),
            &crate::test_support::never_cancelled,
        )
        .unwrap(),
        threads: 4,
        context_tokens: 4096,
        prediction_tokens: 256,
        gpu_layers: 0,
        seed: 1,
    };
    let dir = tempfile::tempdir().unwrap();
    let schema = crate::generation::markdown::schema();
    let result = run_unit(
        &config, dir.path(), crate::generation::markdown::system(),
        "点検日を確認することに決まりました。短いMarkdown議事録をJSONのmarkdownフィールドに返してください。",
        &schema, Duration::from_secs(180), &crate::test_support::never_cancelled,
    ).unwrap().unwrap();
    assert!(!crate::generation::markdown::text(result)
        .unwrap()
        .trim()
        .is_empty());
    assert!(dir.path().join("result.json").is_file());
    println!(
        "real llama.cpp constrained JSON: CLI SHA {}, model SHA {}, CPU only, context {}",
        config.executable.sha256, config.model.sha256, config.context_tokens
    );
}

#[test]
#[cfg(unix)]
fn growing_result_can_finish_after_the_idle_deadline_but_stderr_cannot_extend_it() {
    let (dir, config) = fake("printf '{\"markdown\":\"'\nfor part in a b c d e; do printf '%s' \"$part\"; /bin/sleep 0.4; done\nprintf '\"}'");
    let path = unit(dir.path(), "slow-progress");
    let started = Instant::now();
    let value = run_unit(
        &config,
        &path,
        "fixed",
        "input",
        &json!({}),
        Duration::from_secs(1),
        &crate::test_support::never_cancelled,
    )
    .unwrap()
    .unwrap();
    assert!(started.elapsed() > Duration::from_secs(1));
    assert_eq!(value, json!({"markdown":"abcde"}));

    let (dir, config) = fake("printf '%s' \"$$\" > child.pid\nprintf '{'\nwhile :; do printf 'still waiting\\n' >&2; /bin/sleep 0.1; done");
    let path = unit(dir.path(), "stalled-with-logs");
    let error = run_unit(
        &config,
        &path,
        "fixed",
        "input",
        &json!({}),
        Duration::from_millis(400),
        &crate::test_support::never_cancelled,
    )
    .unwrap_err();
    assert!(error.contains("minutes_generation_stalled"));
    assert!(std::fs::metadata(path.join("stderr.log")).unwrap().len() > 0);
    let pid = std::fs::read_to_string(path.join("child.pid")).unwrap();
    assert!(!Command::new("/bin/kill")
        .args(["-0", &pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success());
    assert!(parse_output(&std::fs::read(path.join("result.json")).unwrap()).is_err());
}

#[test]
#[cfg(unix)]
fn measurements_observe_output_without_changing_json_or_disclosing_meeting_text() {
    let (root, config) = fake("printf '%s\\n' 'llama_perf_context_print: prompt eval time = 12.5 ms / 10 tokens (1.25 ms per token, 800.0 tokens per second)' 'llama_perf_context_print:        eval time = 20.0 ms / 2 runs (10.0 ms per token, 100.0 tokens per second)' >&2\nprintf '%s' '{\"markdown\":\"秘密の本文\"}'");
    let path = unit(root.path(), "measured");
    let events = std::cell::RefCell::new(Vec::new());
    let value = run_unit_observed(
        &config,
        &path,
        "secret system",
        "secret meeting",
        &json!({}),
        Duration::from_secs(5),
        &crate::test_support::never_cancelled,
        &|phase| {
            events
                .borrow_mut()
                .push(serde_json::to_value(phase).unwrap())
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(value["markdown"], "秘密の本文");
    let metrics = std::fs::read_to_string(path.join("performance.json")).unwrap();
    assert!(!metrics.contains("secret") && !metrics.contains("秘密"));
    let metrics: Value = serde_json::from_str(&metrics).unwrap();
    assert_eq!(metrics["promptEvaluation"]["tokens"], 10);
    assert_eq!(metrics["generation"]["tokensPerSecond"], 100.0);
    assert_eq!(metrics["gpuLayers"], 0);
    assert!(metrics["metalDevice"].is_null() && metrics["peakMemoryBytes"].is_null());
    assert!(metrics["firstOutputMs"].as_u64().unwrap() <= metrics["totalMs"].as_u64().unwrap());
    let events = events.borrow();
    assert_eq!(events.first().unwrap(), "verifying");
    assert!(events.iter().any(|v| v == "input"));
    assert!(events.iter().any(|v| v == "generating"));
    assert_eq!(events.last().unwrap(), "saving");
}

#[test]
#[cfg(unix)]
fn pinned_metal_requires_actual_offload_and_explicit_local_device() {
    for layers in [2, 3] {
        let script=format!("printf '%s\\n' \"$@\" > args.log\nprintf '%s\\n' 'llama_prepare_model_devices: using device MTL0 (Apple M1 Max)' 'load_tensors: offloaded {layers}/3 layers to GPU' >&2\nprintf '%s' '{{\"markdown\":\"GPU result\"}}'");
        let (root, mut config) = fake(&script);
        config.gpu_layers = 3;
        let dir = unit(root.path(), "metal");
        let result = run_unit_observed_on_device(
            &config,
            &dir,
            "fixed",
            "input",
            &json!({}),
            Duration::from_secs(5),
            &crate::test_support::never_cancelled,
            &|_| {},
            Some("MTL0"),
        );
        if layers == 3 {
            assert_eq!(result.unwrap().unwrap()["markdown"], "GPU result");
        } else {
            assert!(result.unwrap_err().contains("minutes_gpu_unavailable"));
        }
        let args = std::fs::read_to_string(dir.join("args.log")).unwrap();
        assert!(args.contains("--device\nMTL0\n"));
        assert!(args.contains("--log-verbosity\n4\n"));
        assert!(!args.contains("--no-kv-offload"));
    }
}

#[test]
#[cfg(unix)]
fn auto_metal_request_may_exceed_the_models_total_layers_but_must_actually_offload_all_of_them() {
    for (actual, total, success) in [
        (29, 29, true),
        (37, 37, true),
        (28, 29, false),
        (0, 29, false),
        (30, 29, false),
    ] {
        let script = format!("printf '%s\\n' 'using device MTL0 (Apple GPU)' 'load_tensors: offloaded {actual}/{total} layers to GPU' >&2\nprintf '%s' '{{\"ok\":true}}'");
        let (root, mut config) = fake(&script);
        config.gpu_layers = 256;
        let dir = unit(root.path(), "auto-metal");
        let result = run_unit_observed_on_device(
            &config,
            &dir,
            "fixed",
            "input",
            &json!({}),
            Duration::from_secs(5),
            &crate::test_support::never_cancelled,
            &|_| {},
            Some("MTL0"),
        );
        if success {
            assert_eq!(result.unwrap().unwrap(), json!({"ok":true}));
        } else {
            assert!(result.unwrap_err().contains("minutes_gpu_unavailable"));
        }
    }
}
