//! One native diarization call over the shared, time-aligned meeting WAV.
use super::*;
use crate::saved_artifact::{RestoreAssets, Role};
use crate::storage::KnownInput;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Policy {
    algorithm: String,
    windows: String,
}
impl Policy {
    fn fixed() -> Self {
        Self {
            algorithm: "sherpa_meeting_mix_v1".into(),
            windows: "native".into(),
        }
    }
}

pub(crate) fn helper_path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    return crate::windows_runtime::embedding();
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    // Tests inject an owned signed binary, without changing production lookup.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("MON_EMBEDDING_HELPER") {
        paths.push(PathBuf::from(path));
    }
    if let Some(macos) = executable
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "MacOS"))
    {
        if let Some(contents) = macos.parent() {
            paths.push(contents.join("Resources/mon-speaker-embedding"));
        }
    }
    paths.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../probes/speaker-embedding/.build/mon-speaker-embedding"),
    );
    for path in paths {
        if let Ok(path) = path.canonicalize() {
            if path.is_file()
                && std::process::Command::new("/usr/bin/codesign")
                    .args(["--verify", "--strict"])
                    .arg(&path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            {
                return Ok(path);
            }
        }
    }
    Err("署名済み話者特徴量ヘルパーがありません。Mac用ビルドまたはprobes/speaker-embedding/build.shを実行してください".into())
}

fn validate(parameters: &Parameters) -> Result<&KnownInput, String> {
    super::validate_attribution_policy(parameters)?;
    if let Some(runtime) = &parameters.resident_runtime {
        runtime.accepts("sherpa")?;
    }
    let known = parameters
        .known_input
        .as_ref()
        .ok_or("固定保存区間がありません")?;
    known.validate(&parameters.source)?;
    if parameters.matching.as_ref() != Some(&Policy::fixed())
        || parameters.tracks.len() != parameters.source.tracks.len()
    {
        return Err("保存区間の話者分離条件が不正です".into());
    }
    for (track, settings) in parameters.source.tracks.iter().zip(&parameters.tracks) {
        parameters.engine.validate_settings(settings.num_speakers)?;
        if track.id != settings.track_id {
            return Err("保存区間の人数適用先が不正です".into());
        }
    }
    if parameters.source.tracks.len() != 1
        || parameters.source.tracks[0].kind != "processed"
        || known.runs.len() != 1
    {
        return Err("会議全体の共通音声が必要です".into());
    }
    for index in 0..known.runs.len() {
        run_frames(parameters, index)?;
    }
    Ok(known)
}

fn run_frames(parameters: &Parameters, index: usize) -> Result<u64, String> {
    let run = parameters
        .known_input
        .as_ref()
        .and_then(|known| known.runs.get(index))
        .ok_or("保存区間の位置が範囲外です")?;
    let rate = u64::from(
        parameters
            .source
            .tracks
            .get(run.track_index)
            .ok_or("保存区間の入力がありません")?
            .sample_rate,
    );
    if rate == 0 {
        return Err("保存区間の音声形式が不正です".into());
    }
    let frames = run
        .end_frame
        .checked_sub(run.first_frame)
        .and_then(|n| n.checked_mul(16000))
        .and_then(|n| n.checked_add(rate / 2))
        .ok_or("保存区間のフレームが範囲外です")?
        / rate;
    if frames == 0 || frames > u64::from((u32::MAX - 36) / 2) {
        return Err("保存区間用WAVのサイズ上限を超えています".into());
    }
    Ok(frames)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue(
    storage: &Storage,
    request: MinutesJobRequest,
    engine: SherpaConfig,
    profile: &str,
    tracks: Vec<TrackSettings>,
    transcription_job: Option<&str>,
    source: RecordedAudioPlan,
    known: KnownInput,
    _helper: Option<&Path>,
) -> Result<JobSnapshot, String> {
    let verification = crate::local_engine::verification::Scope::join_or_enter(true)?;
    if !matches!(profile, "standard" | "high_accuracy") {
        return Err("話者分離のプロファイルが不正です".into());
    }
    engine.verify(&|| false)?;
    let selected = storage.job_model_selection(
        &request.meeting_id,
        &request.operation_id,
        profile,
        Execution::Sherpa(&engine),
    )?;
    storage.verify_meeting_asset(
        &request.meeting_id,
        &source.manifest_asset_id,
        &source.manifest_hash,
    )?;
    let mut parameters = Parameters {
        optimization_version: storage
            .optimization_version_for_operation(&request.meeting_id, &request.operation_id)?,
        completion_after: super::completion_after(
            storage,
            &request.meeting_id,
            &request.operation_id,
        )?,
        attribution_policy: super::attribution_policy(
            storage,
            &request.meeting_id,
            &request.operation_id,
        )?,
        resident_runtime: crate::local_engine::resident::prepare_job(
            storage,
            &request.meeting_id,
            &request.operation_id,
            "sherpa",
        )?,
        source,
        engine,
        tracks,
        manual_assignment_hash: String::new(),
        known_input: Some(known),
        matching: Some(Policy::fixed()),
    };
    let known = validate(&parameters)?;
    let total = known.runs.len() as i64;
    let profile = managed::attach(
        serde_json::json!({"profile":profile,"engine":"sherpa-onnx","segmentationSha256":parameters.engine.segmentation_model.sha256,"embeddingSha256":parameters.engine.embedding_model.sha256,"qualityStatus":"unverified"}),
        selected,
    )?;
    verification.finish(&|| false)?;
    storage.enqueue_diarization_source_version(
        request,
        profile,
        transcription_job,
        crate::current_formats::DIARIZATION,
        move |input| {
            if input.audio_manifest_hash != parameters.source.manifest_hash {
                return Err("文字起こしと保存音声の由来が一致しません".into());
            }
            for utterance in &input.analysis.utterances {
                if !parameters.source.tracks.iter().any(|track| {
                    track.id == utterance.track_id
                        && utterance.start_us >= track.start_us
                        && utterance.end_us <= track.end_us
                }) {
                    return Err("発言のトラック・時刻が固定保存音声と一致しません".into());
                }
            }
            parameters.manual_assignment_hash = input.manual_assignment_hash.clone();
            Ok((
                serde_json::to_value(parameters).map_err(|e| e.to_string())?,
                total,
            ))
        },
    )
}

fn envelope(spec: &JobSpec) -> Result<Envelope, String> {
    let value: Envelope =
        serde_json::from_value(spec.parameters.clone()).map_err(|e| e.to_string())?;
    let known = validate(&value.diarization)?;
    if spec.kind != JobKind::Diarization
        || value.schema_version != crate::current_formats::DIARIZATION
        || value.request.meeting_id != spec.meeting_id
        || spec.total_units != Some(known.runs.len() as i64)
    {
        return Err("保存区間別の話者分離固定計画が不正です".into());
    }
    managed::validate(
        &spec.profile_snapshot,
        Execution::Sherpa(&value.diarization.engine),
    )?;
    Ok(value)
}

pub(crate) fn fixed_coverage(spec: &JobSpec) -> Result<(RecordedAudioPlan, KnownInput), String> {
    let value = envelope(spec)?;
    Ok((
        value.diarization.source,
        value.diarization.known_input.unwrap(),
    ))
}

// Bound decoded native output before constructing the candidate.
#[derive(Default)]
struct SavedLimits {
    segments: usize,
}
impl SavedLimits {
    fn add(&mut self, result: &ResultFile) -> Result<(), String> {
        self.segments = self
            .segments
            .checked_add(result.segments.len())
            .ok_or("話者区間件数が範囲外です")?;
        if self.segments > 100_000 {
            return Err("会議全体の話者区間が上限を超えています".into());
        }
        Ok(())
    }
}

pub(super) fn checked_parameters(
    storage: &Storage,
    spec: &JobSpec,
    job: &str,
) -> Result<Parameters, String> {
    let envelope = envelope(spec)?;
    let input = storage.load_diarization_input(job)?;
    if envelope.request.operation_id != storage.fixed_job_initial_operation(job)?
        || envelope.diarization.source.manifest_hash != input.audio_manifest_hash
        || envelope.diarization.manual_assignment_hash != input.manual_assignment_hash
    {
        return Err("保存区間別の固定入力・割り当てが不正です".into());
    }
    Ok(envelope.diarization)
}

pub(super) fn preflight(
    storage: &Storage,
    spec: &JobSpec,
    parameters: &Parameters,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<crate::playback::PreparedAudio>, String> {
    let known = validate(parameters)?;
    let runtime = managed::rebinding::sherpa(&spec.profile_snapshot, &parameters.engine)?;
    runtime.verify(cancelled)?;
    managed::verify(
        &spec.profile_snapshot,
        Execution::Sherpa(&parameters.engine),
        cancelled,
    )?;
    let expected = (parameters.source.clone(), Some(known.clone()));
    if storage.recorded_analysis_input(&spec.meeting_id)? != expected {
        return Err("固定した保存音声・欠損・時計が変更されています".into());
    }
    storage.verify_meeting_asset(
        &spec.meeting_id,
        &parameters.source.manifest_asset_id,
        &parameters.source.manifest_hash,
    )?;
    storage.verify_meeting_mix_sources(&spec.meeting_id, cancelled)?;
    let mut prepared = Vec::new();
    // Verify each physical source once at admission, not one open/hash scan for
    // every gap. A new unit opens only its restricted run, retaining bounded FDs.
    for track in &parameters.source.tracks {
        prepared.push(storage.prepare_analysis_track(&spec.meeting_id, track, cancelled)?);
    }
    if storage.recorded_analysis_input(&spec.meeting_id)? != expected {
        return Err("準備中に固定保存区間が変更されています".into());
    }
    Ok(prepared)
}

fn validate_result(
    result: &ResultFile,
    spec: &JobSpec,
    parameters: &Parameters,
    index: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let known = validate(parameters)?;
    let run = known.runs.get(index).ok_or("保存結果の区間が範囲外です")?;
    let track = &parameters.source.tracks[run.track_index];
    if result.schema_version != 2
        || result.origin_operation_id != spec.parameters["request"]["operationId"]
        || Some(result.input_analysis_id.as_str()) != spec.input_analysis_id.as_deref()
        || result.input_hash != spec.input_hash
        || result.source_manifest_hash != parameters.source.manifest_hash
        || result.track_id != track.id
        || result.track_start_us != run.start_us
        || result.duration_us != (run_frames(parameters, index)? * 1_000_000 / 16000) as i64
        || result.engine != parameters.engine
        || result.num_speakers != parameters.tracks[run.track_index].num_speakers
        || result.run.as_ref() != Some(run)
        || result.matching != parameters.matching
    {
        return Err("保存区間の話者結果・抽出条件の由来が一致しません".into());
    }
    if cancelled() {
        return Err("話者結果の検証を中断しました".into());
    }
    sherpa::validate_saved_segments(&result.segments, result.num_speakers, result.duration_us)?;
    Ok(())
}

pub(super) fn load_output(
    storage: &Storage,
    spec: &JobSpec,
    parameters: &Parameters,
    output: &UnitOutput,
    cancelled: &impl Fn() -> bool,
) -> Result<ResultFile, String> {
    let known = validate(parameters)?;
    let run = known
        .runs
        .get(output.index)
        .ok_or("再開保存区間の位置が不正です")?;
    let track = &parameters.source.tracks[run.track_index];
    crate::restored_processing::verify_pcm(
        storage,
        &spec.meeting_id,
        &output.wav_asset_id,
        &output.wav_hash,
        cancelled,
        |path| {
            storage
                .copy_meeting_mix(&spec.meeting_id, track, path, cancelled)
                .map(|_| ())
        },
    )?;
    let bytes = storage.read_analysis_result(
        &spec.meeting_id,
        &output.result_asset_id,
        &output.result_hash,
    )?;
    let result: ResultFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate_result(&result, spec, parameters, output.index, cancelled)?;
    if crate::storage::backup::omitted_processing_asset(
        storage,
        &spec.meeting_id,
        &output.stdout_asset_id,
        &output.stdout_hash,
        Role::DiarizationLog,
    )?
    .is_none()
    {
        let bytes = storage.read_inference_artifact(
            &spec.meeting_id,
            &output.stdout_asset_id,
            &output.stdout_hash,
            "text/plain",
            16 * 1024 * 1024,
        )?;
        if sherpa::parse_with_merges(
            &bytes,
            &parameters.engine,
            result.num_speakers,
            result.duration_us,
        )? != (result.segments.clone(), result.speaker_merges.clone())
        {
            return Err("保存区間の話者結果と元出力が一致しません".into());
        }
    }
    Ok(result)
}

#[cfg(test)]
pub(super) fn process(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
) -> Result<Option<String>, String> {
    process_observed(storage, job_id, stopping, progress, &|_| {})
}

pub(super) fn process_observed(
    storage: &Storage,
    job_id: &str,
    stopping: &impl Fn() -> bool,
    progress: &impl Fn(JobSnapshot),
    runtime_progress: &impl Fn(super::runtime::Progress),
) -> Result<Option<String>, String> {
    let (spec, raw) = storage.fixed_job_input(job_id)?;
    let parameters = checked_parameters(storage, &spec, job_id)?;
    let optimization =
        crate::processing_optimization::Scope::enter(parameters.optimization_version)?;
    let _resident = crate::local_engine::resident::Scope::enter(
        storage.root(),
        parameters.resident_runtime.as_ref(),
    )?;
    let job = storage.get_job(job_id)?;
    let mut checkpoint = checked_checkpoint(&job, raw)?;
    let cancelled = || {
        stopping()
            || storage
                .get_job(job_id)
                .map_or(true, |job| job.state != "running")
    };
    let attempt = (|| {
        let runtime = managed::rebinding::sherpa(&spec.profile_snapshot, &parameters.engine)?;
        preflight(storage, &spec, &parameters, &cancelled)?;
        let known = validate(&parameters)?;
        let mut results = Vec::new();
        let mut limits = SavedLimits::default();
        for output in &checkpoint.outputs {
            let result = load_output(storage, &spec, &parameters, output, &cancelled)?;
            limits.add(&result)?;
            results.push(result);
        }
        for (index, run) in known.runs.iter().enumerate().skip(checkpoint.outputs.len()) {
            if cancelled() {
                return Ok(None);
            }
            let track = &parameters.source.tracks[run.track_index];
            let directory = storage.analysis_unit_directory(&spec.meeting_id, job_id, index)?;
            let wave_path = directory.join("input.wav");
            let frames =
                storage.copy_meeting_mix(&spec.meeting_id, track, &wave_path, &cancelled)?;
            if frames != run_frames(&parameters, index)? {
                return Err("変換した保存区間のフレームが一致しません".into());
            }
            let native_duration = (frames * 1_000_000 / 16000) as i64;
            let observer = super::runtime::Observer::new(&job, index, runtime_progress);
            observer.emit(0, false);
            let Some(segments) = sherpa::run_track_observed(
                &runtime,
                parameters.tracks[run.track_index].num_speakers,
                &directory,
                native_duration,
                Duration::from_secs(24 * 3600),
                &cancelled,
                &|value| observer.emit(value, false),
            )?
            else {
                return Ok(None);
            };
            observer.emit(10_000, true);
            let native_output = crate::restored_processing::read_local(
                &directory.join("stdout.log"),
                16 * 1024 * 1024,
                &cancelled,
            )?;
            let (_, speaker_merges) = sherpa::parse_with_merges(
                &native_output,
                &runtime,
                parameters.tracks[run.track_index].num_speakers,
                native_duration,
            )?;
            let result = ResultFile {
                schema_version: 2,
                origin_operation_id: spec.parameters["request"]["operationId"]
                    .as_str()
                    .ok_or("元操作がありません")?
                    .into(),
                input_analysis_id: spec
                    .input_analysis_id
                    .clone()
                    .ok_or("固定入力版がありません")?,
                input_hash: spec.input_hash.clone(),
                source_manifest_hash: parameters.source.manifest_hash.clone(),
                track_id: track.id.clone(),
                track_start_us: run.start_us,
                duration_us: native_duration,
                engine: parameters.engine.clone(),
                num_speakers: parameters.tracks[run.track_index].num_speakers,
                segments,
                speaker_merges,
                run: Some(run.clone()),
                matching: parameters.matching.clone(),
            };
            validate_result(&result, &spec, &parameters, index, &cancelled)?;
            limits.add(&result)?;
            let bytes = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("保存区間の話者結果が容量上限を超えています".into());
            }
            crate::restored_processing::write_new(&directory.join("result.json"), &bytes)?;
            let wav =
                storage.register_analysis_artifact(&spec.meeting_id, &wave_path, "audio/wav")?;
            let mut stdout_path = directory.join("stdout.log");
            if runtime != parameters.engine {
                let bytes = crate::restored_processing::read_local(
                    &stdout_path,
                    16 * 1024 * 1024,
                    &cancelled,
                )?;
                let normalized = sherpa::logical_output(
                    &bytes,
                    &runtime,
                    &parameters.engine,
                    parameters.tracks[run.track_index].num_speakers,
                    native_duration,
                )?;
                stdout_path = directory.join("logical-stdout.log");
                crate::restored_processing::write_new(&stdout_path, &normalized)?;
            }
            let stdout =
                storage.register_analysis_artifact(&spec.meeting_id, &stdout_path, "text/plain")?;
            let json = storage.register_analysis_artifact(
                &spec.meeting_id,
                &directory.join("result.json"),
                "application/json",
            )?;
            let output = UnitOutput {
                index,
                wav_asset_id: wav.id,
                wav_hash: wav.content_hash,
                stdout_asset_id: stdout.id,
                stdout_hash: stdout.content_hash,
                result_asset_id: json.id,
                result_hash: json.content_hash,
            };
            let verified = load_output(storage, &spec, &parameters, &output, &cancelled)?;
            checkpoint.outputs.push(output);
            let saved = match storage.save_job_checkpoint(
                job_id,
                index as i64,
                index as i64 + 1,
                serde_json::to_value(&checkpoint).map_err(|e| e.to_string())?,
            ) {
                Ok(job) => job,
                Err(error) => {
                    checkpoint.outputs.pop();
                    return Err(error);
                }
            };
            let paused = saved.state == "paused";
            results.push(verified);
            progress(saved);
            if paused {
                return Ok(None);
            };
        }
        if cancelled() {
            return Ok(None);
        };
        preflight(storage, &spec, &parameters, &cancelled)?;
        for (index, output) in checkpoint.outputs.iter().enumerate() {
            if load_output(storage, &spec, &parameters, output, &cancelled)? != results[index] {
                return Err("公開前に保存区間の話者・特徴量が変化しました".into());
            }
        }
        optimization.finish(&cancelled)?;
        let input = storage.load_diarization_input(job_id)?;
        let candidate = build_candidate(&spec, &job, &parameters, &input, &results, &cancelled)?;
        let id = storage.finish_diarization_candidate(
            &spec.meeting_id,
            job_id,
            checkpoint.outputs.len() as i64,
            candidate,
        )?;
        progress(storage.get_job(job_id)?);
        Ok(Some(id))
    })();
    if cancelled() {
        let current = storage.get_job(job_id)?;
        if current.state == "running" && stopping() {
            storage.request_job_pause(job_id, "exit")?;
        }
        if storage.get_job(job_id)?.state == "pausing" {
            progress(storage.save_job_checkpoint(
                job_id,
                checkpoint.outputs.len() as i64,
                checkpoint.outputs.len() as i64,
                serde_json::to_value(checkpoint).map_err(|e| e.to_string())?,
            )?);
        }
        if let Ok(Some(id)) = attempt {
            return Ok(Some(id));
        }
        return Ok(None);
    }
    attempt
}

fn build_candidate(
    spec: &JobSpec,
    job: &JobSnapshot,
    parameters: &Parameters,
    input: &DiarizationInput,
    results: &[ResultFile],
    cancelled: &impl Fn() -> bool,
) -> Result<AnalysisInput, String> {
    let known = validate(parameters)?;
    if results.len() != known.runs.len() {
        return Err("保存区間の話者結果が揃っていません".into());
    }
    let mut limits = SavedLimits::default();
    for result in results {
        limits.add(result)?;
    }
    let track = &parameters.source.tracks[0];
    let result = results.first().ok_or("会議全体の話者結果がありません")?;
    let labels: Vec<_> = result
        .segments
        .iter()
        .map(|s| s.speaker)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let keys: Vec<_> = labels
        .iter()
        .map(|id| format!("meeting:speaker_{id:02}"))
        .collect();
    let has_text = input
        .analysis
        .utterances
        .iter()
        .any(|u| !u.recognized_text.trim().is_empty());
    let mut speakers: Vec<_> = labels
        .iter()
        .zip(&keys)
        .map(|(id, key)| SpeakerInput {
            key: key.clone(),
            label: format!("話者{}", id + 1),
        })
        .collect();
    if !has_text {
        speakers.clear();
    }
    for speaker in &input.analysis.speakers {
        if input
            .analysis
            .utterances
            .iter()
            .any(|u| input.manual_utterances.contains(&u.id) && u.speaker_ids.contains(&speaker.id))
        {
            speakers.push(SpeakerInput {
                key: format!("prior:{}", speaker.id),
                label: speaker.label.clone(),
            });
        }
    }
    let segments: Vec<_> = result
        .segments
        .iter()
        .map(|s| Segment {
            start_us: s.start_us,
            end_us: s.end_us,
            speaker: labels.binary_search(&s.speaker).unwrap() as u32,
        })
        .collect();
    let raw_segments = result
        .segments
        .iter()
        .map(|s| SpeakerSegmentInput {
            track_id: track.id.clone(),
            speaker_key: has_text.then(|| format!("meeting:speaker_{:02}", s.speaker)),
            start_us: s.start_us,
            end_us: s.end_us,
            overlap_group: None,
            confidence: None,
        })
        .collect();
    let mut timelines = BTreeMap::new();
    timelines.insert(
        track.id.clone(),
        (0, Timeline::build(&segments, result.duration_us)?, keys),
    );
    let observed = vec![labels.len()];
    let unresolved = vec![0_usize];
    let mut utterances = Vec::new();
    for original in &input.analysis.utterances {
        let (offset, timeline, keys) = timelines
            .get(&original.track_id)
            .ok_or("発言の固定トラックがありません")?;
        let contained = known.runs.iter().any(|run| {
            parameters.source.tracks[run.track_index].id == original.track_id
                && original.start_us >= run.start_us
                && original.end_us <= run.end_us
        });
        let speaker_keys = if input.manual_utterances.contains(&original.id) {
            original
                .speaker_ids
                .iter()
                .map(|id| format!("prior:{id}"))
                .collect()
        } else if !contained {
            vec![]
        } else {
            match timeline.attribute_with_policy(
                original.start_us - offset,
                original.end_us - offset,
                parameters.attribution_policy.is_some(),
            )? {
                Attribution::Assigned { speaker } => vec![keys[speaker as usize].clone()],
                Attribution::Overlap { speakers } => speakers
                    .iter()
                    .map(|speaker| keys[*speaker as usize].clone())
                    .collect(),
                _ => vec![],
            }
        };
        utterances.push(UtteranceInput {
            track_id: original.track_id.clone(),
            start_us: original.start_us,
            end_us: original.end_us,
            text: original.recognized_text.clone(),
            speaker_keys,
            confidence: None,
        });
    }
    Ok(AnalysisInput {
        audio_manifest_hash: parameters.source.manifest_hash.clone(),
        profile_snapshot: spec.profile_snapshot.clone(),
        config: serde_json::json!({"jobId":job.id,"specHash":job.spec_hash,"retryOfJobId":job.retry_of_job_id,"attempt":job.attempt,"inputAnalysisId":input.analysis.id,"inputContentHash":spec.input_hash,"diarization":"sherpa-onnx","tracks":parameters.tracks,"requestedSettings":parameters.engine,"effectiveSettings":{"numClusters":parameters.tracks[0].num_speakers.map_or(-1,|n|n as i32),"clusterThresholdMilli":if parameters.engine.cluster_threshold_milli==0 {500}else{parameters.engine.cluster_threshold_milli},"minDurationOnMs":if parameters.engine.min_duration_on_ms==0 {300}else{parameters.engine.min_duration_on_ms},"minDurationOffMs":if parameters.engine.min_duration_off_ms==0 {500}else{parameters.engine.min_duration_off_ms}},"speakerMerges":results.iter().flat_map(|r|r.speaker_merges.iter()).collect::<Vec<_>>(),"estimatedSpeakers":observed,"unmatchedRunGroups":unresolved,"speakerScope":"meeting_mix","speakerMatching":parameters.matching,"overlapDetection":"same_track_interval_intersection","ambiguousTurns":parameters.attribution_policy.as_deref().unwrap_or("keep_text_unknown_needs_split"),"manualAssignments":"preserved","timelineAlignment":"unverified","audioCoverage":{"mode":"known_runs","runCount":known.runs.len(),"missingRangeCount":known.gaps.iter().filter(|gap|!gap.recovered).count(),"timeBase":"meeting"}}),
        speakers,
        segments: raw_segments,
        utterances,
    })
}

fn validate_restore(
    old: &JobSpec,
    new: &JobSpec,
    before: &Envelope,
    after: &Envelope,
) -> Result<(), String> {
    if before.diarization.completion_after != after.diarization.completion_after
        || before.diarization.attribution_policy != after.diarization.attribution_policy
        || before.diarization.resident_runtime != after.diarization.resident_runtime
        || before.diarization.engine != after.diarization.engine
        || before.diarization.known_input != after.diarization.known_input
        || before.diarization.matching != after.diarization.matching
        || old.profile_snapshot != new.profile_snapshot
        || old.total_units != new.total_units
        || before
            .diarization
            .tracks
            .iter()
            .map(|track| track.num_speakers)
            .collect::<Vec<_>>()
            != after
                .diarization
                .tracks
                .iter()
                .map(|track| track.num_speakers)
                .collect::<Vec<_>>()
    {
        return Err("復元前後の保存区間・抽出・人数条件が一致しません".into());
    }
    for (old_track, new_track) in before
        .diarization
        .source
        .tracks
        .iter()
        .zip(&after.diarization.source.tracks)
    {
        if (
            old_track.start_us,
            old_track.end_us,
            old_track.sample_rate,
            old_track.channels,
            &old_track.kind,
        ) != (
            new_track.start_us,
            new_track.end_us,
            new_track.sample_rate,
            new_track.channels,
            &new_track.kind,
        ) || old_track.chunks.len() != new_track.chunks.len()
            || old_track
                .chunks
                .iter()
                .zip(&new_track.chunks)
                .any(|(old, new)| {
                    (
                        &old.hash,
                        old.size,
                        old.start_us,
                        old.end_us,
                        old.frames,
                        old.sample_rate,
                        old.channels,
                        &old.clock_segments,
                    ) != (
                        &new.hash,
                        new.size,
                        new.start_us,
                        new.end_us,
                        new.frames,
                        new.sample_rate,
                        new.channels,
                        &new.clock_segments,
                    )
                })
        {
            return Err("復元前後の音声証拠・入力形式・取得時計が一致しません".into());
        }
    }
    Ok(())
}

pub(super) fn restore_result(
    bytes: &[u8],
    old: &JobSpec,
    new: &JobSpec,
) -> Result<Vec<u8>, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("話者結果が容量上限を超えています".into());
    }
    let before = envelope(old)?;
    let after = envelope(new)?;
    validate_restore(old, new, &before, &after)?;
    let mut result: ResultFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let run = result
        .run
        .as_ref()
        .ok_or("保存話者結果に区間がありません")?;
    let index = before
        .diarization
        .known_input
        .as_ref()
        .unwrap()
        .runs
        .iter()
        .position(|old| old == run)
        .ok_or("保存話者結果の固定区間がありません")?;
    validate_result(&result, old, &before.diarization, index, &|| false)?;
    let new_track = &after.diarization.source.tracks[run.track_index];
    let old_track = &before.diarization.source.tracks[run.track_index];
    if (
        old_track.start_us,
        old_track.end_us,
        old_track.sample_rate,
        old_track.channels,
    ) != (
        new_track.start_us,
        new_track.end_us,
        new_track.sample_rate,
        new_track.channels,
    ) {
        return Err("復元前後の入力形式・時間が一致しません".into());
    }
    result.origin_operation_id = after.request.operation_id;
    result.input_analysis_id = new
        .input_analysis_id
        .clone()
        .ok_or("復元入力がありません")?;
    result.input_hash = new.input_hash.clone();
    result.source_manifest_hash = after.diarization.source.manifest_hash.clone();
    result.track_id = new_track.id.clone();
    validate_result(&result, new, &after.diarization, index, &|| false)?;
    let bytes = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("復元話者結果が容量上限を超えています".into());
    }
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn restore_saved_results(
    job: &JobSnapshot,
    old: &JobSpec,
    new: &JobSpec,
    value: serde_json::Value,
    old_initial: &str,
    new_initial: &str,
    assets: &mut impl RestoreAssets,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let before = envelope(old)?;
    let after = envelope(new)?;
    validate_restore(old, new, &before, &after)?;
    if before.request.operation_id != old_initial
        || after.request.operation_id != new_initial
        || job.kind != "diarization"
        || job.meeting_id != old.meeting_id
        || job.input_hash != old.input_hash
        || job.total_units != old.total_units
    {
        return Err("復元する保存区間ジョブの由来が不正です".into());
    }
    let checkpoint = checked_checkpoint(job, Some(value))?;
    for output in checkpoint.outputs {
        if cancelled() {
            return Err("話者特徴量の復元を中断しました".into());
        }
        let bytes = assets.read(
            Role::DiarizationResult,
            &output.result_asset_id,
            &output.result_hash,
        )?;
        let result: ResultFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        validate_result(&result, old, &before.diarization, output.index, cancelled)?;
        let rebuilt = restore_result(&bytes, old, new)?;
        assets.write(
            Role::DiarizationResult,
            &output.result_asset_id,
            &output.result_hash,
            &rebuilt,
        )?;
    }
    if cancelled() {
        return Err("話者特徴量の復元を中断しました".into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "known_tests.rs"]
mod tests;
