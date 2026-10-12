//! Local file import. Paths never become permanent analysis dependencies.
mod mp3;
use crate::audio::VerifiedAudio;
use crate::local_engine::{Fingerprint, ManagedChild, StableFile};
use crate::storage::{imports::ImportSnapshot, Storage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ImportOptions {
    pub meeting_vocabulary: Vec<crate::storage::VocabularyEntry>,
    pub expected_global_version: Option<i64>,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            meeting_vocabulary: vec![],
            expected_global_version: None,
        }
    }
}
impl ImportOptions {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.expected_global_version.is_some_and(|v| v < 0) {
            return Err("取り込みの用語版が不正です".into());
        }
        crate::storage::vocabulary::validate(&self.meeting_vocabulary)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AudioInfo {
    pub schema_version: u32,
    pub detected_format: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: u64,
    pub duration_us: i64,
}
impl AudioInfo {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || !matches!(
                (self.detected_format.as_str(), self.codec.as_str()),
                ("wav", "pcm") | ("mp3", "mp3") | ("m4a", "aac" | "alac") | ("flac", "flac")
            )
            || !(8000..=384000).contains(&self.sample_rate)
            || !(1..=8).contains(&self.channels)
            || self.frames == 0
            || self.frames > u64::from(self.sample_rate) * 14400
            || self.duration_us != (self.frames * 1_000_000 / u64::from(self.sample_rate)) as i64
        {
            return Err("音声の形式・長さが対応範囲外です".into());
        }
        Ok(())
    }
    pub(crate) fn required_bytes(&self, original: u64) -> Result<u64, String> {
        self.frames
            .checked_mul(u64::from(self.channels))
            .and_then(|n| n.checked_mul(8))
            .and_then(|n| n.checked_add(original))
            .and_then(|n| n.checked_add(128 * 1024 * 1024))
            .ok_or("音声保存に必要な容量が範囲外です".into())
    }
}

pub(crate) struct SelectedAudio {
    pub proof: Fingerprint,
    pub file_name: String,
    pub info: AudioInfo,
    file: File,
}

// Production callers obtain the helper from the signed bundled probe, not IPC.
pub(crate) fn inspect(
    input: &Path,
    helper: &Fingerprint,
    scratch: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<SelectedAudio, String> {
    let proof = Fingerprint::capture(input, cancelled)?;
    let watch = StableFile::watch(&proof)?;
    let file = File::open(&proof.path).map_err(|e| e.to_string())?;
    let file_name = input
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("音声ファイル名が不正です")?
        .to_owned();
    if file_name.chars().count() > 255 || file_name.chars().any(char::is_control) {
        return Err("音声ファイル名が長すぎるか制御文字を含んでいます".into());
    }
    let info = decode(helper, &proof.path, None, scratch, cancelled)?;
    info.validate()?;
    // Native FLAC input additionally uses the strict CRC/PCM-MD5 decoder.
    if info.detected_format == "flac" {
        let audio = VerifiedAudio::open_cancellable(
            file.try_clone().map_err(|e| e.to_string())?,
            proof.size,
            &proof.sha256,
            cancelled,
        )?;
        if audio.format.frames != info.frames
            || audio.format.sample_rate != info.sample_rate
            || audio.format.channels != info.channels
        {
            return Err("FLACの検証結果と復号結果が一致しません".into());
        }
    }
    watch.verify()?;
    proof.verify(cancelled)?;
    Ok(SelectedAudio {
        proof,
        file_name,
        info,
        file,
    })
}

pub(crate) fn decode(
    helper: &Fingerprint,
    input: &Path,
    output: Option<&Path>,
    scratch: &Path,
    cancelled: &impl Fn() -> bool,
) -> Result<AudioInfo, String> {
    let mut header = [0; 3];
    File::open(input)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|e| e.to_string())?;
    if &header == b"ID3" || (header[0] == 0xff && header[1] & 0xe0 == 0xe0) {
        return mp3::decode(input, output, cancelled);
    }
    helper.verify(cancelled)?;
    let watch = StableFile::watch(helper)?;
    let report = tempfile::Builder::new()
        .prefix(".mon-import-report-")
        .tempfile_in(scratch)
        .map_err(|e| e.to_string())?;
    let mut child = ManagedChild::direct(
        Command::new(&helper.path)
            .arg("--import-audio")
            .arg(input)
            .arg(output.unwrap_or(Path::new("-")))
            .arg(std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                report.as_file().try_clone().map_err(|e| e.to_string())?,
            ))
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let started = Instant::now();
    let status = loop {
        check_cancelled(cancelled)?;
        watch.verify()?;
        if report
            .as_file()
            .metadata()
            .map_err(|e| e.to_string())?
            .len()
            > 16384
        {
            return Err("音声検証の応答が大きすぎます".into());
        }
        if let Some(status) = child.child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(3600) {
            return Err("音声の復号が時間上限を超えました".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    check_cancelled(cancelled)?;
    helper.verify(cancelled)?;
    let length = report
        .as_file()
        .metadata()
        .map_err(|e| e.to_string())?
        .len();
    if length == 0 || length > 16384 {
        return Err("音声検証の結果がありません".into());
    }
    let mut file = report.as_file().try_clone().map_err(|e| e.to_string())?;
    file.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(16385)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let error: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        return Err(error
            .get("errorMessage")
            .and_then(|e| e.as_str())
            .unwrap_or("音声の復号に失敗しました")
            .chars()
            .take(2048)
            .collect());
    }
    let info: AudioInfo = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    info.validate()?;
    Ok(info)
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("音声取り込みを中断しました".into())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn available_bytes(root: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(root.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // Valid NUL-terminated path and an output struct owned by this call.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let stats = unsafe { stats.assume_init() };
    (stats.f_bavail as u64)
        .checked_mul(stats.f_frsize as u64)
        .ok_or("保存先容量が範囲外です".into())
}
#[cfg(not(unix))]
pub(crate) fn available_bytes(_: &Path) -> Result<u64, String> {
    Err("このOSの音声取り込みはまだ接続されていません".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ImportManifest {
    pub schema_version: u32,
    pub meeting_id: String,
    pub import_id: String,
    pub original_file_name: String,
    pub info: AudioInfo,
    pub original: FileEvidence,
    pub audio: FileEvidence,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct FileEvidence {
    pub size: u64,
    pub sha256: String,
}

#[cfg(test)]
pub(crate) fn import(
    storage: &Storage,
    selected: SelectedAudio,
    helper: &Fingerprint,
    title: &str,
    operation: &str,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(&str, u64),
) -> Result<ImportSnapshot, String> {
    import_configured(
        storage,
        selected,
        helper,
        title,
        operation,
        &ImportOptions::default(),
        cancelled,
        progress,
    )
}
pub(crate) fn import_configured(
    storage: &Storage,
    mut selected: SelectedAudio,
    helper: &Fingerprint,
    title: &str,
    operation: &str,
    options: &ImportOptions,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(&str, u64),
) -> Result<ImportSnapshot, String> {
    selected.info.validate()?;
    check_cancelled(cancelled)?;
    if available_bytes(storage.import_scratch()?)?
        < selected.info.required_bytes(selected.proof.size)?
    {
        return Err(
            "保存先の空き容量が不足しています。原本と変換用音声の容量を確保してください".into(),
        );
    }
    let (snapshot, created) = storage.begin_audio_import_configured(
        title,
        operation,
        &selected.file_name,
        &selected.proof,
        &selected.info,
        options,
    )?;
    if snapshot.state == "ready" {
        return Ok(snapshot);
    }
    if !created || snapshot.state != "copying" {
        return Err("この取り込みは既に進行中または終了しています".into());
    }
    let base = match storage.audio_import_directory(&snapshot) {
        Ok(base) => base,
        Err(error) => {
            storage.fail_audio_import(&snapshot.id, "failed", &error)?;
            return Err(error);
        }
    };
    let original = base.join(format!("original.{}", selected.info.detected_format));
    let result = (|| -> Result<ImportSnapshot, String> {
        let watch = StableFile::watch(&selected.proof)?;
        selected.file.rewind().map_err(|e| e.to_string())?;
        let mut output = File::options()
            .write(true)
            .create_new(true)
            .open(&original)
            .map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        let mut size = 0;
        loop {
            check_cancelled(cancelled)?;
            let count = selected.file.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            size += count as u64;
            if size > selected.proof.size {
                return Err("コピー中に元音声が変更されました".into());
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            hash.update(&buffer[..count]);
            progress("copying", size);
        }
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        watch.verify()?;
        if size != selected.proof.size || format!("{:x}", hash.finalize()) != selected.proof.sha256
        {
            return Err("管理コピーのハッシュが元音声と一致しません".into());
        }
        storage.audio_import_converting(&snapshot.id)?;
        check_cancelled(cancelled)?;
        if available_bytes(&base)? < selected.info.required_bytes(0)? {
            return Err("変換用音声を保存する容量が不足しています".into());
        }
        let caf = base.join("decoded.caf");
        progress("decoding", 0);
        let decoded = decode(helper, &original, Some(&caf), &base, cancelled)?;
        if decoded != selected.info {
            return Err("管理コピーの復号結果が確認した音声と一致しません".into());
        }
        let proof = Fingerprint::capture(&caf, cancelled)?;
        let mut audio = VerifiedAudio::open_cancellable(
            File::open(&caf).map_err(|e| e.to_string())?,
            proof.size,
            &proof.sha256,
            cancelled,
        )?;
        if audio.format.frames != decoded.frames
            || audio.format.sample_rate != decoded.sample_rate
            || audio.format.channels != decoded.channels
        {
            return Err("解析用PCMの形式・長さが復号結果と一致しません".into());
        }
        let saved = crate::audio::flac::write_segment(
            &mut audio,
            0,
            decoded.frames,
            &base.join("audio.flac"),
            cancelled,
            &|n| progress("converting", n),
        )
        .map_err(|e| e.message)?;
        drop(audio);
        std::fs::remove_file(&caf).map_err(|e| e.to_string())?;
        check_cancelled(cancelled)?;
        let manifest = ImportManifest {
            schema_version: 1,
            meeting_id: snapshot.meeting_id.clone(),
            import_id: snapshot.id.clone(),
            original_file_name: selected.file_name.clone(),
            info: decoded.clone(),
            original: FileEvidence {
                size: selected.proof.size,
                sha256: selected.proof.sha256.clone(),
            },
            audio: FileEvidence {
                size: saved.size,
                sha256: saved.sha256,
            },
        };
        let mut pending = tempfile::Builder::new()
            .prefix(".mon-import-manifest-")
            .suffix(".part")
            .tempfile_in(&base)
            .map_err(|e| e.to_string())?;
        pending
            .write_all(&serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        pending.as_file().sync_all().map_err(|e| e.to_string())?;
        pending
            .persist_noclobber(base.join("manifest.json"))
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        File::open(&base)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        progress("saving", decoded.frames);
        storage.finalize_audio_import_cancellable(&snapshot.id, cancelled)
    })();
    match result {
        Ok(saved) => Ok(saved),
        Err(error) => {
            // A complete manifest is a recovery boundary, not disposable temp.
            if base.join("manifest.json").exists() && !cancelled() {
                storage.audio_import_pending_finalization(&snapshot.id, &error)?;
                Err(format!(
                    "取り込み音声を保持しました。次回起動時に保存を再検証します: {error}"
                ))
            } else {
                storage.fail_audio_import(
                    &snapshot.id,
                    if cancelled() { "cancelled" } else { "failed" },
                    &error,
                )?;
                storage.cleanup_audio_import(&snapshot.id)?;
                Err(error)
            }
        }
    }
}
