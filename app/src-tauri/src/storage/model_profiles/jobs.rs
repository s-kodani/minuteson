//! Portable, non-owning provenance sealed inside a job's specification hash.
//! New work uses the reviewed current selection; fixed work never rereads it.
use super::*;
use crate::diarization::sherpa::SherpaConfig;
use crate::generation::llama::LlamaConfig;
use crate::local_engine::Fingerprint;
use crate::model_compatibility::ProbeReport;
use crate::transcription::asr::AsrConfig;
use serde_json::{json, Value};

pub(crate) mod rebinding;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Snapshot {
    schema_version: u32,
    profile: ProfileName,
    // Origin identities are evidence, NOT foreign keys to destination-local
    // profiles/packages. Portable restore retains them without installing AI.
    profile_id: String,
    profile_version: i64,
    profile_hash: String,
    package: PlacementRecord,
    compatibility: ProbeReport,
    configuration: Value,
}

#[derive(Clone, Copy)]
pub(crate) enum Execution<'a> {
    Asr(&'a AsrConfig),
    Sherpa(&'a SherpaConfig),
    Llama(&'a LlamaConfig, usize),
}
impl Execution<'_> {
    fn validate_settings(&self) -> Result<(), String> {
        match self {
            Self::Asr(e) => e.validate_settings(),
            Self::Sherpa(e) => e.validate_settings(None),
            Self::Llama(e, b) => {
                e.validate()?;
                let available = e
                    .context_tokens
                    .checked_sub(e.prediction_tokens)
                    .and_then(|n| n.checked_sub(512))
                    .ok_or("生成の入力予算が不正です")? as usize;
                if !(2048..=60000).contains(b) || *b > available {
                    return Err("生成の入力予算が不正です".into());
                }
                Ok(())
            }
        }
    }
    fn engine(&self) -> Engine {
        match self {
            Self::Asr(e) => crate::transcription::asr::engine(e),
            Self::Sherpa(_) => Engine::Sherpa,
            Self::Llama(_, _) => Engine::Llama,
        }
    }
    fn files(&self) -> Vec<(Role, &Fingerprint)> {
        match self {
            Self::Asr(e) => {
                let mut files = vec![(Role::Executable, &e.executable), (Role::Asr, &e.model)];
                files.extend(e.vad_model.iter().map(|f| (Role::Vad, f)));
                if let Some(backend) = &e.backend {
                    files.extend(backend.files.iter().map(|(k,f)| (if k == "library" { Role::RuntimeLibrary } else { Role::AsrAsset }, f)));
                }
                files
            }
            Self::Sherpa(e) => {
                let mut files = vec![
                    (Role::Executable, &e.executable),
                    (Role::Segmentation, &e.segmentation_model),
                    (Role::SpeakerEmbedding, &e.embedding_model),
                ];
                files.extend(
                    e.runtime_libraries
                        .iter()
                        .map(|f| (Role::RuntimeLibrary, f)),
                );
                files
            }
            Self::Llama(e, _) => vec![(Role::Executable, &e.executable), (Role::Llm, &e.model)],
        }
    }
    fn settings(&self) -> Result<Settings, String> {
        let path = |f: &Fingerprint| -> Result<String, String> {
            f.path
                .to_str()
                .map(str::to_owned)
                .ok_or("モデルパスが不正です".into())
        };
        Ok(match self {
            Self::Asr(e) => { let preferences = preferences::EnginePreferences {
                executable_path: path(&e.executable)?,
                model_path: path(&e.model)?,
                vad_model_path: e.vad_model.as_ref().map(path).transpose()?,
                vad_options: e.vad_options,
                threads: e.threads,
                use_gpu: e.use_gpu,
                backend: e.backend.as_ref().map(|b| -> Result<_,String> { Ok(crate::transcription::asr::BackendPreferences { id: b.id.clone(), files: b.files.iter().map(|(k,f)| Ok((k.clone(), path(f)?))).collect::<Result<_,String>>()? }) }).transpose()?,
            }; if e.backend.is_some() { Settings::Asr(preferences) } else { Settings::Whisper(preferences) } },
            Self::Sherpa(e) => {
                Settings::Sherpa(diarization_preferences::DiarizationEnginePreferences {
                    executable_path: path(&e.executable)?,
                    segmentation_model_path: path(&e.segmentation_model)?,
                    embedding_model_path: path(&e.embedding_model)?,
                    runtime_library_paths: e
                        .runtime_libraries
                        .iter()
                        .map(path)
                        .collect::<Result<_, _>>()?,
                    threads: e.threads,
                    cluster_threshold_milli: e.cluster_threshold_milli,
                    window_shift_milli: e.window_shift_milli,
                    min_duration_on_ms: e.min_duration_on_ms,
                    min_duration_off_ms: e.min_duration_off_ms,
                    speech_sensitivity_milli: e.speech_sensitivity_milli,
                })
            }
            Self::Llama(e, prompt_budget) => {
                Settings::Llama(minutes_preferences::MinutesEnginePreferences {
                    executable_path: path(&e.executable)?,
                    model_path: path(&e.model)?,
                    threads: e.threads,
                    context_tokens: e.context_tokens,
                    prediction_tokens: e.prediction_tokens,
                    gpu_layers: e.gpu_layers,
                    seed: e.seed,
                    prompt_budget: *prompt_budget,
                })
            }
        })
    }
    // No host paths or vocabulary hint in the portable selection. The latter
    // has its own immutable vocabulary snapshot in the transcription job.
    fn configuration(&self) -> Value {
        match self {
            Self::Asr(e) => {
                let mut value = json!({"threads":e.threads,"useGpu":e.use_gpu});
                if let Some(options) = e.vad_options {
                    value["vadOptions"] = serde_json::to_value(options).unwrap();
                }
                value
            }
            Self::Sherpa(e) => {
                let mut value = json!({"threads":e.threads,"clusterThresholdMilli":e.cluster_threshold_milli,
                "windowShiftMilli":e.window_shift_milli,"minDurationOnMs":e.min_duration_on_ms,"minDurationOffMs":e.min_duration_off_ms});
                if e.speech_sensitivity_milli != 0 {
                    value["speechSensitivityMilli"] = json!(e.speech_sensitivity_milli);
                }
                value
            }
            Self::Llama(e, b) => json!({"threads":e.threads,"contextTokens":e.context_tokens,
                "predictionTokens":e.prediction_tokens,"gpuLayers":e.gpu_layers,"seed":e.seed,"promptBudget":b}),
        }
    }
    fn same_conditions(&self, other: &Self) -> bool {
        self.engine() == other.engine()
            && self.configuration() == other.configuration()
            && self.files() == other.files()
    }
}
fn name(value: &str) -> Result<ProfileName, String> {
    match value {
        "standard" => Ok(ProfileName::Standard),
        "high_accuracy" => Ok(ProfileName::HighAccuracy),
        _ => Err("未対応のモデルプロファイルです".into()),
    }
}

impl Snapshot {
    fn validate_provenance(&self) -> Result<(), String> {
        self.compatibility.validate_portable(&self.package)?;
        if self.schema_version != 1
            || !canonical_id(&self.profile_id)
            || self.profile_version <= 0
            || !hash(&self.profile_hash)
            || !self.configuration.is_object()
        {
            return Err("固定した管理モデルの構成版が不正です".into());
        }
        Ok(())
    }
    pub(crate) fn validate(&self, profile: &str, execution: &Execution<'_>) -> Result<(), String> {
        self.validate_provenance()?;
        execution.validate_settings()?;
        if self.profile != name(profile)?
            || self.package.manifest.engine != execution.engine()
            || self.configuration != execution.configuration()
        {
            return Err("固定した管理モデルの構成版・条件が一致しません".into());
        }
        let proofs = execution.files();
        let declared = self
            .package
            .files
            .iter()
            .filter(|f| {
                // Whisper/llama runtime libraries are verified as part of the whole
                // package even though their CLI configs have no library argument.
                f.role != Role::Notice
                    && (f.role != Role::RuntimeLibrary || matches!(execution.engine(), Engine::Sherpa | Engine::Asr))
            })
            .collect::<Vec<_>>();
        if proofs.len() != declared.len() {
            return Err("固定した管理モデルの役割が不足・重複しています".into());
        }
        let mut seen = BTreeSet::new();
        let mut directory: Option<PathBuf> = None;
        let mut unavailable = None;
        for (role, proof) in proofs {
            let portable =
                proof.path.to_str() == Some(format!("mon-unavailable:{}", proof.sha256).as_str());
            if unavailable
                .replace(portable)
                .is_some_and(|old| old != portable)
            {
                return Err("管理モデルの配置済み・未配置参照が混在しています".into());
            }
            let candidates = declared
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    f.role == role
                        && f.actual_size == proof.size
                        && f.actual_hash == proof.sha256
                        && (portable
                            || package_directory(
                                &proof.path,
                                &f.relative_path,
                                &self.package.package_id,
                            )
                            .is_some_and(|root| {
                                directory.as_ref().is_none_or(|expected| expected == root)
                            }))
                })
                .map(|(i, _)| i)
                .filter(|i| !seen.contains(i))
                .collect::<Vec<_>>();
            // Portable identical bytes may have multiple library names. Match
            // each once; native paths disambiguate them by relative name.
            let i = *candidates
                .first()
                .ok_or("管理モデルの確認済み容量・SHA・役割と固定条件が一致しません")?;
            seen.insert(i);
            if !portable {
                if !proof.path.is_absolute()
                    || proof
                        .path
                        .components()
                        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
                {
                    return Err("管理モデルの絶対パスが不正です".into());
                }
                let root = package_directory(
                    &proof.path,
                    &declared[i].relative_path,
                    &self.package.package_id,
                )
                .ok_or("管理モデルの配置先が不正です")?;
                if directory
                    .replace(root.to_owned())
                    .is_some_and(|old| old != root)
                {
                    return Err("管理モデルの同一パッケージ参照が一致しません".into());
                }
            }
        }
        Ok(())
    }
    fn verify(
        &self,
        profile: &str,
        execution: &Execution<'_>,
        cancel: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.validate(profile, execution)?;
        let executable = execution.files()[0].1;
        let relative = &self
            .package
            .files
            .iter()
            .find(|f| f.role == Role::Executable)
            .ok_or("実行ファイルがありません")?
            .relative_path;
        let mut directory = executable.path.as_path();
        for _ in Path::new(relative).components() {
            directory = directory.parent().ok_or("管理モデルは未配置です")?;
        }
        let root = directory.parent().ok_or("管理モデルは未配置です")?;
        crate::model_compatibility::verify_fixed_package(root, &self.package, cancel)
    }
}

fn package_directory<'a>(path: &'a Path, relative: &str, package: &str) -> Option<&'a Path> {
    if !path.ends_with(relative) {
        return None;
    }
    let mut root = path;
    for _ in Path::new(relative).components() {
        root = root.parent()?;
    }
    (root.file_name() == Some(std::ffi::OsStr::new(package))
        && root.parent().and_then(Path::file_name) == Some(std::ffi::OsStr::new("models")))
    .then_some(root)
}

// All declared dependencies travel as unavailable historical metadata, including
// libraries without an explicit whisper/llama CLI argument. No local path or
// profile/check/package installation is transported.
pub(in crate::storage) fn portable_files(
    value: &Value,
) -> Result<Vec<super::super::model_metadata::ModelFileMetadata>, String> {
    let snapshot: Snapshot = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    snapshot.validate_provenance()?;
    snapshot
        .package
        .files
        .iter()
        .map(|file| {
            let metadata = super::super::model_metadata::ModelFileMetadata {
                name: Path::new(&file.relative_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or("モデルの履歴名が不正です")?
                    .into(),
                size: file.actual_size,
                sha256: file.actual_hash.clone(),
                role: match file.role {
                    Role::Executable => "executable",
                    Role::Asr | Role::Llm => "model",
                    Role::Vad => "vadModel",
                    _ => "artifact",
                }
                .into(),
            };
            metadata.validate()?;
            Ok(metadata)
        })
        .collect()
}

pub(crate) fn attach(mut profile: Value, snapshot: Option<Snapshot>) -> Result<Value, String> {
    if let Some(snapshot) = snapshot {
        profile["managedModel"] = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    }
    Ok(profile)
}
pub(crate) fn read(profile: &Value) -> Result<Option<Snapshot>, String> {
    profile
        .get("managedModel")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())
}
pub(crate) fn validate(profile: &Value, execution: Execution<'_>) -> Result<(), String> {
    if let Some(snapshot) = read(profile)? {
        snapshot.validate(
            profile["profile"]
                .as_str()
                .ok_or("モデルプロファイルがありません")?,
            &execution,
        )?;
    }
    if let Some(binding) = rebinding::read(profile)? {
        binding.validate(profile, execution)?;
    }
    Ok(())
}
pub(crate) fn verify(
    profile: &Value,
    execution: Execution<'_>,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    validate(profile, execution)?;
    if let Some(binding) = rebinding::read(profile)? {
        return binding.verify(profile, execution, cancel);
    }
    if let Some(snapshot) = read(profile)? {
        snapshot.verify(
            profile["profile"]
                .as_str()
                .ok_or("モデルプロファイルがありません")?,
            &execution,
            cancel,
        )?;
    }
    Ok(())
}

fn current(
    db: &Connection,
    profile: &str,
    execution: &Execution<'_>,
) -> Result<Option<(Snapshot, PathBuf)>, String> {
    let n = name(profile)?;
    let (heads, _) = heads(db)?;
    if let Some(head) = heads.get(n) {
        let stored = load_profile(db, n, head)?;
        if let Some(binding) = stored
            .bindings
            .iter()
            .find(|b| b.settings.engine() == execution.engine())
        {
            if binding.settings != settings(db, n, execution.engine())?.0
                || binding.settings != execution.settings()?
            {
                return Err("適用済みモデル構成と解析設定が異なります。構成を再確認して明示的に再適用してください".into());
            }
            let latest = super::super::model_checks::latest(db, &binding.package_id)?
                .ok_or("互換性確認がありません")?;
            if latest.summary.state != CheckState::Succeeded
                || latest.summary.operation_id != binding.check_operation_id
            {
                return Err(
                    "適用後に互換性確認が変わっています。再確認・再適用してください".into(),
                );
            }
            let record = binding.validate(db)?;
            let compatibility = super::super::model_checks::read(db, &binding.check_operation_id)?
                .and_then(|c| c.report)
                .ok_or("成功した互換性証拠がありません")?;
            let snapshot = Snapshot {
                schema_version: 1,
                profile: n,
                profile_id: head.id.clone(),
                profile_version: stored.version,
                profile_hash: head.content_hash.clone(),
                package: record,
                compatibility,
                configuration: execution.configuration(),
            };
            snapshot.validate(profile, execution)?;
            return Ok(Some((snapshot, binding.models_root.clone())));
        }
    }
    // Do not silently adopt bytes from a managed directory as an unreviewed
    // external model, including the other profile's selected package.
    if execution.files().iter().any(|(_, p)| {
        p.path.ancestors().any(|a| {
            a.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(canonical_id)
                && a.parent().and_then(Path::file_name) == Some(std::ffi::OsStr::new("models"))
        })
    }) {
        return Err("管理モデルは対象プロファイルへ確認・適用してから解析してください".into());
    }
    Ok(None)
}

// Decode just the engine used by the current native producers.
fn with_execution<T>(
    spec: &JobSpec,
    run: impl FnOnce(Execution<'_>) -> Result<T, String>,
) -> Result<T, String> {
    match spec.kind {
        JobKind::Transcription => {
            let e: AsrConfig = serde_json::from_value(spec.parameters["engine"].clone())
                .map_err(|e| e.to_string())?;
            run(Execution::Asr(&e))
        }
        JobKind::VoiceRegistration | JobKind::VoiceMatching => {
            let e: SherpaConfig =
                serde_json::from_value(spec.parameters["voice"]["engine"].clone())
                    .map_err(|e| e.to_string())?;
            run(Execution::Sherpa(&e))
        }
        JobKind::SpeakerCompletion => {
            let e: SherpaConfig =
                serde_json::from_value(spec.parameters["completion"]["engine"].clone())
                    .map_err(|e| e.to_string())?;
            run(Execution::Sherpa(&e))
        }
        JobKind::Diarization => {
            let e: SherpaConfig =
                serde_json::from_value(spec.parameters["diarization"]["engine"].clone())
                    .map_err(|e| e.to_string())?;
            run(Execution::Sherpa(&e))
        }
        JobKind::MinutesGeneration => {
            let e: LlamaConfig =
                serde_json::from_value(spec.parameters["generation"]["engine"].clone())
                    .map_err(|e| e.to_string())?;
            let b = spec.parameters["generation"]["promptBudget"]
                .as_u64()
                .and_then(|b| usize::try_from(b).ok())
                .ok_or("入力上限がありません")?;
            run(Execution::Llama(&e, b))
        }
    }
}
fn flow_selection(
    db: &Connection,
    meeting: &str,
    operation: &str,
    profile: &str,
    execution: &Execution<'_>,
) -> Result<Option<Option<Snapshot>>, String> {
    if !operation.starts_with("flow:") {
        return Ok(None);
    }
    let flow = super::super::analysis_flows::read_flow(db, meeting)?
        .ok_or("固定した解析工程がありません")?;
    let kind = match execution {
        Execution::Asr(_) => "transcription",
        Execution::Sherpa(_) => "diarization",
        _ => return Err("工程のモデル種別が不正です".into()),
    };
    if operation != format!("flow:{}:{kind}", flow.id) || profile != flow.profile {
        return Err("解析工程の由来が一致しません".into());
    }
    let plan = crate::analysis_flow::saved_plan(
        flow.plan.ok_or("解析工程の固定条件がありません")?,
        profile,
    )?;
    let (fixed, snapshot) = match execution {
        Execution::Asr(_) => (
            Execution::Asr(&plan.whisper),
            plan.whisper_managed.clone(),
        ),
        Execution::Sherpa(_) => (Execution::Sherpa(&plan.sherpa), plan.sherpa_managed.clone()),
        _ => unreachable!(),
    };
    if !execution.same_conditions(&fixed) {
        return Err("モデル条件が解析工程の固定計画と一致しません".into());
    }
    Ok(Some(snapshot))
}

impl Storage {
    // Admission reads sealed installation metadata only. Large model/package
    // byte verification belongs to the claimed worker, before any inference.
    pub(crate) fn prepare_minutes_admission(
        &self,
        request: &crate::generation::StartRequest,
    ) -> Result<(LlamaConfig, Snapshot), String> {
        let requested = Settings::Llama(
            super::super::minutes_preferences::MinutesEnginePreferences {
                executable_path: request
                    .executable_path
                    .to_str()
                    .ok_or("実行パスが不正です")?
                    .into(),
                model_path: request
                    .model_path
                    .to_str()
                    .ok_or("モデルパスが不正です")?
                    .into(),
                threads: request.threads,
                context_tokens: request.context_tokens,
                prediction_tokens: request.prediction_tokens,
                gpu_layers: request.gpu_layers,
                seed: request.seed,
                prompt_budget: request.prompt_budget,
            },
        );
        requested.validate()?;
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let existing: Option<String> = db
            .query_row(
                "SELECT id FROM processing_jobs WHERE operation_id=?1",
                params![request.input.operation_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = existing {
            let (spec, _) = super::super::jobs::read_fixed_input(&db, &id)?;
            if spec.kind != JobKind::MinutesGeneration
                || spec.meeting_id != request.input.meeting_id
                || spec.profile_snapshot["profile"] != request.profile
            {
                return Err("同じ操作IDで異なる議事録生成が指定されました".into());
            }
            let engine: LlamaConfig =
                serde_json::from_value(spec.parameters["generation"]["engine"].clone())
                    .map_err(|e| e.to_string())?;
            let execution = Execution::Llama(&engine, request.prompt_budget);
            if execution.settings()? != requested {
                return Err("同じ操作IDで異なるモデル条件が指定されました".into());
            }
            let snapshot =
                read(&spec.profile_snapshot)?.ok_or("議事録の管理モデル証拠がありません")?;
            snapshot.validate(&request.profile, &execution)?;
            return Ok((engine, snapshot));
        }
        let n = name(&request.profile)?;
        let (heads, _) = heads(&db)?;
        let head = heads.get(n).ok_or("設定で議事録モデルを準備してください")?;
        let stored = load_profile(&db, n, head)?;
        let binding = stored
            .bindings
            .iter()
            .find(|b| b.settings.engine() == Engine::Llama)
            .ok_or("設定で議事録モデルを準備してください")?;
        if binding.settings != requested {
            return Err("選択した議事録モデルが変更されています。再確認してください".into());
        }
        let record = binding.validate(&db)?;
        let fingerprint = |role| -> Result<Fingerprint, String> {
            let files: Vec<_> = record.files.iter().filter(|f| f.role == role).collect();
            if files.len() != 1 {
                return Err("議事録モデルの構成が不正です".into());
            }
            let file = files[0];
            Ok(Fingerprint {
                path: binding
                    .models_root
                    .join(&record.package_id)
                    .join(&file.relative_path),
                size: file.actual_size,
                sha256: file.actual_hash.clone(),
            })
        };
        let engine = LlamaConfig {
            executable: fingerprint(Role::Executable)?,
            model: fingerprint(Role::Llm)?,
            threads: request.threads,
            context_tokens: request.context_tokens,
            prediction_tokens: request.prediction_tokens,
            gpu_layers: request.gpu_layers,
            seed: request.seed,
        };
        let snapshot = current(
            &db,
            &request.profile,
            &Execution::Llama(&engine, request.prompt_budget),
        )?
        .ok_or("議事録の管理モデル証拠がありません")?
        .0;
        Ok((engine, snapshot))
    }

    pub(crate) fn prepare_model_selection(
        &self,
        profile: &str,
        execution: Execution<'_>,
        cancel: &impl Fn() -> bool,
    ) -> Result<Option<Snapshot>, String> {
        let selected = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            current(&db, profile, &execution)?
        };
        if let Some((snapshot, root)) = selected {
            let (record, _) = crate::model_compatibility::verify_success(
                self,
                &root,
                &snapshot.compatibility.operation_id,
                &snapshot.package.package_id,
                &snapshot.package.review_hash,
                cancel,
            )?;
            if record != snapshot.package {
                return Err("確認済みモデルの証拠が変化しています".into());
            }
            Ok(Some(snapshot))
        } else {
            Ok(None)
        }
    }
    pub(crate) fn job_model_selection(
        &self,
        meeting: &str,
        operation: &str,
        profile: &str,
        execution: Execution<'_>,
    ) -> Result<Option<Snapshot>, String> {
        let fixed = {
            let db = self
                .connection
                .lock()
                .map_err(|_| "DBロックを取得できません")?;
            let job: Option<String> = db
                .query_row(
                    "SELECT id FROM processing_jobs WHERE operation_id=?1",
                    params![operation],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(id) = job {
                let (spec, _) = super::super::jobs::read_fixed_input(&db, &id)?;
                if spec.meeting_id != meeting
                    || spec.profile_snapshot["profile"] != profile
                    || !with_execution(&spec, |e| Ok(execution.same_conditions(&e)))?
                {
                    return Err("同じ操作IDで異なるモデル条件が指定されました".into());
                }
                Some(read(&spec.profile_snapshot)?)
            } else {
                flow_selection(&db, meeting, operation, profile, &execution)?
            }
        };
        if let Some(snapshot) = fixed {
            if let Some(s) = &snapshot {
                s.verify(profile, &execution, &|| false)?;
            }
            Ok(snapshot)
        } else {
            self.prepare_model_selection(profile, execution, &|| false)
        }
    }
}

pub(in crate::storage) fn gate(db: &Connection, spec: &JobSpec) -> Result<(), String> {
    // A relocation proof belongs only to a separately reviewed retry. A new
    // job must use the ordinary profile selection, never supplied retry paths.
    if spec.profile_snapshot.get("runtimeModel").is_some() {
        return Err("モデルの再結合は確認付きの新しい再試行として行ってください".into());
    }
    let native = matches!(
        spec.profile_snapshot["profile"].as_str(),
        Some("standard" | "high_accuracy")
    ) && match spec.kind {
        JobKind::Transcription => spec.parameters["engine"].is_object(),
        JobKind::SpeakerCompletion => spec.parameters["completion"]["engine"].is_object(),
        JobKind::Diarization => spec.parameters["diarization"]["engine"].is_object(),
        JobKind::VoiceRegistration | JobKind::VoiceMatching => {
            spec.parameters["voice"]["engine"].is_object()
        }
        JobKind::MinutesGeneration => spec.parameters["generation"]["engine"].is_object(),
    };
    if !native {
        if spec.profile_snapshot.get("managedModel").is_some() {
            return Err("管理モデルの固定条件が不足しています".into());
        }
        return Ok(());
    }
    with_execution(spec, |e| {
        validate(&spec.profile_snapshot, e)?;
        let profile = spec.profile_snapshot["profile"]
            .as_str()
            .ok_or("プロファイルがありません")?;
        let expected = if let Some(fixed) =
            flow_selection(db, &spec.meeting_id, &spec.operation_id, profile, &e)?
        {
            fixed
        } else {
            current(db, profile, &e)?.map(|(s, _)| s)
        };
        if expected != read(&spec.profile_snapshot)? {
            return Err(
                "解析開始前に管理モデルの構成版が変わっています。再確認してください".into(),
            );
        }
        Ok(())
    })
}

pub(in crate::storage) fn gate_plan(
    db: &Connection,
    profile: &str,
    value: &Value,
) -> Result<(), String> {
    let plan = crate::analysis_flow::saved_plan(value.clone(), profile)?;
    if current(db, profile, &Execution::Asr(&plan.whisper))?.map(|(s, _)| s)
        != plan.whisper_managed
        || current(db, profile, &Execution::Sherpa(&plan.sherpa))?.map(|(s, _)| s)
            != plan.sherpa_managed
    {
        return Err("工程保存前に管理モデルの構成版が変わっています".into());
    }
    Ok(())
}
