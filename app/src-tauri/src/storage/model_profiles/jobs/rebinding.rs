//! Relocate verified identical bytes, never rewrite a job's logical parameters.
//! Preparing a binding is read-only: it does not apply a profile, run a trial,
//! change a fixed job, or enqueue work. Retry admission must consume this
//! evidence separately, after review and input/checkpoint verification.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Binding {
    schema_version: u32,
    // Destination identities are non-owning provenance on the next restore,
    // just like the original managedModel. They do not install any model.
    package: PlacementRecord,
    compatibility: ProbeReport,
    // Exactly one proof for every manifest entry, in manifest order, including
    // libraries not represented by a whisper/llama CLI argument.
    files: Vec<Fingerprint>,
}

pub(crate) enum Resolved {
    Whisper(WhisperConfig),
    Sherpa(SherpaConfig),
    Llama(LlamaConfig, usize),
}
impl Resolved {
    #[cfg(test)]
    pub(crate) fn execution(&self) -> Execution<'_> {
        match self {
            Self::Whisper(e) => Execution::Whisper(e),
            Self::Sherpa(e) => Execution::Sherpa(e),
            Self::Llama(e, budget) => Execution::Llama(e, *budget),
        }
    }
}

impl Binding {
    pub(crate) fn recheck(
        &self,
        storage: &Storage,
        models_root: &Path,
        spec: &JobSpec,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let again = prepare(
            storage,
            models_root,
            spec,
            &self.package.package_id,
            &self.package.review_hash,
            &self.compatibility.operation_id,
            cancelled,
        )?;
        if &again != self {
            return Err("確認後に再結合するモデルの証拠が変わっています".into());
        }
        Ok(())
    }

    // No filesystem work under the retry transaction. Admission has already
    // checked physical bytes and trial artifacts; recheck the exact current
    // device-local records, not the destination's unrelated profile heads.
    pub(in crate::storage) fn gate(&self, db: &Connection, spec: &JobSpec) -> Result<(), String> {
        with_execution(spec, |e| self.validate(&spec.profile_snapshot, e))?;
        self.package.validate()?;
        if self.validate_provenance()?.is_none() {
            return Err("再結合先のモデルは未配置です".into());
        }
        let (record, placed) =
            crate::storage::model_placements::load(db, &self.package.package_id)?
                .ok_or("再結合先の配置記録がありません")?;
        let saved = crate::storage::model_checks::latest(db, &self.package.package_id)?
            .ok_or("再結合先の互換性記録がありません")?;
        if !placed
            || record != self.package
            || saved.summary.state != CheckState::Succeeded
            || saved.summary.operation_id != self.compatibility.operation_id
            || saved.report.as_ref() != Some(&self.compatibility)
        {
            return Err("再試行の確定前に配置・互換性の証拠が変わっています".into());
        }
        Ok(())
    }

    // Pure structural validation is also used when transporting unavailable
    // history. It must not read native paths, query a local catalog, or execute.
    fn validate_provenance(&self) -> Result<Option<PathBuf>, String> {
        self.compatibility.validate_portable(&self.package)?;
        if self.schema_version != 1 || self.files.len() != self.package.files.len() {
            return Err("再結合したモデルの形式版・依存ファイル数が不正です".into());
        }
        let mut directory = None;
        let mut unavailable = None;
        for (declared, proof) in self.package.files.iter().zip(&self.files) {
            if proof.size != declared.actual_size || proof.sha256 != declared.actual_hash {
                return Err("再結合したモデルの容量・SHAが確認済み証拠と一致しません".into());
            }
            let portable =
                proof.path.to_str() == Some(format!("mon-unavailable:{}", proof.sha256).as_str());
            if unavailable
                .replace(portable)
                .is_some_and(|old| old != portable)
            {
                return Err("再結合したモデルの配置済み・未配置参照が混在しています".into());
            }
            if !portable {
                if !proof.path.is_absolute()
                    || proof
                        .path
                        .to_str()
                        .is_none_or(|p| p.contains(['\0', '\n', '\r']) || p.len() > 4096)
                    || proof
                        .path
                        .components()
                        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
                {
                    return Err("再結合したモデルの絶対パスが不正です".into());
                }
                let root = package_directory(
                    &proof.path,
                    &declared.relative_path,
                    &self.package.package_id,
                )
                .ok_or("再結合したモデルの配置先・相対名が一致しません")?;
                if directory
                    .replace(root.to_owned())
                    .is_some_and(|old| old != root)
                {
                    return Err("再結合したモデルの同一パッケージ参照が一致しません".into());
                }
            }
        }
        Ok(directory)
    }

    // Returns the target manifest indices for the original CLI proofs. The
    // original profile/model identity, parameters, hint and budget stay fixed.
    fn mapping(&self, profile: &Value, execution: Execution<'_>) -> Result<Vec<usize>, String> {
        self.validate_provenance()?;
        execution.validate_settings()?;
        name(
            profile["profile"]
                .as_str()
                .ok_or("固定プロファイルがありません")?,
        )?;
        if self.package.manifest.engine != execution.engine() {
            return Err("再結合するモデルの工程が固定条件と一致しません".into());
        }
        let origin = super::read(profile)?;
        if let Some(origin) = &origin {
            origin.validate(profile["profile"].as_str().unwrap(), &execution)?;
            // Only the installation identity may change. Version, target,
            // names/roles, optional expected hashes, all runtime dependencies,
            // license declaration and the reviewed manifest remain identical.
            if origin.package.manifest != self.package.manifest
                || origin.package.source_manifest_hash != self.package.source_manifest_hash
                || origin.package.review_hash != self.package.review_hash
                || origin.package.files != self.package.files
            {
                return Err(
                    "再配置したパッケージの構成・版・依存ファイルが固定した証拠と一致しません"
                        .into(),
                );
            }
        }
        let declared = self
            .package
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.role != Role::RuntimeLibrary
                    || execution.engine() == Engine::Sherpa
                    || origin.is_none()
            })
            .collect::<Vec<_>>();
        let proofs = execution.files();
        // Legacy jobs have no evidence for undeclared dependencies. Do not
        // introduce a VAD or dynamic library absent from their fixed config.
        if proofs.len() != declared.len() {
            return Err("再結合で固定条件にないモデル・依存ファイルを追加・削除できません".into());
        }
        let mut used = BTreeSet::new();
        let mut indices = Vec::new();
        for (role, proof) in proofs {
            let portable =
                proof.path.to_str() == Some(format!("mon-unavailable:{}", proof.sha256).as_str());
            let index = declared
                .iter()
                .find_map(|(i, file)| {
                    (file.role == role
                        && file.actual_size == proof.size
                        && file.actual_hash == proof.sha256
                        && !used.contains(i)
                        && origin.as_ref().is_none_or(|o| {
                            portable
                                || package_directory(
                                    &proof.path,
                                    &file.relative_path,
                                    &o.package.package_id,
                                )
                                .is_some()
                        }))
                    .then_some(*i)
                })
                .ok_or("再結合するファイルの容量・SHA・役割が固定条件と一致しません")?;
            used.insert(index);
            indices.push(index);
        }
        Ok(indices)
    }

    pub(crate) fn validate(&self, profile: &Value, execution: Execution<'_>) -> Result<(), String> {
        self.mapping(profile, execution).map(|_| ())
    }

    // Pure path substitution on an owned clone; no numeric/hint adjustment and
    // no mutation to the persisted logical parameters or source job.
    pub(crate) fn resolve(
        &self,
        profile: &Value,
        execution: Execution<'_>,
    ) -> Result<Resolved, String> {
        let indices = self.mapping(profile, execution)?;
        if self.validate_provenance()?.is_none() {
            return Err("再結合したモデルは未配置です。再配置を確認してください".into());
        }
        let mut proofs = indices.into_iter().map(|i| self.files[i].clone());
        Ok(match execution {
            Execution::Whisper(e) => {
                let mut c = e.clone();
                c.executable = proofs.next().unwrap();
                c.model = proofs.next().unwrap();
                c.vad_model = if c.vad_model.is_some() {
                    Some(proofs.next().unwrap())
                } else {
                    None
                };
                Resolved::Whisper(c)
            }
            Execution::Sherpa(e) => {
                let mut c = e.clone();
                c.executable = proofs.next().unwrap();
                c.segmentation_model = proofs.next().unwrap();
                c.embedding_model = proofs.next().unwrap();
                c.runtime_libraries = proofs.collect();
                Resolved::Sherpa(c)
            }
            Execution::Llama(e, budget) => {
                let mut c = e.clone();
                c.executable = proofs.next().unwrap();
                c.model = proofs.next().unwrap();
                Resolved::Llama(c, budget)
            }
        })
    }

    pub(crate) fn verify(
        &self,
        profile: &Value,
        execution: Execution<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.validate(profile, execution)?;
        let directory = self
            .validate_provenance()?
            .ok_or("再結合したモデルは未配置です")?;
        crate::model_compatibility::verify_fixed_package(
            directory.parent().ok_or("モデル管理領域がありません")?,
            &self.package,
            cancelled,
        )
    }

    // A reviewed prospective retry profile, not authorization to mutate an
    // existing job or bypass normal enqueue's current-selection gate.
    pub(crate) fn attach(&self, spec: &JobSpec) -> Result<Value, String> {
        with_execution(spec, |e| self.validate(&spec.profile_snapshot, e))?;
        let mut profile = spec.profile_snapshot.clone();
        profile["runtimeModel"] = serde_json::to_value(self).map_err(|e| e.to_string())?;
        Ok(profile)
    }
}

pub(crate) fn read(profile: &Value) -> Result<Option<Binding>, String> {
    profile
        .get("runtimeModel")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())
}

pub(crate) fn base_profile(profile: &Value) -> Result<Value, String> {
    read(profile)?; // Reject null, unknown fields and malformed typed proofs.
    let mut base = profile.clone();
    base.as_object_mut()
        .ok_or("固定プロファイルが不正です")?
        .remove("runtimeModel");
    Ok(base)
}

pub(in crate::storage) fn validate_spec(spec: &JobSpec) -> Result<(), String> {
    with_execution(spec, |e| super::validate(&spec.profile_snapshot, e))
}

// Path-free review data. Never include the vocabulary text or transcript.
pub(crate) fn conditions(spec: &JobSpec) -> Result<Value, String> {
    with_execution(spec, |e| {
        e.validate_settings()?;
        super::validate(&spec.profile_snapshot, e)?;
        let hint = match e {
            Execution::Whisper(w) => {
                json!({"characters":w.prompt.chars().count(),"sha256":format!("{:x}",Sha256::digest(w.prompt.as_bytes()))})
            }
            _ => Value::Null,
        };
        Ok(
            json!({"profile":spec.profile_snapshot["profile"],"engine":e.engine(),"configuration":e.configuration(),"vocabularyHint":hint,"trackSettings":if spec.kind == JobKind::Diarization { spec.parameters["diarization"]["tracks"].clone() } else { Value::Null }}),
        )
    })
}

pub(in crate::storage) fn same_profile(left: &JobSpec, right: &JobSpec) -> Result<bool, String> {
    if left.profile_snapshot.get("runtimeModel").is_none()
        && right.profile_snapshot.get("runtimeModel").is_none()
    {
        return Ok(left.profile_snapshot == right.profile_snapshot);
    }
    for spec in [left, right] {
        with_execution(spec, |e| super::validate(&spec.profile_snapshot, e))?;
    }
    Ok(base_profile(&left.profile_snapshot)? == base_profile(&right.profile_snapshot)?)
}

pub(crate) fn runtime(profile: &Value, execution: Execution<'_>) -> Result<Resolved, String> {
    super::validate(profile, execution)?;
    if let Some(binding) = read(profile)? {
        return binding.resolve(profile, execution);
    }
    Ok(match execution {
        Execution::Whisper(e) => Resolved::Whisper(e.clone()),
        Execution::Sherpa(e) => Resolved::Sherpa(e.clone()),
        Execution::Llama(e, budget) => Resolved::Llama(e.clone(), budget),
    })
}
pub(crate) fn whisper(profile: &Value, engine: &WhisperConfig) -> Result<WhisperConfig, String> {
    let Resolved::Whisper(e) = runtime(profile, Execution::Whisper(engine))? else {
        unreachable!()
    };
    Ok(e)
}
pub(crate) fn sherpa(profile: &Value, engine: &SherpaConfig) -> Result<SherpaConfig, String> {
    let Resolved::Sherpa(e) = runtime(profile, Execution::Sherpa(engine))? else {
        unreachable!()
    };
    Ok(e)
}
pub(crate) fn llama(
    profile: &Value,
    engine: &LlamaConfig,
    budget: usize,
) -> Result<LlamaConfig, String> {
    let Resolved::Llama(e, resolved_budget) = runtime(profile, Execution::Llama(engine, budget))?
    else {
        unreachable!()
    };
    if resolved_budget != budget {
        return Err("再結合で固定した入力予算が変更されています".into());
    }
    Ok(e)
}

/// Existing explicit CPU success is required for the destination, but never
/// rerun here. The entire package and trial artifacts are reread off the DB
/// lock. A final reread detects intervening trial changes; retry admission
/// must recheck the current destination proof again at its commit boundary.
pub(crate) fn prepare(
    storage: &Storage,
    models_root: &Path,
    spec: &JobSpec,
    package: &str,
    review: &str,
    check: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Binding, String> {
    if cancelled() {
        return Err("モデルの再結合確認を中断しました".into());
    }
    with_execution(spec, |e| super::validate(&spec.profile_snapshot, e))?;
    let (record, directory) = crate::model_compatibility::verify_success(
        storage,
        models_root,
        check,
        package,
        review,
        cancelled,
    )?;
    let (_, report) = storage
        .model_check_record(check)?
        .ok_or("互換性の成功記録がありません")?;
    let compatibility = report.ok_or("互換性の成功証拠がありません")?;
    let files = record
        .files
        .iter()
        .map(|f| Fingerprint::capture(&directory.join(&f.relative_path), cancelled))
        .collect::<Result<Vec<_>, _>>()?;
    let binding = Binding {
        schema_version: 1,
        package: record,
        compatibility,
        files,
    };
    with_execution(spec, |e| {
        binding.verify(&spec.profile_snapshot, e, cancelled)
    })?;
    let (again, _) = crate::model_compatibility::verify_success(
        storage,
        models_root,
        check,
        package,
        review,
        cancelled,
    )?;
    let (summary, report) = storage
        .model_check_record(check)?
        .ok_or("互換性の成功記録がありません")?;
    if again != binding.package
        || summary.state != CheckState::Succeeded
        || report.as_ref() != Some(&binding.compatibility)
    {
        return Err("再結合の確認中にモデル・互換性の証拠が変わっています".into());
    }
    if cancelled() {
        return Err("モデルの再結合確認を中断しました".into());
    }
    Ok(binding)
}

// Unlike managedModel, runtimeModel has native paths. Preserve provenance but
// redact every proof and derive its historical name/role from the manifest,
// even after repeated backups of an already-unavailable binding.
pub(in crate::storage) fn portable(
    value: &Value,
) -> Result<
    (
        Value,
        Vec<super::super::super::model_metadata::ModelFileMetadata>,
    ),
    String,
> {
    let mut binding: Binding = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    binding.validate_provenance()?;
    let mut references = Vec::new();
    for (file, proof) in binding.package.files.iter().zip(&mut binding.files) {
        let metadata = super::super::super::model_metadata::ModelFileMetadata {
            name: Path::new(&file.relative_path)
                .file_name()
                .and_then(|s| s.to_str())
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
        references.push(metadata);
        proof.path = format!("mon-unavailable:{}", proof.sha256).into();
    }
    binding.validate_provenance()?;
    Ok((
        serde_json::to_value(binding).map_err(|e| e.to_string())?,
        references,
    ))
}
