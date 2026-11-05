//! A single native-owned resource ID, not general local URL support. Index reads
//! are bounded/read-only; acquisition binds the whole recipe again, holds the
//! no-link resource handles, and checks every byte without executing anything.
use crate::model_download_recipe::RecipeExt;
use crate::model_download_recipe::{ArchiveMember, DownloadSource, Recipe, BUNDLED_WHISPER};
use crate::model_inspection::{Engine, PackageFile, PackageManifest, Role, Source, Target};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_PACKAGE: u64 = 16 * 1024 * 1024;
const MEMBERS: [&str; 3] = [
    "bin/whisper-cli",
    "notices/whisper-MIT.txt",
    "build-info.json",
];
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Index {
    schema_version: u32,
    engine: String,
    engine_version: String,
    source_commit: String,
    source_url: String,
    source_bytes: u64,
    source_sha256: String,
    target: String,
    #[serde(rename = "minimumMacOS")]
    minimum_macos: String,
    build_options: Vec<String>,
    cpu_only: bool,
    publisher_authenticity: String,
    license_review: String,
    distribution_ready: bool,
    archive: Archive,
    files: Vec<Member>,
    runtime_signing: String,
    signing_team_id: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    file: String,
    bytes: u64,
    sha256: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Member {
    relative_path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Network {
    files: Vec<PackageFile>,
    sources: Vec<DownloadSource>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    name: String,
    bytes: u64,
    sha256: String,
}
pub(crate) struct Bundle {
    index: Index,
    evidence: Source,
    archive: Option<Source>,
}
fn check(cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("同梱モデル素材の準備を中断しました".into())
    } else {
        Ok(())
    }
}
impl Bundle {
    pub(crate) fn open(root: &Path) -> Result<Self, String> {
        // Do NOT canonicalize a caller-supplied link into a trusted root.
        let mut evidence = Source::open(root, "runtime-index.json")
            .map_err(|_| "文字起こし用の同梱情報を読めません。アプリを再確認してください")?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut evidence.file)
            .take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|_| "同梱情報を読み取れません")?;
        evidence.verify()?;
        if bytes.is_empty() || bytes.len() > 65_536 {
            return Err("同梱情報の容量が不正です".into());
        }
        let shape: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "同梱情報が不正です")?;
        if shape["schemaVersion"] != 2
            || !shape
                .as_object()
                .is_some_and(|o| o.contains_key("signingTeamId"))
        {
            return Err("非対応の同梱ランタイム形式です".into());
        }
        let index: Index =
            serde_json::from_slice(&bytes).map_err(|_| "同梱情報の形式が不正です")?;
        let mut info = serde_json::to_value(&index).map_err(|e| e.to_string())?;
        for key in ["archive", "files", "runtimeSigning", "signingTeamId"] {
            info.as_object_mut().unwrap().remove(key);
        }
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../model-catalog/whisper-runtime-build.json"
        ))
        .map_err(|_| "固定された同梱定義が不正です")?;
        // This is packaging metadata, not OS certificate verification. A
        // release builder verifies the inner CLI before signing the app. Keep
        // publisher authenticity/readiness unverified and bind every byte as
        // before; no signing label bypasses hashes, provenance or inspection.
        let signing_valid = match (index.runtime_signing.as_str(), &index.signing_team_id) {
            ("ad_hoc_only", None) => true,
            ("developer_id_signed", Some(team)) => {
                team.len() == 10
                    && team
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            }
            _ => false,
        };
        if info != expected
            || !signing_valid
            || index.archive.file != "whisper-runtime.tar"
            || index.archive.bytes == 0
            || index.archive.bytes > MAX_PACKAGE
            || !crate::model_download_ipc::hash(&index.archive.sha256)
            || index.files.len() != MEMBERS.len()
        {
            return Err("同梱情報が固定されたCPU・Metal実行ファイルの定義と一致しません".into());
        }
        for (i, member) in index.files.iter().enumerate() {
            let limit = if i == 0 { MAX_PACKAGE } else { 65_536 };
            if member.relative_path != MEMBERS[i]
                || member.bytes == 0
                || member.bytes > limit
                || !crate::model_download_ipc::hash(&member.sha256)
            {
                return Err("同梱ファイルの構成・容量・SHAが不正です".into());
            }
        }
        Ok(Self {
            index,
            evidence,
            archive: None,
        })
    }
    #[cfg(test)]
    pub(crate) fn recipe(&self) -> Result<Recipe, String> {
        self.recipe_for("medium")
    }
    pub(crate) fn recipes(&self) -> Result<Vec<(String, Recipe)>, String> {
        Self::variants()?
            .into_iter()
            .map(|v| {
                let recipe = self.recipe_for(&v.name)?;
                Ok((v.name, recipe))
            })
            .collect()
    }
    fn variants() -> Result<Vec<Variant>, String> {
        let variants: Vec<Variant> =
            serde_json::from_str(include_str!("../../model-catalog/whisper-variants.json"))
                .map_err(|_| "文字起こしモデルの候補情報が不正です")?;
        let names: Vec<_> = variants.iter().map(|v| v.name.as_str()).collect();
        if names != ["medium", "large-v3", "large-v3-turbo"]
            || variants.iter().any(|v| {
                v.bytes == 0
                    || v.bytes > 4 * 1024 * 1024 * 1024
                    || !crate::model_download_ipc::hash(&v.sha256)
            })
        {
            return Err("文字起こしモデルの固定候補・容量・SHAが不正です".into());
        }
        Ok(variants)
    }
    fn recipe_for(&self, name: &str) -> Result<Recipe, String> {
        self.evidence.verify()?;
        let variant = Self::variants()?
            .into_iter()
            .find(|v| v.name == name)
            .ok_or("非対応の文字起こしモデルです")?;
        let mut network: Network =
            serde_json::from_str(include_str!("../../model-catalog/whisper-network.json"))
                .map_err(|_| "文字起こしモデルの固定配布情報が不正です")?;
        let path = format!("models/ggml-{name}.bin");
        network.files[0].relative_path = path.clone();
        network.files[0].expected_size = Some(variant.bytes);
        network.files[0].expected_hash = Some(variant.sha256);
        network.sources[0].relative_path = path;
        network.sources[0].url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-{name}.bin");
        let mut files = Vec::new();
        let mut sources = Vec::new();
        for (i, member) in self.index.files.iter().enumerate() {
            files.push(PackageFile {
                relative_path: member.relative_path.clone(),
                role: if i == 0 {
                    Role::Executable
                } else {
                    Role::Notice
                },
                expected_size: Some(member.bytes),
                expected_hash: Some(member.sha256.clone()),
            });
            sources.push(DownloadSource {
                relative_path: member.relative_path.clone(),
                url: BUNDLED_WHISPER.into(),
                archive: Some(ArchiveMember {
                    format: "tar".into(),
                    expected_size: self.index.archive.bytes,
                    expected_hash: self.index.archive.sha256.clone(),
                    member_path: member.relative_path.clone(),
                }),
            });
        }
        let vad: Network =
            serde_json::from_str(include_str!("../../model-catalog/whisper-vad-network.json"))
                .map_err(|_| "VADの固定配布情報が不正です")?;
        network.files.extend(vad.files);
        network.sources.extend(vad.sources);
        files.extend(network.files);
        sources.extend(network.sources);
        let recipe = Recipe { schema_version: 4, manifest: PackageManifest { backend: None,
            schema_version: 1, engine: Engine::Whisper, engine_version: "v1.9.4".into(),
            model_name: format!("Whisper {name}（多言語）"), model_version: "5359861c739e955e79d9a303bcbc70fb988958b1".into(),
            quantization: None, target: Target::MacosAarch64,
            license_ref: "MIT: notices/whisper-MIT.txt, notices/openai-whisper-MIT.txt, notices/silero-vad-MIT.txt; VAD card: notices/silero-vad-model-card.md; model card: notices/whisper-model-card.md; app-built CPU/Metal provenance: build-info.json".into(),
            files,
        }, sources };
        recipe.review()?;
        Ok(recipe)
    }
    pub(crate) fn bind(&self, recipe: &Recipe) -> Result<(), String> {
        if !self.recipes()?.iter().any(|(_, allowed)| recipe == allowed) {
            return Err(
                "同梱モデルの準備内容がこのアプリの固定構成と一致しません。再確認してください"
                    .into(),
            );
        }
        Ok(())
    }
    pub(crate) fn prepare(
        &mut self,
        root: &Path,
        recipe: &Recipe,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        check(cancelled)?;
        self.bind(recipe)?;
        self.archive = Some(Source::open(root, "whisper-runtime.tar")?);
        // Before any staging/intent or network, reject a replaced archive.
        self.copy(None, cancelled)?;
        self.verify()
    }
    fn verify(&self) -> Result<(), String> {
        self.evidence.verify()?;
        if let Some(source) = &self.archive {
            source.verify()?;
        }
        Ok(())
    }
    fn copy(
        &mut self,
        mut output: Option<&mut std::fs::File>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        self.verify()?;
        let source = self
            .archive
            .as_mut()
            .ok_or("同梱実体の事前照合がありません")?;
        source
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let start = std::time::Instant::now();
        let mut digest = Sha256::new();
        let mut count = 0_u64;
        let mut buffer = [0_u8; 65_536];
        loop {
            check(cancelled)?;
            if start.elapsed().as_secs() > 60 {
                return Err("同梱素材の読取時間の上限を超えました".into());
            }
            let n = source
                .file
                .read(&mut buffer)
                .map_err(|_| "同梱素材を読み取れません")?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > self.index.archive.bytes {
                return Err("同梱素材の容量が一致しません".into());
            }
            digest.update(&buffer[..n]);
            if let Some(file) = output.as_mut() {
                file.write_all(&buffer[..n])
                    .map_err(|_| "同梱素材を保存できません")?;
            }
        }
        if count != self.index.archive.bytes
            || format!("{:x}", digest.finalize()) != self.index.archive.sha256
        {
            return Err("同梱素材の容量・SHAが一致しません".into());
        }
        self.verify()
    }
    pub(crate) fn fetch(
        &mut self,
        url: &str,
        size: u64,
        hash: &str,
        output: &mut std::fs::File,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        if url != BUNDLED_WHISPER
            || size != self.index.archive.bytes
            || hash != self.index.archive.sha256
        {
            return Err("同梱素材の参照が確認した構成と一致しません".into());
        }
        self.copy(Some(output), cancelled)
    }
}

#[cfg(test)]
mod tests;
