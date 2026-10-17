use super::*;
use crate::local_engine::Fingerprint;
use crate::model_placement::fs::Directory;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("モデル互換性確認を取り消しました".into())
    } else {
        Ok(())
    }
}
fn package_path(models_root: &Path, package: &str) -> Result<PathBuf, String> {
    if !models_root.is_absolute() || !canonical_id(package) {
        return Err("モデル管理領域・IDが不正です".into());
    }
    let root_meta = models_root.symlink_metadata().map_err(|e| e.to_string())?;
    if !root_meta.is_dir() {
        return Err("モデル管理領域が通常フォルダーではありません".into());
    }
    let root = models_root.canonicalize().map_err(|e| e.to_string())?;
    let path = root.join(package);
    let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    if !meta.is_dir()
        || meta.permissions().mode() & 0o077 != 0
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("モデル配置先の実体・権限が不正です".into());
    }
    Ok(path)
}
pub(super) fn verify_package(
    models_root: &Path,
    record: &PlacementRecord,
    cancelled: &impl Fn() -> bool,
) -> Result<PathBuf, String> {
    check_cancelled(cancelled)?;
    let path = package_path(models_root, &record.package_id)?;
    let report = crate::model_inspection::inspect(&path, cancelled)?;
    if report.review_hash != record.review_hash {
        return Err("配置済みモデルの全構成・容量・SHAが変わっています".into());
    }
    Ok(path)
}
fn fingerprints(record: &PlacementRecord, directory: &Path, role: Role) -> Vec<Fingerprint> {
    record
        .files
        .iter()
        .filter(|file| file.role == role)
        .map(|file| Fingerprint {
            path: directory.join(&file.relative_path),
            size: file.actual_size,
            sha256: file.actual_hash.clone(),
        })
        .collect()
}
fn one(record: &PlacementRecord, directory: &Path, role: Role) -> Result<Fingerprint, String> {
    let files = fingerprints(record, directory, role);
    if files.len() != 1 {
        return Err("互換性確認に必要なモデル構成がありません".into());
    }
    Ok(files.into_iter().next().unwrap())
}
fn artifact(
    directory: &Path,
    name: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Artifact, String> {
    let path = directory.join(name);
    let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.len() > 16 * 1024 * 1024
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("互換性の証拠が通常ファイルではないか上限を超えています".into());
    }
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        check_cancelled(cancelled)?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .ok_or("互換性の証拠容量が範囲外です")?;
        if count > 16 * 1024 * 1024 {
            return Err("互換性の証拠が増大しました".into());
        }
        hash.update(&buffer[..n]);
    }
    use std::os::unix::fs::MetadataExt;
    let same = |current: &std::fs::Metadata| {
        current.is_file()
            && current.dev() == before.dev()
            && current.ino() == before.ino()
            && current.len() == before.len()
            && current.modified().ok() == before.modified().ok()
            && current.ctime() == before.ctime()
            && current.ctime_nsec() == before.ctime_nsec()
    };
    if count != before.len()
        || !same(&meta)
        || !same(&file.metadata().map_err(|e| e.to_string())?)
        || !same(&path.symlink_metadata().map_err(|e| e.to_string())?)
    {
        return Err("照合中に互換性の証拠が変更されました".into());
    }
    Ok(Artifact {
        name: name.into(),
        size: count,
        sha256: format!("{:x}", hash.finalize()),
    })
}
fn artifacts(
    directory: &Path,
    engine: Engine,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<Artifact>, String> {
    artifact_names(engine)
        .iter()
        .map(|name| artifact(directory, name, cancelled))
        .collect()
}
fn wave(directory: &Directory) -> Result<(), String> {
    let (mut file, parent) = directory.create("input.wav")?;
    let frames = 160_000_u32;
    let mut bytes = Vec::with_capacity(44 + frames as usize * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + frames * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&16000_u32.to_le_bytes());
    bytes.extend_from_slice(&32000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(frames * 2).to_le_bytes());
    bytes.resize(44 + frames as usize * 2, 0);
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    parent.sync()
}
fn run(
    record: &PlacementRecord,
    package: &Path,
    directory: &Directory,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    check_cancelled(cancelled)?;
    let executable = one(record, package, Role::Executable)?;
    match record.manifest.engine {
        Engine::Whisper => {
            wave(directory)?;
            let config = crate::transcription::whisper::WhisperConfig {
                executable,
                model: one(record, package, Role::Asr)?,
                vad_model: fingerprints(record, package, Role::Vad).into_iter().next(),
                vad_options: record.files.iter().any(|f| f.role == Role::Vad).then(Default::default),
                threads: 1,
                use_gpu: false,
                prompt: String::new(),
            };
            config.verify(cancelled)?;
            crate::transcription::whisper::run_unit(
                &config,
                directory.path(),
                10_000_000,
                timeout,
                cancelled,
            )?
            .ok_or("モデル互換性確認を取り消しました")?;
        }
        Engine::Sherpa => {
            wave(directory)?;
            let config = crate::diarization::sherpa::SherpaConfig {
                executable,
                segmentation_model: one(record, package, Role::Segmentation)?,
                embedding_model: one(record, package, Role::SpeakerEmbedding)?,
                runtime_libraries: fingerprints(record, package, Role::RuntimeLibrary),
                threads: 1,
                cluster_threshold_milli: 500,
                window_shift_milli: 500,
                min_duration_on_ms: 300,
                min_duration_off_ms: 500,
                speech_sensitivity_milli: 0,
            };
            crate::diarization::sherpa::run_track(
                &config,
                None,
                directory.path(),
                10_000_000,
                timeout,
                cancelled,
            )?
            .ok_or("モデル互換性確認を取り消しました")?;
        }
        Engine::Llama => {
            let config = crate::generation::llama::LlamaConfig {
                executable,
                model: one(record, package, Role::Llm)?,
                threads: 1,
                context_tokens: 2048,
                prediction_tokens: 64,
                gpu_layers: 0,
                seed: 42,
            };
            let schema = serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean","const":true}},"required":["ok"],"additionalProperties":false});
            let result=crate::generation::llama::run_unit(&config,directory.path(),"Return the exact JSON object required by the schema. This is a local compatibility test.","Return {\"ok\":true}.",&schema,timeout,cancelled)?.ok_or("モデル互換性確認を取り消しました")?;
            if result != serde_json::json!({"ok":true}) {
                return Err("議事録エンジンが固定schemaの確認結果を返しませんでした".into());
            }
        }
    }
    check_cancelled(cancelled)
}
pub(super) fn verify_success(
    storage: &Storage,
    models_root: &Path,
    operation: &str,
    package: &str,
    expected_review: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<(PlacementRecord, PathBuf), String> {
    if !canonical_id(operation) {
        return Err("互換性確認の操作IDが不正です".into());
    }
    check_cancelled(cancelled)?;
    let record = storage.model_package_for_check(package, expected_review)?;
    let (summary, report) = storage
        .model_check_record(operation)?
        .ok_or("互換性の成功記録がありません")?;
    if summary.package_id != record.package_id
        || summary.review_hash != record.review_hash
        || summary.state != CheckState::Succeeded
    {
        return Err("最新の成功した互換性確認と対象が一致しません".into());
    }
    let report = report.ok_or("互換性の成功証拠がありません")?;
    report.validate(&record)?;
    let package_path = verify_package(models_root, &record, cancelled)?;
    let check_root = models_root
        .parent()
        .ok_or("モデル管理領域の親がありません")?
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join("model-checks");
    if !check_root
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("互換性の記録領域が通常フォルダーではありません".into());
    }
    let directory = check_root.join(operation);
    if directory.canonicalize().map_err(|e| e.to_string())? != directory
        || artifacts(&directory, record.manifest.engine, cancelled)? != report.artifacts
    {
        return Err("保存済みの互換性試行の証拠が欠損・変更されています".into());
    }
    check_cancelled(cancelled)?;
    Ok((record, package_path))
}
pub(super) fn check(
    storage: &Storage,
    models_root: &Path,
    operation: &str,
    package: &str,
    expected_review: &str,
    timeout: Duration,
    cancelled: &impl Fn() -> bool,
) -> Result<CheckSummary, String> {
    if !canonical_id(operation) || timeout.is_zero() || timeout > Duration::from_secs(120) {
        return Err("互換性確認の操作ID・制限時間が不正です".into());
    }
    check_cancelled(cancelled)?;
    let record = storage.model_package_for_check(package, expected_review)?;
    if let Some((summary, _)) = storage.model_check_record(operation)? {
        if summary.package_id != record.package_id
            || summary.review_hash != record.review_hash
            || summary.state != CheckState::Succeeded
        {
            return Err(
                "この操作は再利用できません。新しい操作として互換性を確認してください".into(),
            );
        }
        verify_success(
            storage,
            models_root,
            operation,
            package,
            expected_review,
            cancelled,
        )?;
        return Ok(summary);
    }
    storage.begin_model_check(operation, &record)?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<ProbeReport, String> {
            let started = Instant::now();
            let package = verify_package(models_root, &record, cancelled)?;
            let root = Directory::root(
                &package
                    .parent()
                    .and_then(|p| p.parent())
                    .ok_or("モデル管理領域の親がありません")?
                    .join("model-checks"),
            )?;
            if root.available()? < 64 * 1024 * 1024 {
                return Err("互換性の試行・ログ領域の空き容量が不足しています".into());
            }
            let directory = root.child(operation, true)?;
            run(&record, &package, &directory, timeout, cancelled)?;
            root.verify()?;
            directory.verify()?;
            directory.sync()?;
            verify_package(models_root, &record, cancelled)?;
            let report = ProbeReport {
                schema_version: 1,
                operation_id: operation.into(),
                package_id: record.package_id.clone(),
                review_hash: record.review_hash.clone(),
                engine: record.manifest.engine,
                declared_engine_version: record.manifest.engine_version.clone(),
                adapter_revision: 1,
                settings: settings(record.manifest.engine),
                artifacts: artifacts(directory.path(), record.manifest.engine, cancelled)?,
                elapsed_ms: u64::try_from(started.elapsed().as_millis())
                    .map_err(|e| e.to_string())?,
            };
            report.validate(&record)?;
            check_cancelled(cancelled)?;
            root.verify()?;
            directory.verify()?;
            Ok(report)
        },
    ))
    .unwrap_or_else(|_| Err("モデル互換性の確認処理が予期せず停止しました".into()));
    match result {
        Ok(report) if !cancelled() => storage.finish_model_check(
            operation,
            &record,
            Some(&report),
            CheckState::Succeeded,
            None,
        ),
        Ok(_) => storage.finish_model_check(
            operation,
            &record,
            None,
            CheckState::Cancelled,
            Some("モデル互換性確認を取り消しました"),
        ),
        Err(error) => storage.finish_model_check(
            operation,
            &record,
            None,
            if cancelled() {
                CheckState::Cancelled
            } else {
                CheckState::Failed
            },
            Some(&error),
        ),
    }
}
