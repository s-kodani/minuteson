//! Path-free catalog construction for portable backup. Origin UUIDs and hashes
//! are provenance, never destination foreign keys or execution capabilities.
//! No model bytes, installed state, CPU probe, device, or current-head mutation.
use super::*;

pub(in crate::storage) mod history;

const MAX_RECORDS: i64 = 65_536;
const MAX_BYTES: usize = 32 * 1024 * 1024;

fn budget(value: &impl Serialize, used: &mut usize) -> Result<(), String> {
    *used = used
        .checked_add(serde_json::to_vec(value).map_err(|e| e.to_string())?.len() + 1)
        .ok_or("持ち運び用モデル構成の容量が範囲外です")?;
    if *used > MAX_BYTES {
        return Err("持ち運び用モデル構成が容量上限を超えています".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, tag = "engine")]
enum Configuration {
    #[serde(rename = "whisper.cpp", rename_all = "camelCase")]
    Whisper { threads: u32, use_gpu: bool },
    #[serde(rename = "sherpa-onnx", rename_all = "camelCase")]
    Sherpa {
        threads: u32,
        cluster_threshold_milli: u32,
        window_shift_milli: u32,
        min_duration_on_ms: u32,
        min_duration_off_ms: u32,
    },
    #[serde(rename = "llama.cpp", rename_all = "camelCase")]
    Llama {
        threads: u32,
        context_tokens: u32,
        prediction_tokens: u32,
        gpu_layers: u32,
        seed: u32,
        prompt_budget: usize,
    },
}
impl Configuration {
    fn from_settings(settings: &Settings) -> Self {
        match settings {
            Settings::Whisper(s) => Self::Whisper {
                threads: s.threads,
                use_gpu: s.use_gpu,
            },
            Settings::Sherpa(s) => Self::Sherpa {
                threads: s.threads,
                cluster_threshold_milli: s.cluster_threshold_milli,
                window_shift_milli: s.window_shift_milli,
                min_duration_on_ms: s.min_duration_on_ms,
                min_duration_off_ms: s.min_duration_off_ms,
            },
            Settings::Llama(s) => Self::Llama {
                threads: s.threads,
                context_tokens: s.context_tokens,
                prediction_tokens: s.prediction_tokens,
                gpu_layers: s.gpu_layers,
                seed: s.seed,
                prompt_budget: s.prompt_budget,
            },
        }
    }
    fn settings(&self) -> Settings {
        // Empty paths allow numerical validation by the actual preferences
        // validators. They are not selected model references on this host.
        match *self {
            Self::Whisper { threads, use_gpu } => {
                Settings::Whisper(preferences::EnginePreferences {
                    threads,
                    use_gpu,
                    ..Default::default()
                })
            }
            Self::Sherpa {
                threads,
                cluster_threshold_milli,
                window_shift_milli,
                min_duration_on_ms,
                min_duration_off_ms,
            } => Settings::Sherpa(diarization_preferences::DiarizationEnginePreferences {
                threads,
                cluster_threshold_milli,
                window_shift_milli,
                min_duration_on_ms,
                min_duration_off_ms,
                ..Default::default()
            }),
            Self::Llama {
                threads,
                context_tokens,
                prediction_tokens,
                gpu_layers,
                seed,
                prompt_budget,
            } => Settings::Llama(minutes_preferences::MinutesEnginePreferences {
                threads,
                context_tokens,
                prediction_tokens,
                gpu_layers,
                seed,
                prompt_budget,
                ..Default::default()
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Component {
    source_package_id: String,
    package_review_hash: String,
    configuration: Configuration,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Profile {
    source_profile_id: String,
    name: ProfileName,
    profile_version: i64,
    // These source hashes include the original host binding. Retain as opaque
    // provenance only; they cannot be recomputed from this path-free record.
    source_content_hash: String,
    source_application_hash: String,
    was_current: bool,
    components: Vec<Component>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Preferences {
    name: ProfileName,
    configurations: Vec<Configuration>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(in crate::storage) struct Catalog {
    schema_version: u32,
    packages: Vec<PlacementRecord>,
    profiles: Vec<Profile>,
    preferences: Vec<Preferences>,
}
impl Catalog {
    pub(in crate::storage) fn file_references(
        &self,
    ) -> Result<Vec<super::super::model_metadata::ModelFileMetadata>, String> {
        self.packages
            .iter()
            .flat_map(|p| &p.files)
            .map(|file| {
                let reference = super::super::model_metadata::ModelFileMetadata {
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
                reference.validate()?;
                Ok(reference)
            })
            .collect()
    }
    pub(in crate::storage) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.packages.len() > MAX_RECORDS as usize
            || self.profiles.len() > MAX_RECORDS as usize
            || self.preferences.len() != 2
        {
            return Err("持ち運び用モデル構成の版・件数が不正です".into());
        }
        let mut packages = BTreeMap::new();
        for package in &self.packages {
            package.validate_portable()?;
            if packages.insert(&package.package_id, package).is_some() {
                return Err("持ち運び用モデルの由来IDが重複しています".into());
            }
        }
        let mut ids = BTreeSet::new();
        let mut versions = BTreeSet::new();
        let mut current = BTreeMap::new();
        let mut latest = BTreeMap::new();
        for profile in &self.profiles {
            if !canonical_id(&profile.source_profile_id)
                || packages.contains_key(&profile.source_profile_id)
                || !ids.insert(&profile.source_profile_id)
                || profile.profile_version <= 0
                || !versions.insert((profile.name.text(), profile.profile_version))
                || !hash(&profile.source_content_hash)
                || !hash(&profile.source_application_hash)
                || profile.components.is_empty()
                || profile.components.len() > 3
            {
                return Err("持ち運び用モデル構成の由来・版が不正です".into());
            }
            let name = profile.name.text();
            let maximum = latest.entry(name).or_insert(0);
            *maximum = (*maximum).max(profile.profile_version);
            if profile.was_current && current.insert(name, profile.profile_version).is_some() {
                return Err("持ち運び用モデル構成の元現在版が重複しています".into());
            }
            let mut engines = BTreeSet::new();
            for component in &profile.components {
                let package = packages
                    .get(&component.source_package_id)
                    .ok_or("持ち運び用構成のモデル記録が欠損しています")?;
                let settings = component.configuration.settings();
                settings.validate()?;
                if package.manifest.engine != settings.engine()
                    || component.package_review_hash != package.review_hash
                    || !engines.insert(settings.key())
                {
                    return Err("持ち運び用構成のモデルSHA・エンジンが一致しません".into());
                }
            }
        }
        if current != latest {
            return Err("持ち運び用モデル構成の元現在版が最新の構成版と一致しません".into());
        }
        let mut names = BTreeSet::new();
        for preferences in &self.preferences {
            if !names.insert(preferences.name.text()) || preferences.configurations.len() != 3 {
                return Err("持ち運び用の標準・高精度設定が欠損・重複しています".into());
            }
            let mut engines = BTreeSet::new();
            for configuration in &preferences.configurations {
                let settings = configuration.settings();
                settings.validate()?;
                if !engines.insert(settings.key()) {
                    return Err("持ち運び用の数値設定にエンジンの重複があります".into());
                }
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_BYTES {
            return Err("持ち運び用モデル構成が容量上限を超えています".into());
        }
        Ok(())
    }
}

/// Read only the committed SQLite image. Never open models or execute engines;
/// unavailable/unused packages and every superseded profile remain metadata.
pub(in crate::storage) fn export(
    db: &Connection,
    cancelled: &impl Fn() -> bool,
) -> Result<Catalog, String> {
    let check = || {
        if cancelled() {
            Err("モデル構成の持ち運び準備を取り消しました".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    for table in ["model_packages", "model_profiles"] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count > MAX_RECORDS {
            return Err("持ち運び用モデル構成が件数上限を超えています".into());
        }
    }
    // Validate legacy single-file historical rows separately, never attempt to
    // deserialize them as managed placements or infer missing engine metadata.
    super::super::model_metadata::historical(db)?;
    let rows = db
        .prepare("SELECT id,source_type FROM model_packages ORDER BY id")
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut packages = Vec::new();
    let mut used_bytes = 128;
    for (id, source) in rows {
        check()?;
        match source.as_str() {
            "historical_snapshot" => {}
            "manual_import" | "download" => {
                let (record, _) = super::super::model_placements::load(db, &id)?
                    .ok_or("持ち運び用のモデル配置記録が欠損しています")?;
                budget(&record, &mut used_bytes)?;
                packages.push(record);
            }
            _ => return Err("未対応のモデル導入元の履歴は省略せず保全してください".into()),
        }
    }
    let (heads, _) = heads(db)?;
    for name in [ProfileName::Standard, ProfileName::HighAccuracy] {
        if let Some(head) = heads.get(name) {
            load_profile(db, name, head)?;
        }
    }
    let rows = db
        .prepare("SELECT id,name FROM model_profiles ORDER BY name,profile_version,id")
        .map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut profiles = Vec::new();
    for (id, name) in rows {
        check()?;
        let name = match name.as_str() {
            "standard" => ProfileName::Standard,
            "high_accuracy" => ProfileName::HighAccuracy,
            _ => return Err("未対応のモデル構成名です".into()),
        };
        let stored = read_profile(db, name, &id)?;
        let profile = Profile {
            source_profile_id: id.clone(),
            name,
            profile_version: stored.version,
            source_content_hash: digest(&stored)?,
            source_application_hash: stored.application_hash,
            was_current: heads.get(name).is_some_and(|head| head.id == id),
            components: stored
                .bindings
                .into_iter()
                .map(|binding| Component {
                    source_package_id: binding.package_id,
                    package_review_hash: binding.review_hash,
                    configuration: Configuration::from_settings(&binding.settings),
                })
                .collect(),
        };
        budget(&profile, &mut used_bytes)?;
        profiles.push(profile);
    }
    let mut preferences = Vec::new();
    for name in [ProfileName::Standard, ProfileName::HighAccuracy] {
        check()?;
        let preference = Preferences {
            name,
            configurations: [Engine::Whisper, Engine::Sherpa, Engine::Llama]
                .into_iter()
                .map(|engine| {
                    let settings = settings(db, name, engine)?.0;
                    settings.validate()?;
                    Ok(Configuration::from_settings(&settings))
                })
                .collect::<Result<_, String>>()?,
        };
        budget(&preference, &mut used_bytes)?;
        preferences.push(preference);
    }
    let catalog = Catalog {
        schema_version: 1,
        packages,
        profiles,
        preferences,
    };
    catalog.validate()?;
    check()?;
    Ok(catalog)
}
