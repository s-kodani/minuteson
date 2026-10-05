//! Reviewed, atomic application to the existing engine preferences. These are
//! immutable configuration records, not quality or current filesystem health.
use super::*;
use crate::model_compatibility::{canonical_id, CheckState};
use crate::model_inspection::{Engine, Role};
use crate::model_placement::PlacementRecord;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod jobs;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProfileName {
    Standard,
    HighAccuracy,
}
impl ProfileName {
    fn text(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::HighAccuracy => "high_accuracy",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, tag = "engine", content = "settings")]
pub(crate) enum Settings {
    #[serde(rename = "whisper.cpp")]
    Whisper(preferences::EnginePreferences),
    #[serde(rename = "sherpa-onnx")]
    Sherpa(diarization_preferences::DiarizationEnginePreferences),
    #[serde(rename = "llama.cpp")]
    Llama(minutes_preferences::MinutesEnginePreferences),
}
impl Settings {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Whisper(p) => super::preferences::validate(&AnalysisPreferences {
                standard: p.clone(),
                high_accuracy: p.clone(),
            }),
            Self::Sherpa(p) => super::diarization_preferences::validate(&DiarizationPreferences {
                standard: p.clone(),
                high_accuracy: p.clone(),
            }),
            Self::Llama(p) => super::minutes_preferences::validate(&MinutesPreferences {
                standard: p.clone(),
                high_accuracy: p.clone(),
            }),
        }
    }
    fn engine(&self) -> Engine {
        match self {
            Self::Whisper(_) => Engine::Whisper,
            Self::Sherpa(_) => Engine::Sherpa,
            Self::Llama(_) => Engine::Llama,
        }
    }
    fn roles(&self) -> &'static [&'static str] {
        match self {
            Self::Whisper(_) => &["asr"],
            Self::Sherpa(_) => &["segmentation", "speaker_embedding"],
            Self::Llama(_) => &["llm"],
        }
    }
    fn key(&self) -> &'static str {
        match self {
            Self::Whisper(_) => "whisper_profiles",
            Self::Sherpa(_) => "diarization_profiles",
            Self::Llama(_) => "llama_profiles",
        }
    }
    fn configured(&self, record: &PlacementRecord, models_root: &Path) -> Result<Self, String> {
        let directory = models_root.join(&record.package_id);
        let path = |role: Role| -> Result<String, String> {
            let files = record
                .files
                .iter()
                .filter(|f| f.role == role)
                .collect::<Vec<_>>();
            if files.len() != 1 {
                return Err("適用するモデル構成が不足・重複しています".into());
            }
            directory
                .join(&files[0].relative_path)
                .to_str()
                .map(str::to_owned)
                .ok_or("モデル配置パスを保存できません".into())
        };
        if self.engine() != record.manifest.engine {
            return Err("適用するエンジン種別が一致しません".into());
        }
        let executable = path(Role::Executable)?;
        Ok(match self {
            Self::Whisper(before) => {
                let mut p = before.clone();
                p.executable_path = executable;
                p.model_path = path(Role::Asr)?;
                p.vad_model_path = if record.files.iter().any(|f| f.role == Role::Vad) {
                    Some(path(Role::Vad)?)
                } else {
                    None
                };
                Self::Whisper(p)
            }
            Self::Sherpa(before) => {
                let mut p = before.clone();
                p.executable_path = executable;
                p.segmentation_model_path = path(Role::Segmentation)?;
                p.embedding_model_path = path(Role::SpeakerEmbedding)?;
                p.runtime_library_paths = record
                    .files
                    .iter()
                    .filter(|f| f.role == Role::RuntimeLibrary)
                    .map(|f| {
                        directory
                            .join(&f.relative_path)
                            .to_str()
                            .map(str::to_owned)
                            .ok_or("ランタイム配置パスを保存できません".into())
                    })
                    .collect::<Result<_, String>>()?;
                Self::Sherpa(p)
            }
            Self::Llama(before) => {
                let mut p = before.clone();
                p.executable_path = executable;
                p.model_path = path(Role::Llm)?;
                Self::Llama(p)
            }
        })
    }
}
fn digest<T: Serialize>(value: &T) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|e| e.to_string())?)
    ))
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("モデルの適用を取り消しました".into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Binding {
    schema_version: u32,
    package_id: String,
    review_hash: String,
    check_operation_id: String,
    models_root: PathBuf,
    settings: Settings,
}
impl Binding {
    fn validate(&self, db: &Connection) -> Result<PlacementRecord, String> {
        self.settings.validate()?;
        if self.schema_version != 1
            || !canonical_id(&self.package_id)
            || !canonical_id(&self.check_operation_id)
            || !hash(&self.review_hash)
            || !self.models_root.is_absolute()
            || self
                .models_root
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
            || self.models_root.file_name() != Some(std::ffi::OsStr::new("models"))
        {
            return Err("プロファイルのモデル参照が不正です".into());
        }
        let (record, placed) = super::model_placements::load(db, &self.package_id)?
            .ok_or("プロファイルのモデル記録が欠損しています")?;
        if !placed
            || record.review_hash != self.review_hash
            || self.settings.configured(&record, &self.models_root)? != self.settings
        {
            return Err("プロファイルと配置モデルの構成が一致しません".into());
        }
        let check = super::model_checks::read(db, &self.check_operation_id)?
            .ok_or("プロファイルの互換性証拠が欠損しています")?;
        if check.summary.state != CheckState::Succeeded
            || check.summary.package_id != self.package_id
            || check.summary.review_hash != self.review_hash
        {
            return Err("プロファイルの互換性証拠が一致しません".into());
        }
        check
            .report
            .ok_or("プロファイルの成功証拠がありません")?
            .validate(&record)?;
        Ok(record)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ComponentRecord {
    schema_version: u32,
    application_hash: String,
    binding: Binding,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Head {
    id: String,
    content_hash: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Heads {
    standard: Option<Head>,
    high_accuracy: Option<Head>,
}
impl Heads {
    fn get(&self, name: ProfileName) -> Option<&Head> {
        match name {
            ProfileName::Standard => self.standard.as_ref(),
            ProfileName::HighAccuracy => self.high_accuracy.as_ref(),
        }
    }
    fn set(&mut self, name: ProfileName, head: Head) {
        match name {
            ProfileName::Standard => self.standard = Some(head),
            ProfileName::HighAccuracy => self.high_accuracy = Some(head),
        }
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct StoredProfile {
    id: String,
    name: ProfileName,
    version: i64,
    state: String,
    application_hash: String,
    bindings: Vec<Binding>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ComponentView {
    pub roles: Vec<String>,
    pub package_id: String,
    pub model_name: String,
    pub model_version: String,
    pub engine: Engine,
    pub check_operation_id: String,
    pub settings_match: bool,
    pub compatibility_current: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ProfileView {
    pub name: ProfileName,
    pub id: Option<String>,
    pub profile_version: i64,
    pub components: Vec<ComponentView>,
    pub configuration_complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApplyPreview {
    pub schema_version: u32,
    pub profile: ProfileName,
    pub package_id: String,
    pub review_hash: String,
    pub check_operation_id: String,
    pub expected_profile_id: Option<String>,
    pub expected_profile_hash: Option<String>,
    pub expected_profile_version: i64,
    pub expected_preferences_version: i64,
    pub before: Settings,
    pub after: Settings,
    pub review_digest: String,
}
fn heads(db: &Connection) -> Result<(Heads, i64), String> {
    let row:Option<(String,String,i64)>=db.query_row("SELECT value_json,content_hash,lock_version FROM app_settings WHERE key='managed_profile_heads'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let Some((text, h, version)) = row else {
        if db
            .query_row("SELECT EXISTS(SELECT 1 FROM model_profiles)", [], |r| {
                r.get::<_, bool>(0)
            })
            .map_err(|e| e.to_string())?
        {
            return Err("モデル構成の参照記録が欠損しています".into());
        }
        return Ok((Heads::default(), 0));
    };
    if text.len() > 16384 || version < 0 || format!("{:x}", Sha256::digest(text.as_bytes())) != h {
        return Err("モデル構成の参照・内容ハッシュが不正です".into());
    }
    let heads: Heads = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for head in [heads.standard.as_ref(), heads.high_accuracy.as_ref()]
        .into_iter()
        .flatten()
    {
        if !canonical_id(&head.id) || !hash(&head.content_hash) {
            return Err("モデル構成の参照ID・SHAが不正です".into());
        }
    }
    for name in [ProfileName::Standard, ProfileName::HighAccuracy] {
        if heads.get(name).is_none()
            && db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM model_profiles WHERE name=?1)",
                    params![name.text()],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?
        {
            return Err("モデル構成の現在版の参照が欠損しています".into());
        }
    }
    Ok((heads, version))
}
fn settings(db: &Connection, name: ProfileName, engine: Engine) -> Result<(Settings, i64), String> {
    let high = name == ProfileName::HighAccuracy;
    let (value, version) = match engine {
        Engine::Whisper => {
            let p = super::preferences::load(db)?;
            (
                Settings::Whisper(if high {
                    p.payload.high_accuracy
                } else {
                    p.payload.standard
                }),
                p.lock_version,
            )
        }
        Engine::Sherpa => {
            let p = super::diarization_preferences::load(db)?;
            (
                Settings::Sherpa(if high {
                    p.payload.high_accuracy
                } else {
                    p.payload.standard
                }),
                p.lock_version,
            )
        }
        Engine::Llama => {
            let p = super::minutes_preferences::load(db)?;
            (
                Settings::Llama(if high {
                    p.payload.high_accuracy
                } else {
                    p.payload.standard
                }),
                p.lock_version,
            )
        }
    };
    if version < 0 {
        return Err("解析設定の版番号が不正です".into());
    }
    Ok((value, version))
}
fn load_profile(db: &Connection, name: ProfileName, head: &Head) -> Result<StoredProfile, String> {
    let row: Option<(String, i64, String)> = db
        .query_row(
            "SELECT name,profile_version,state FROM model_profiles WHERE id=?1",
            params![head.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (stored_name, version, state) = row.ok_or("モデル構成の版が欠損しています")?;
    if stored_name != name.text()
        || version <= 0
        || !matches!(state.as_str(), "ready" | "unavailable")
    {
        return Err("モデル構成の名前・確定状態が不正です".into());
    }
    let latest: i64 = db
        .query_row(
            "SELECT max(profile_version) FROM model_profiles WHERE name=?1",
            params![name.text()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if latest != version {
        return Err("モデル構成の現在版が古い版へ差し戻されています".into());
    }
    let rows=db.prepare("SELECT role,package_id,settings_json FROM profile_components WHERE profile_id=?1 ORDER BY role").map_err(|e|e.to_string())?.query_map(params![head.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    if rows.is_empty() || rows.len() > 4 {
        return Err("モデル構成要素の数が不正です".into());
    }
    let mut bindings = BTreeMap::new();
    let mut roles = BTreeSet::new();
    let mut application = None;
    for (role, package, json) in rows {
        if json.len() > 65536 {
            return Err("モデル構成要素が大きすぎます".into());
        }
        let component: ComponentRecord = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let binding = component.binding;
        if component.schema_version != 1
            || !hash(&component.application_hash)
            || binding.package_id != package
            || !binding.settings.roles().contains(&role.as_str())
            || !roles.insert(role)
        {
            return Err("モデル構成要素の由来・役割が不正です".into());
        }
        if application
            .as_ref()
            .is_some_and(|h| h != &component.application_hash)
        {
            return Err("モデル適用の確認内容が一致しません".into());
        }
        application = Some(component.application_hash);
        binding.validate(db)?;
        if let Some(old) = bindings.insert(binding.settings.roles()[0].to_owned(), binding.clone())
        {
            if old != binding {
                return Err("話者分離の両モデルの構成が一致しません".into());
            }
        }
    }
    let bindings = bindings.into_values().collect::<Vec<_>>();
    if roles
        != bindings
            .iter()
            .flat_map(|b| b.settings.roles().iter().map(|s| (*s).to_owned()))
            .collect()
    {
        return Err("モデル構成要素の役割が欠損しています".into());
    }
    let stored = StoredProfile {
        id: head.id.clone(),
        name,
        version,
        state,
        application_hash: application.unwrap(),
        bindings,
    };
    if digest(&stored)? != head.content_hash {
        return Err("モデル構成の内容SHAが一致しません".into());
    }
    Ok(stored)
}
fn view(
    db: &Connection,
    name: ProfileName,
    stored: Option<&StoredProfile>,
) -> Result<ProfileView, String> {
    let mut components = Vec::new();
    if let Some(stored) = stored {
        for binding in &stored.bindings {
            let record = binding.validate(db)?;
            let latest = super::model_checks::latest(db, &binding.package_id)?
                .ok_or("最新の互換性確認が欠損しています")?;
            components.push(ComponentView {
                roles: binding
                    .settings
                    .roles()
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
                package_id: binding.package_id.clone(),
                model_name: record.manifest.model_name,
                model_version: record.manifest.model_version,
                engine: record.manifest.engine,
                check_operation_id: binding.check_operation_id.clone(),
                settings_match: settings(db, name, binding.settings.engine())?.0
                    == binding.settings,
                compatibility_current: latest.summary.state == CheckState::Succeeded
                    && latest.summary.operation_id == binding.check_operation_id,
            });
        }
    }
    let complete = components.len() == 3
        && components
            .iter()
            .all(|c| c.settings_match && c.compatibility_current);
    Ok(ProfileView {
        name,
        id: stored.map(|p| p.id.clone()),
        profile_version: stored.map_or(0, |p| p.version),
        components,
        configuration_complete: complete,
    })
}
fn preview(
    db: &Connection,
    name: ProfileName,
    record: &PlacementRecord,
    models_root: &Path,
    check: &str,
) -> Result<ApplyPreview, String> {
    let binding = Binding {
        schema_version: 1,
        package_id: record.package_id.clone(),
        review_hash: record.review_hash.clone(),
        check_operation_id: check.into(),
        models_root: models_root.into(),
        settings: settings(db, name, record.manifest.engine)?
            .0
            .configured(record, models_root)?,
    };
    if binding.validate(db)? != *record {
        return Err("適用する配置記録が変わっています".into());
    }
    let latest =
        super::model_checks::latest(db, &record.package_id)?.ok_or("互換性確認がありません")?;
    if latest.summary.operation_id != check || latest.summary.state != CheckState::Succeeded {
        return Err("最新の成功した互換性確認が必要です".into());
    }
    let (heads, _) = heads(db)?;
    let head = heads.get(name);
    let stored = head.map(|h| load_profile(db, name, h)).transpose()?;
    let (before, version) = settings(db, name, record.manifest.engine)?;
    let mut preview = ApplyPreview {
        schema_version: 1,
        profile: name,
        package_id: record.package_id.clone(),
        review_hash: record.review_hash.clone(),
        check_operation_id: check.into(),
        expected_profile_id: head.map(|h| h.id.clone()),
        expected_profile_hash: head.map(|h| h.content_hash.clone()),
        expected_profile_version: stored.as_ref().map_or(0, |p| p.version),
        expected_preferences_version: version,
        before,
        after: binding.settings,
        review_digest: String::new(),
    };
    preview.review_digest = digest(&preview)?;
    Ok(preview)
}
fn persist<T: Serialize>(
    db: &Connection,
    key: &str,
    version: i64,
    payload: &T,
) -> Result<(), String> {
    let text = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    if text.len() > 192 * 1024 {
        return Err("モデル適用後の設定が大きすぎます".into());
    }
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    db.execute("INSERT INTO app_settings(key,value_json,content_hash,lock_version,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,content_hash=excluded.content_hash,lock_version=excluded.lock_version,updated_at=excluded.updated_at",params![key,text,hash,version,now_ms()?]).map_err(|e|e.to_string())?;
    Ok(())
}
fn save_selected(db: &Connection, preview: &ApplyPreview) -> Result<(), String> {
    if preview.before == preview.after {
        return Ok(());
    }
    let version = preview
        .expected_preferences_version
        .checked_add(1)
        .ok_or("解析設定の版番号が範囲外です")?;
    let high = preview.profile == ProfileName::HighAccuracy;
    match &preview.after {
        Settings::Whisper(p) => {
            let mut value = super::preferences::load(db)?.payload;
            if high {
                value.high_accuracy = p.clone();
            } else {
                value.standard = p.clone();
            }
            super::preferences::validate(&value)?;
            persist(db, preview.after.key(), version, &value)?;
        }
        Settings::Sherpa(p) => {
            let mut value = super::diarization_preferences::load(db)?.payload;
            if high {
                value.high_accuracy = p.clone();
            } else {
                value.standard = p.clone();
            }
            super::diarization_preferences::validate(&value)?;
            persist(db, preview.after.key(), version, &value)?;
        }
        Settings::Llama(p) => {
            let mut value = super::minutes_preferences::load(db)?.payload;
            if high {
                value.high_accuracy = p.clone();
            } else {
                value.standard = p.clone();
            }
            super::minutes_preferences::validate(&value)?;
            persist(db, preview.after.key(), version, &value)?;
        }
    }
    Ok(())
}
impl Storage {
    pub(crate) fn model_profiles(&self) -> Result<Vec<ProfileView>, String> {
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let (heads, _) = heads(&db)?;
        [ProfileName::Standard, ProfileName::HighAccuracy]
            .into_iter()
            .map(|name| {
                let stored = heads
                    .get(name)
                    .map(|h| load_profile(&db, name, h))
                    .transpose()?;
                view(&db, name, stored.as_ref())
            })
            .collect()
    }
    pub(crate) fn preview_model_profile(
        &self,
        name: ProfileName,
        record: &PlacementRecord,
        directory: &Path,
        check: &str,
    ) -> Result<ApplyPreview, String> {
        let root = directory.parent().ok_or("モデル管理領域がありません")?;
        if directory.file_name().and_then(|p| p.to_str()) != Some(&record.package_id) {
            return Err("モデル配置IDと実体が一致しません".into());
        }
        let db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        preview(&db, name, record, root, check)
    }
    pub(crate) fn apply_model_profile(
        &self,
        operation: &str,
        expected: &ApplyPreview,
        record: &PlacementRecord,
        directory: &Path,
        check_cancelled: &impl Fn() -> bool,
    ) -> Result<ProfileView, String> {
        if !canonical_id(operation)
            || expected.schema_version != 1
            || !hash(&expected.review_digest)
        {
            return Err("モデル適用の操作ID・確認内容が不正です".into());
        }
        let mut checked = expected.clone();
        checked.review_digest.clear();
        if digest(&checked)? != expected.review_digest {
            return Err("モデル適用の確認内容SHAが一致しません".into());
        }
        cancelled(check_cancelled)?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let (mut heads, heads_version) = heads(&tx)?;
        let old = heads
            .get(expected.profile)
            .map(|h| load_profile(&tx, expected.profile, h))
            .transpose()?;
        if let Some(old) = &old {
            if old.id == operation {
                if old.application_hash != expected.review_digest {
                    return Err("同じ適用操作の確認内容が一致しません".into());
                }
                let binding = old
                    .bindings
                    .iter()
                    .find(|b| {
                        b.package_id == expected.package_id
                            && b.check_operation_id == expected.check_operation_id
                            && b.settings == expected.after
                    })
                    .ok_or("適用済みの構成が一致しません")?;
                let latest = super::model_checks::latest(&tx, &binding.package_id)?
                    .ok_or("互換性確認がありません")?;
                if binding.validate(&tx)? != *record
                    || binding.review_hash != expected.review_hash
                    || latest.summary.state != CheckState::Succeeded
                    || latest.summary.operation_id != binding.check_operation_id
                    || binding.models_root.join(&binding.package_id) != directory
                    || settings(&tx, expected.profile, binding.settings.engine())?.0
                        != binding.settings
                {
                    return Err("適用後のモデル・設定が変わっています".into());
                }
                return view(&tx, expected.profile, Some(old));
            }
        }
        if !super::model_placements::unused_identity(&tx, operation)? {
            return Err("モデル適用IDが既存データ・古い適用と衝突しています".into());
        }
        let root = directory.parent().ok_or("モデル管理領域がありません")?;
        if directory.file_name().and_then(|p| p.to_str()) != Some(&record.package_id)
            || preview(
                &tx,
                expected.profile,
                record,
                root,
                &expected.check_operation_id,
            )? != *expected
        {
            return Err("確認後にモデル構成・解析設定が変わりました。再確認してください".into());
        }
        cancelled(check_cancelled)?;
        let mut bindings = old.as_ref().map_or_else(Vec::new, |p| p.bindings.clone());
        bindings.retain(|b| b.settings.engine() != record.manifest.engine);
        bindings.push(Binding {
            schema_version: 1,
            package_id: record.package_id.clone(),
            review_hash: record.review_hash.clone(),
            check_operation_id: expected.check_operation_id.clone(),
            models_root: root.into(),
            settings: expected.after.clone(),
        });
        bindings.sort_by_key(|b| b.settings.roles()[0]);
        let version = expected
            .expected_profile_version
            .checked_add(1)
            .ok_or("モデル構成の版番号が範囲外です")?;
        tx.execute("INSERT INTO model_profiles(id,name,profile_version,state) VALUES(?1,?2,?3,'preparing')",params![operation,expected.profile.text(),version]).map_err(|e|e.to_string())?;
        for binding in &bindings {
            let text = serde_json::to_string(&ComponentRecord {
                schema_version: 1,
                application_hash: expected.review_digest.clone(),
                binding: binding.clone(),
            })
            .map_err(|e| e.to_string())?;
            for role in binding.settings.roles() {
                tx.execute("INSERT INTO profile_components(profile_id,role,package_id,settings_json) VALUES(?1,?2,?3,?4)",params![operation,role,binding.package_id,text]).map_err(|e|e.to_string())?;
            }
            cancelled(check_cancelled)?;
        }
        save_selected(&tx, expected)?;
        let mut stored = StoredProfile {
            id: operation.into(),
            name: expected.profile,
            version,
            state: "unavailable".into(),
            application_hash: expected.review_digest.clone(),
            bindings,
        };
        let result = view(&tx, expected.profile, Some(&stored))?;
        if result.configuration_complete {
            stored.state = "ready".into();
        }
        tx.execute(
            "UPDATE model_profiles SET state=?1 WHERE id=?2",
            params![stored.state, operation],
        )
        .map_err(|e| e.to_string())?;
        heads.set(
            expected.profile,
            Head {
                id: operation.into(),
                content_hash: digest(&stored)?,
            },
        );
        persist(
            &tx,
            "managed_profile_heads",
            heads_version
                .checked_add(1)
                .ok_or("モデル構成参照の版番号が範囲外です")?,
            &heads,
        )?;
        cancelled(check_cancelled)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
}

#[cfg(test)]
pub(crate) fn remove_schema_for_test(db: &Connection) {
    super::jobs::remove_rebinding_schema_for_test(db);
    if db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=25)",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    {
        assert_eq!(
            db.query_row("SELECT count(*) FROM model_profiles", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("DROP TRIGGER model_profile_fixed; DROP TRIGGER model_profile_no_delete; DROP TRIGGER profile_component_insert; DROP TRIGGER profile_component_fixed; DROP TRIGGER profile_component_no_delete; DROP INDEX model_profile_version; DELETE FROM schema_migrations WHERE version=25;").unwrap();
    }
}
#[cfg(all(test, unix))]
pub(in crate::storage) mod tests;
