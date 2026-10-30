//! Read-only attribution information from this build's existing resources.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use tauri::{Manager, WebviewWindow};

const MAX_TEXT: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Credit {
    name: String,
    version: String,
    license: String,
    sources: Vec<String>,
}
#[derive(Debug, Serialize)]
pub(crate) struct Notice {
    title: String,
    text: String,
}
#[derive(Debug, Serialize)]
pub(crate) struct Credits {
    technologies: Vec<Credit>,
    models: Vec<Credit>,
    dependencies: Vec<Credit>,
    notices: Vec<Notice>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Inventory {
    schema_version: u32,
    target: String,
    notices_sha256: String,
    packages: Vec<Dependency>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dependency {
    ecosystem: String,
    name: String,
    version: String,
    declared_license: Option<String>,
    repository: Option<String>,
    source_archive: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Windows,
    Macos,
}
impl Target {
    fn triple(self) -> &'static str {
        match self {
            Self::Windows => "x86_64-pc-windows-msvc",
            Self::Macos => "aarch64-apple-darwin",
        }
    }
}
struct Paths {
    notices: PathBuf,
    runtime: Vec<PathBuf>,
    extra_notices: PathBuf,
}
fn paths(resources: &Path, repository: Option<&Path>, target: Target) -> Paths {
    if !resources.join("third-party-inventory.json").is_file() {
        if let Some(repository) = repository {
            let cache = repository.join(".cache");
            return match target {
                Target::Windows => Paths {
                    notices: cache.join("windows-third-party-notices"),
                    runtime: vec![cache.join("windows-runtime/package")],
                    extra_notices: cache.join("windows-capture-headers"),
                },
                Target::Macos => Paths {
                    notices: cache.join("third-party-notices"),
                    runtime: vec![
                        cache.join("whisper-metal-runtime/package"),
                        cache.join("inference-runtime/package"),
                    ],
                    extra_notices: resources.join("notices"),
                },
            };
        }
    }
    Paths {
        notices: resources.to_path_buf(),
        runtime: match target {
            Target::Windows => vec![resources.join("windows-runtime")],
            Target::Macos => vec![
                resources.join("asr-runtime"),
                resources.join("inference-runtime"),
            ],
        },
        extra_notices: resources.join("notices"),
    }
}
fn read_text(path: &Path, limit: u64) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| {
        format!(
            "クレジット資料を読めません: {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit {
        return Err("クレジット資料の形式・容量が不正です".into());
    }
    let mut text = String::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_string(&mut text)
        .map_err(|_| "クレジット資料をUTF-8で読めません")?;
    if text.len() as u64 > limit {
        return Err("クレジット資料が容量上限を超えています".into());
    }
    Ok(text)
}
fn require_main(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("クレジットはメイン画面で開いてください".into())
    }
}

#[tauri::command]
pub(crate) async fn get_credits(window: WebviewWindow) -> Result<Credits, mon_contracts::AppError> {
    let result: Result<Credits, String> = async move {
        require_main(window.label())?;
        let resources = window
            .app_handle()
            .path()
            .resource_dir()
            .map_err(|e| e.to_string())?;
        let target = if cfg!(windows) {
            Target::Windows
        } else {
            Target::Macos
        };
        let repository =
            cfg!(debug_assertions).then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        let paths = paths(&resources, repository.as_deref(), target);
        tauri::async_runtime::spawn_blocking(move || load(paths, target))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    result.map_err(|message| mon_contracts::AppError::from_command("get_credits", message))
}

fn load(paths: Paths, target: Target) -> Result<Credits, String> {
    let inventory: Inventory = serde_json::from_str(&read_text(
        &paths.notices.join("third-party-inventory.json"),
        4 * 1024 * 1024,
    )?)
    .map_err(|_| "依存ライブラリ一覧の形式が不正です")?;
    if inventory.schema_version != 1 || inventory.target != target.triple() {
        return Err("依存ライブラリ一覧がこのOSの形式と一致しません".into());
    }
    let dependency_notices = read_text(&paths.notices.join("THIRD_PARTY_NOTICES.txt"), MAX_TEXT)?;
    if format!("{:x}", Sha256::digest(dependency_notices.as_bytes())) != inventory.notices_sha256 {
        return Err("依存ライブラリの通知文と一覧が一致しません".into());
    }
    let mut technologies = Vec::new();
    let mut dependencies = Vec::new();
    for dependency in inventory.packages {
        let mut sources = vec![format!(
            "{}:{}@{}",
            dependency.ecosystem, dependency.name, dependency.version
        )];
        sources.extend(dependency.repository);
        if let Some(url) = dependency
            .source_archive
            .as_ref()
            .and_then(|v| v["url"].as_str())
        {
            sources.push(url.into());
        }
        let entry = Credit {
            name: dependency.name,
            version: dependency.version,
            license: dependency
                .declared_license
                .unwrap_or_else(|| "記載なし".into()),
            sources,
        };
        if matches!(
            (dependency.ecosystem.as_str(), entry.name.as_str()),
            (
                "npm",
                "react" | "react-dom" | "@tauri-apps/api" | "react-markdown" | "sonner"
            ) | ("cargo", "tauri" | "rusqlite" | "symphonia" | "flacenc")
        ) {
            technologies.push(entry.clone());
        }
        dependencies.push(entry);
    }
    let catalog = catalog(&paths.runtime, target)?;
    let (engines, models) = catalog_credits(&catalog)?;
    technologies.extend(engines);
    technologies.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    dependencies.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    let mut notices = vec![Notice {
        title: "依存ライブラリの通知文".into(),
        text: dependency_notices,
    }];
    for runtime in &paths.runtime {
        collect_notices(runtime, runtime, 0, false, &mut notices)?;
        for archive in ["whisper-runtime.tar", "asr-runtime.tar"] {
            let path = runtime.join(archive);
            if path.is_file() {
                tar_notices(&path, &mut notices)?;
            }
        }
    }
    if paths.extra_notices.is_dir() {
        collect_notices(
            &paths.extra_notices,
            &paths.extra_notices,
            0,
            true,
            &mut notices,
        )?;
    }
    let mut seen = BTreeSet::new();
    notices.retain(|notice| seen.insert(format!("{:x}", Sha256::digest(notice.text.as_bytes()))));
    if notices
        .iter()
        .map(|notice| notice.text.len())
        .sum::<usize>()
        > MAX_TOTAL
    {
        return Err("クレジット資料の合計容量が上限を超えています".into());
    }
    Ok(Credits {
        technologies,
        models,
        dependencies,
        notices,
    })
}

fn catalog(runtime: &[PathBuf], target: Target) -> Result<Vec<Value>, String> {
    match target {
        Target::Windows => serde_json::from_str(&read_text(
            &runtime[0].join("catalog.json"),
            4 * 1024 * 1024,
        )?)
        .map_err(|_| "モデルカタログの形式が不正です".into()),
        Target::Macos => {
            let mut choices: Vec<Value> =
                serde_json::from_str(include_str!("../../model-catalog/macos.json"))
                    .map_err(|e| e.to_string())?;
            let asr: Vec<Value> =
                serde_json::from_str(include_str!("../../model-catalog/asr-transducer.json"))
                    .map_err(|e| e.to_string())?;
            choices.extend(asr);
            #[cfg(target_os = "macos")]
            for (name, recipe) in
                crate::model_bundled_runtime::Bundle::open(&runtime[0])?.recipes()?
            {
                choices.push(serde_json::json!({"label":format!("Whisper {name}（多言語）"),"recipe":recipe}));
            }
            Ok(choices)
        }
    }
}
fn catalog_credits(choices: &[Value]) -> Result<(Vec<Credit>, Vec<Credit>), String> {
    let mut engines = BTreeMap::new();
    let mut models = BTreeMap::new();
    for choice in choices {
        let manifest = &choice["recipe"]["manifest"];
        let text = |field: &str| {
            manifest[field]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "モデルのクレジット情報が不正です".to_string())
        };
        let engine = text("engine")?;
        let engine_version = text("engineVersion")?;
        let license = text("licenseRef")?;
        let sources = choice["recipe"]["sources"]
            .as_array()
            .ok_or("モデルの出典情報が不正です")?;
        let files = manifest["files"]
            .as_array()
            .ok_or("モデルの構成情報が不正です")?;
        let urls = |roles: &[&str]| {
            let paths: BTreeSet<&str> = files
                .iter()
                .filter(|file| {
                    file["role"]
                        .as_str()
                        .is_some_and(|role| roles.contains(&role))
                })
                .filter_map(|file| file["relativePath"].as_str())
                .collect();
            sources
                .iter()
                .filter(|source| {
                    source["relativePath"]
                        .as_str()
                        .is_some_and(|path| paths.contains(path))
                })
                .filter_map(|source| source["url"].as_str())
                .filter(|url| url.starts_with("https://"))
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        };
        let key = (engine.clone(), engine_version.clone());
        let entry = engines.entry(key).or_insert_with(|| Credit {
            name: engine,
            version: engine_version,
            license: license.clone(),
            sources: vec![],
        });
        for url in urls(&["executable", "notice"]) {
            if !entry.sources.contains(&url) {
                entry.sources.push(url);
            }
        }
        let name = text("modelName")?;
        let version = text("modelVersion")?;
        let quantization = manifest["quantization"].as_str();
        let name = quantization.map_or_else(|| name.clone(), |q| format!("{name} · {q}"));
        models.insert(
            (name.clone(), version.clone()),
            Credit {
                name,
                version,
                license,
                sources: urls(&["asr", "llm", "segmentation", "speaker_embedding", "vad"]),
            },
        );
    }
    Ok((
        engines.into_values().collect(),
        models.into_values().collect(),
    ))
}

fn collect_notices(
    root: &Path,
    path: &Path,
    depth: usize,
    inside_notices: bool,
    output: &mut Vec<Notice>,
) -> Result<(), String> {
    if depth > 4 {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "同梱ランタイムのクレジット資料を読めません")?;
    if metadata.file_type().is_symlink() {
        return Err("クレジット資料へのリンクは読み取れません".into());
    }
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let inside_notices = inside_notices || name == "notices" || name == "licenses";
    if metadata.is_dir() {
        let mut children = std::fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        children.sort();
        for child in children {
            collect_notices(root, &child, depth + 1, inside_notices, output)?;
        }
    } else if metadata.is_file()
        && (inside_notices
            || name.contains("license")
            || name.contains("copyright")
            || name.ends_with("-mit.txt")
            || name.contains("-apache-"))
        && !name.ends_with(".json")
    {
        let prefix = root.file_name().unwrap_or_default().to_string_lossy();
        output.push(Notice {
            title: format!(
                "{prefix}/{}",
                path.strip_prefix(root).unwrap_or(path).to_string_lossy()
            ),
            text: read_text(path, 1024 * 1024)?,
        });
    }
    if output.iter().map(|notice| notice.text.len()).sum::<usize>() > MAX_TOTAL {
        return Err("クレジット資料の合計容量が上限を超えています".into());
    }
    Ok(())
}
// Read only text members; never unpack or execute the bundled runtimes.
fn tar_notices(path: &Path, output: &mut Vec<Notice>) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || metadata.len() > 256 * 1024 * 1024 {
        return Err("同梱通知文アーカイブの形式・容量が不正です".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut position = 0u64;
    while position + 512 <= metadata.len() {
        let mut header = [0u8; 512];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let string = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .trim()
                .to_string()
        };
        let name = string(&header[..100]);
        let prefix = string(&header[345..500]);
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let size = u64::from_str_radix(&string(&header[124..136]), 8)
            .map_err(|_| "同梱通知文アーカイブの容量が不正です")?;
        let next = position
            .checked_add(512)
            .and_then(|n| n.checked_add(size.div_ceil(512) * 512))
            .filter(|end| *end <= metadata.len())
            .ok_or("同梱通知文アーカイブが途中で終わっています")?;
        if (header[156] == b'0' || header[156] == 0) && name.starts_with("notices/") {
            if size > 1024 * 1024 {
                return Err("同梱通知文が容量上限を超えています".into());
            }
            let mut text = String::new();
            Read::by_ref(&mut file)
                .take(size)
                .read_to_string(&mut text)
                .map_err(|_| "同梱通知文をUTF-8で読めません")?;
            output.push(Notice {
                title: format!(
                    "{}/{name}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                text,
            });
            if output.iter().map(|notice| notice.text.len()).sum::<usize>() > MAX_TOTAL {
                return Err("クレジット資料の合計容量が上限を超えています".into());
            }
        }
        file.seek(SeekFrom::Start(next))
            .map_err(|e| e.to_string())?;
        position = next;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires MON_PACKAGED_RESOURCES; reads packaged credits without opening app storage"]
    fn packaged_macos_credits_read_dependencies_models_and_notices() {
        let resources = PathBuf::from(std::env::var_os("MON_PACKAGED_RESOURCES").unwrap());
        let result = load(paths(&resources, None, Target::Macos), Target::Macos).unwrap();
        assert!(result
            .technologies
            .iter()
            .any(|entry| entry.name == "react"));
        assert!(result
            .models
            .iter()
            .any(|entry| entry.name.contains("Whisper")));
        assert!(result
            .models
            .iter()
            .any(|entry| entry.name.contains("ReazonSpeech")));
        assert!(!result.dependencies.is_empty());
        assert!(!result.notices.is_empty());
        assert!(result.notices.iter().all(|notice| !notice.text.is_empty()));
    }

    #[test]
    fn credits_are_main_only_and_development_roots_are_os_specific() {
        assert!(require_main("main").is_ok());
        for role in ["recorder", "recorder-info", "recording-confirmation"] {
            assert!(require_main(role).is_err());
        }
        let root = tempfile::tempdir().unwrap();
        let windows = paths(root.path(), Some(root.path()), Target::Windows);
        assert_eq!(
            windows.notices,
            root.path().join(".cache/windows-third-party-notices")
        );
        assert_eq!(
            windows.runtime[0],
            root.path().join(".cache/windows-runtime/package")
        );
        let macos = paths(root.path(), Some(root.path()), Target::Macos);
        assert_eq!(
            macos.notices,
            root.path().join(".cache/third-party-notices")
        );
        assert_eq!(
            macos.runtime[0],
            root.path().join(".cache/whisper-metal-runtime/package")
        );
        std::fs::write(root.path().join("third-party-inventory.json"), "{}").unwrap();
        let packaged = paths(root.path(), Some(root.path()), Target::Macos);
        assert_eq!(
            packaged.runtime,
            vec![
                root.path().join("asr-runtime"),
                root.path().join("inference-runtime")
            ]
        );
    }
    #[test]
    fn catalog_lists_exact_versions_licenses_and_separate_model_sources() {
        let choices: Vec<Value> =
            serde_json::from_str(include_str!("../../model-catalog/macos.json")).unwrap();
        let (engines, models) = catalog_credits(&choices).unwrap();
        assert!(!engines.is_empty());
        assert!(!models.is_empty());
        assert!(models.iter().all(|model| !model.version.is_empty()
            && !model.license.is_empty()
            && !model.sources.is_empty()));
        assert!(models.iter().any(|model| model.name.contains("Qwen")));
        assert!(catalog_credits(&[serde_json::json!({})]).is_err());
    }
    #[test]
    fn inventory_and_notice_integrity_are_required() {
        let root = tempfile::tempdir().unwrap();
        let paths = || Paths {
            notices: root.path().to_owned(),
            runtime: vec![root.path().to_owned()],
            extra_notices: root.path().join("absent"),
        };
        assert!(load(paths(), Target::Windows).is_err());
        std::fs::write(root.path().join("THIRD_PARTY_NOTICES.txt"), "actual text").unwrap();
        std::fs::write(root.path().join("third-party-inventory.json"), r#"{"schemaVersion":1,"target":"x86_64-pc-windows-msvc","noticesSha256":"wrong","packages":[]}"#).unwrap();
        assert!(load(paths(), Target::Windows)
            .unwrap_err()
            .contains("一致しません"));
    }
    #[test]
    fn packaged_resources_preserve_notice_text_and_dependency_provenance() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("windows-runtime");
        std::fs::create_dir_all(runtime.join("notices")).unwrap();
        let text = "Original attribution\r\nCopyright holder\n";
        std::fs::write(root.path().join("THIRD_PARTY_NOTICES.txt"), text).unwrap();
        std::fs::write(root.path().join("third-party-inventory.json"), serde_json::to_vec(&serde_json::json!({
            "schemaVersion":1,"target":Target::Windows.triple(),"noticesSha256":format!("{:x}",Sha256::digest(text.as_bytes())),
            "packages":[{"ecosystem":"npm","name":"react","version":"19.0.0","declaredLicense":"MIT","repository":null,"sourceArchive":null}]
        })).unwrap()).unwrap();
        std::fs::write(
            runtime.join("catalog.json"),
            include_str!("../../model-catalog/macos.json"),
        )
        .unwrap();
        std::fs::write(runtime.join("notices/engine-MIT.txt"), "Engine copyright\n").unwrap();
        let result = load(paths(root.path(), None, Target::Windows), Target::Windows).unwrap();
        assert!(result
            .technologies
            .iter()
            .any(|entry| entry.name == "react" && entry.version == "19.0.0"));
        assert_eq!(result.dependencies[0].sources, vec!["npm:react@19.0.0"]);
        assert_eq!(result.notices[0].text, text);
        assert!(result
            .notices
            .iter()
            .any(|notice| notice.text == "Engine copyright\n"));
        assert!(result
            .models
            .iter()
            .any(|entry| entry.name.contains("Qwen")));
    }
}
