use super::*;
use serde_json::json;

fn segments(texts: &[&str], step: i64) -> Vec<Segment> {
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| Segment {
            start_us: i as i64 * step,
            end_us: (i as i64 + 1) * step,
            text: (*text).into(),
        })
        .collect()
}
fn vocabulary(terms: &[(&str, Option<&str>)]) -> VocabularySnapshot {
    serde_json::from_value(json!({"schemaVersion":1,"id":"id","meetingId":"meeting","stamp":{"globalVersion":1,"meetingVersion":0},"entries":terms.iter().map(|(term,reading)|json!({"scope":"global","entryVersion":1,"entry":{"id":term,"term":term,"reading":reading,"note":"説明文は認識へ渡さない","enabled":true}})).collect::<Vec<_>>(),"method":"whisper-vocabulary-v2","extraPrompt":"","actualPrompt":"","contentHash":"hash"})).unwrap()
}
#[test]
fn repeated_sentences_segments_and_alternating_cycles_are_detected_without_editing_text() {
    let phrase = "無条件な本文置換ではありません。";
    for original in [
        segments(&[phrase, phrase, phrase], 4_000_000),
        segments(&[&phrase.repeat(3)], 20_000_000),
        segments(
            &[
                "この項目を確認します。",
                "次の項目へ進みます。",
                "この項目を確認します。",
                "次の項目へ進みます。",
                "この項目を確認します。",
                "次の項目へ進みます。",
            ],
            2_000_000,
        ),
        segments(&[&"本日の説明を開始します".repeat(5)], 10_000_000),
        segments(
            &[
                " ALPHA Beta.",
                "alpha beta!",
                "ALPHA BETA?",
                "alpha beta.",
                "alpha beta!",
                "ALPHA BETA?",
            ],
            3_000_000,
        ),
    ] {
        let copy = original.clone();
        assert_eq!(reasons(&original, None), vec![Reason::Repetition]);
        assert_eq!(original, copy);
    }
}
#[test]
fn short_responses_two_repeats_and_distinct_speech_are_not_rejected() {
    for original in [
        segments(&["はい"; 12], 10_000_000),
        segments(
            &["次の資料を確認します。", "次の資料を確認します。"],
            15_000_000,
        ),
        segments(
            &[
                "次の資料を確認します。",
                "質問について説明します。",
                "以上で会議を終了します。",
            ],
            15_000_000,
        ),
    ] {
        assert!(reasons(&original, None).is_empty());
    }
    // Three real repeated instructions may trigger a retry, but are never deleted.
    let real = segments(&["安全を確認してください。"; 3], 5_000_000);
    assert_eq!(reasons(&real, None), vec![Reason::Repetition]);
    assert_eq!(real.len(), 3);
}
#[test]
fn reading_and_adjacent_dictionary_echo_are_detected_but_one_term_is_allowed() {
    let v = vocabulary(&[
        (
            "dbt Projects on Snowflake",
            Some("でぃーびーてぃーぷろじぇくつおんすのーふれーく"),
        ),
        ("Authentication Policy", None),
    ]);
    assert_eq!(
        reasons(
            &segments(
                &["でぃーびーてぃーぷろじぇくつおんすのーふれーく"],
                3_000_000
            ),
            Some(&v)
        ),
        vec![Reason::ReadingEcho]
    );
    assert_eq!(
        reasons(
            &segments(
                &["dbt Projects on Snowflake Authentication Policy"],
                3_000_000
            ),
            Some(&v)
        ),
        vec![Reason::VocabularyEcho]
    );
    assert!(reasons(
        &segments(&["dbt Projects on Snowflakeを利用します。"], 3_000_000),
        Some(&v)
    )
    .is_empty());
    assert!(reasons(&segments(&["説明文は認識へ渡さない"], 3_000_000), Some(&v)).is_empty());
}

#[test]
fn hinted_timestamp_gaps_trigger_retry_but_do_not_reject_real_unhinted_pauses() {
    let s = vec![
        Segment {
            start_us: 0,
            end_us: 2_000_000,
            text: "お待ちください".into(),
        },
        Segment {
            start_us: 31_000_000,
            end_us: 38_000_000,
            text: "次の内容をご説明します".into(),
        },
    ];
    assert_eq!(hinted_reasons(&s, None), vec![Reason::TimestampGap]);
    assert!(reasons(&s, None).is_empty());
    let mut normal = s.clone();
    normal[1].start_us = 13_000_000;
    assert!(hinted_reasons(&normal, None).is_empty());
    let one = &s[..1];
    assert!(hinted_reasons(one, None).is_empty());
}

#[test]
#[ignore = "requires MON_GUARD_PCM_MANIFEST with verified PCM and unhinted reference paths"]
fn real_guarded_vocabulary_rechecks_verified_pcm_and_reference() {
    use crate::local_engine::Fingerprint;
    let manifest_path = PathBuf::from(std::env::var("MON_GUARD_PCM_MANIFEST").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let root = manifest_path.parent().unwrap();
    assert!(root
        .canonicalize()
        .unwrap()
        .starts_with(std::env::temp_dir().canonicalize().unwrap()));
    let config: WhisperConfig = serde_json::from_value(manifest["engine"].clone()).unwrap();
    let vocabulary: VocabularySnapshot =
        serde_json::from_value(manifest["vocabulary"].clone()).unwrap();
    config.verify(&|| false).unwrap();
    assert_eq!(vocabulary.method, "whisper-vocabulary-v2");
    vocabulary.validate().unwrap();
    assert_eq!(config.prompt, vocabulary.actual_prompt);
    let mut results = vec![];
    for item in manifest["units"].as_array().unwrap() {
        let index = item["index"].as_u64().unwrap();
        let source = PathBuf::from(item["pcm"].as_str().unwrap());
        let proof = Fingerprint::capture(&source, &|| false).unwrap();
        assert_eq!(proof.sha256, item["sha256"].as_str().unwrap());
        let d = root.join(format!("final-unit-{index}"));
        std::fs::create_dir(&d).unwrap();
        std::fs::copy(source, d.join("input.wav")).unwrap();
        let duration = item["durationUs"].as_i64().unwrap();
        let start = std::time::Instant::now();
        let output = run(
            &config,
            &d,
            duration,
            true,
            true,
            Some(&vocabulary),
            &|| false,
        )
        .unwrap()
        .unwrap();
        let selected = whisper::parse_output(
            &std::fs::read(&output.selected).unwrap(),
            &config.model.path,
            duration,
        )
        .unwrap();
        let baseline = whisper::parse_output(
            &std::fs::read(item["reference"].as_str().unwrap()).unwrap(),
            &config.model.path,
            duration,
        )
        .unwrap();
        let coverage = |s: &[Segment]| -> i64 {
            let mut end = 0;
            let mut total = 0;
            for v in s {
                total += (v.end_us - v.start_us.max(end)).max(0);
                end = end.max(v.end_us);
            }
            total
        };
        let selected_text = selected.iter().map(|s| s.text.as_str()).collect::<String>();
        assert!(!selected_text.contains("無条件な本文置換ではありません"));
        assert!(!selected_text.contains("表記・読みの認識ヒント"));
        assert!(reasons(&selected, None).is_empty());
        let reference_problems = reasons(&baseline, None);
        let record = json!({"index":index,"fallback":output.rejected.is_some(),"reasons":output.reasons,"selectedPath":output.selected,"rejectedPath":output.rejected,"segments":selected.len(),"selectedCoveredUs":coverage(&selected),"referenceCoveredUs":coverage(&baseline),"referenceProblems":reference_problems,"textExactMatch":selected==baseline,"elapsedSeconds":start.elapsed().as_secs_f64()});
        results.push(record);
        std::fs::write(
            root.join("final-comparison.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
        // A looping reference is not evidence of speech duration: its
        // repeated timestamps can cover real pauses or unrelated speech.
        if reference_problems.is_empty() {
            assert!(
                coverage(&selected) * 100 >= coverage(&baseline) * 85,
                "unit {index} lost reference coverage"
            );
        }
        eprintln!("final guarded vocabulary PCM: {}/19", index + 1);
    }
    assert_eq!(results.len(), 19);
    config.verify(&|| false).unwrap();
}
