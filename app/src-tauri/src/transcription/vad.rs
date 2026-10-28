//! Integer-only persisted VAD settings. Absent settings retain legacy CLI defaults.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct VadOptions {
    pub enabled: bool,
    pub threshold_milli: u32,
    pub min_speech_duration_ms: u32,
    pub min_silence_duration_ms: u32,
    pub max_speech_duration_s: u32,
    pub speech_pad_ms: u32,
    pub samples_overlap_ms: u32,
}
impl Default for VadOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_milli: 600,
            min_speech_duration_ms: 300,
            min_silence_duration_ms: 100,
            max_speech_duration_s: 30,
            speech_pad_ms: 30,
            samples_overlap_ms: 100,
        }
    }
}
impl VadOptions {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(10..=990).contains(&self.threshold_milli)
            || self.threshold_milli % 10 != 0
            || !(1..=5000).contains(&self.min_speech_duration_ms)
            || !(1..=5000).contains(&self.min_silence_duration_ms)
            || !(1..=120).contains(&self.max_speech_duration_s)
            || self.speech_pad_ms > 1000
            || self.samples_overlap_ms > 1000
            || self.samples_overlap_ms % 10 != 0
            || self.min_speech_duration_ms > self.max_speech_duration_s * 1000
        {
            return Err("VADの数値が入力範囲外か、最小発話時間が最大発話区間を超えています".into());
        }
        Ok(())
    }
    pub(crate) fn args(&self, command: &mut std::process::Command) {
        command.args([
            "--vad-threshold",
            &format!("{:.2}", self.threshold_milli as f64 / 1000.0),
            "--vad-min-speech-duration-ms",
            &self.min_speech_duration_ms.to_string(),
            "--vad-min-silence-duration-ms",
            &self.min_silence_duration_ms.to_string(),
            "--vad-max-speech-duration-s",
            &self.max_speech_duration_s.to_string(),
            "--vad-speech-pad-ms",
            &self.speech_pad_ms.to_string(),
            "--vad-samples-overlap",
            &format!("{:.2}", self.samples_overlap_ms as f64 / 1000.0),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_engine::Fingerprint;
    use crate::transcription::whisper::{run_unit, AsrConfig};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    #[test]
    fn vad_settings_reject_invalid_combinations_and_preserve_disabled_values() {
        let mut v = VadOptions::default();
        v.enabled = false;
        v.threshold_milli = 990;
        v.validate().unwrap();
        for field in 0..8 {
            let mut bad = v;
            match field {
                0 => bad.threshold_milli = 601,
                1 => bad.threshold_milli = 1000,
                2 => bad.min_speech_duration_ms = 0,
                3 => bad.min_silence_duration_ms = 5001,
                4 => bad.max_speech_duration_s = 0,
                5 => bad.speech_pad_ms = 1001,
                6 => bad.samples_overlap_ms = 101,
                _ => {
                    bad.max_speech_duration_s = 1;
                    bad.min_speech_duration_ms = 1001;
                }
            }
            assert!(bad.validate().is_err());
        }
    }
    #[test]
    #[cfg(unix)]
    fn vad_cli_arguments_freeze_all_values_and_off_or_legacy_omit_new_flags() {
        let root = tempfile::tempdir().unwrap();
        let model = root.path().canonicalize().unwrap().join("model");
        let vad = root.path().canonicalize().unwrap().join("vad");
        let cli = root.path().canonicalize().unwrap().join("cli");
        std::fs::write(&model, b"synthetic model").unwrap();
        std::fs::write(&vad, b"synthetic vad").unwrap();
        std::fs::write(&cli, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > args.log\nprintf '%s' '{}' > result.json\n", serde_json::json!({"model":{"multilingual":true},"params":{"model":model,"language":"ja","translate":false},"transcription":[]}))).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        for (index, options) in [
            Some(VadOptions::default()),
            Some(VadOptions {
                enabled: false,
                ..Default::default()
            }),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let dir = root.path().join(index.to_string());
            std::fs::create_dir(&dir).unwrap();
            crate::test_support::write_wave(&dir.join("input.wav"));
            let capture = |p: &std::path::Path| Fingerprint::capture(p, &|| false).unwrap();
            let config = AsrConfig {
                backend: None,
                executable: capture(&cli),
                model: capture(&model),
                vad_model: Some(capture(&vad)),
                vad_options: options,
                threads: 1,
                use_gpu: false,
                prompt: String::new(),
            };
            let before = serde_json::to_vec(&config).unwrap();
            assert!(
                run_unit(&config, &dir, 1_000_000, Duration::from_secs(5), &|| false)
                    .unwrap()
                    .unwrap()
                    .is_empty()
            );
            let args = std::fs::read_to_string(dir.join("args.log")).unwrap();
            assert_eq!(args.lines().any(|s| s == "--vad"), index != 1);
            assert_eq!(args.contains("--vad-threshold\n0.60\n"), index == 0);
            if index == 0 {
                for arg in [
                    "--vad-min-speech-duration-ms\n300\n",
                    "--vad-min-silence-duration-ms\n100\n",
                    "--vad-max-speech-duration-s\n30\n",
                    "--vad-speech-pad-ms\n30\n",
                    "--vad-samples-overlap\n0.10\n",
                ] {
                    assert!(args.contains(arg));
                }
            }
            if index == 2 {
                assert!(!String::from_utf8(before.clone())
                    .unwrap()
                    .contains("vadOptions"));
            }
            assert_eq!(serde_json::to_vec(&config).unwrap(), before);
        }
    }
}

#[cfg(test)]
mod real_tests {
    use super::*;
    use crate::local_engine::Fingerprint;
    use crate::transcription::whisper::{run_unit, AsrConfig};
    use std::path::PathBuf;
    use std::time::Duration;
    #[test]
    #[ignore = "requires the pinned base/Silero models, bundled CLI and synthetic wave fixtures; no recording"]
    fn real_vad_compare_silence_noise_speech_and_chunk_boundary() {
        let path = |key| PathBuf::from(std::env::var(key).expect(key));
        let capture = |key| Fingerprint::capture(&path(key), &|| false).unwrap();
        let model = capture("MON_QA_VAD_ASR");
        assert_eq!(
            model.sha256,
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
        );
        let vad = capture("MON_QA_VAD_MODEL");
        assert_eq!(
            vad.sha256,
            "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987"
        );
        let cli = capture("MON_QA_VAD_CLI");
        let inputs = path("MON_QA_VAD_INPUTS");
        let results = path("MON_QA_VAD_RESULTS");
        std::fs::create_dir(&results).unwrap();
        let mut report = vec![];
        for name in [
            "silence",
            "noise",
            "speech-gap",
            "quiet-speech",
            "short-speech",
            "boundary-full",
            "boundary-first",
            "boundary-second",
        ] {
            let wav = inputs.join(format!("{name}.wav"));
            let duration =
                ((std::fs::metadata(&wav).unwrap().len() - 44) * 1_000_000 / 32000) as i64;
            for enabled in [false, true] {
                let dir = results.join(format!("{name}-{enabled}"));
                std::fs::create_dir(&dir).unwrap();
                std::fs::copy(&wav, dir.join("input.wav")).unwrap();
                let config = AsrConfig {
                    backend: None,
                    executable: cli.clone(),
                    model: model.clone(),
                    vad_model: Some(vad.clone()),
                    vad_options: Some(VadOptions {
                        enabled,
                        ..Default::default()
                    }),
                    threads: 2,
                    use_gpu: false,
                    prompt: String::new(),
                };
                let start = std::time::Instant::now();
                let segments =
                    run_unit(&config, &dir, duration, Duration::from_secs(600), &|| false)
                        .unwrap()
                        .unwrap();
                if enabled && matches!(name, "silence" | "noise") {
                    assert!(
                        segments.is_empty(),
                        "VAD false positive: {name} {segments:?}"
                    );
                }
                if enabled && matches!(name, "speech-gap" | "boundary-full" | "boundary-second") {
                    assert!(!segments.is_empty(), "missing normal speech: {name}");
                    let expected = if name == "speech-gap" {
                        5_000_000
                    } else if name == "boundary-full" {
                        119_000_000
                    } else {
                        1_000_000
                    };
                    assert!(
                        (segments[0].start_us - expected).abs() < 1_000_000,
                        "original timeline not preserved: {name} {segments:?}"
                    );
                }
                report.push(serde_json::json!({"case":name,"vad":enabled,"seconds":start.elapsed().as_secs_f64(),"segments":segments}));
                println!("{}", report.last().unwrap());
            }
        }
        std::fs::write(
            results.join("comparison.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}
