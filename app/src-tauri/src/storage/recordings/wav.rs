//! The native producer's single PCM file. Only advertised, hashed ranges may
//! be repaired or published; a missing receipt never authorizes a guessed tail.
use super::*;
use crate::storage::publication_files::{Directory, Identity, Stamp};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Tip {
    schema_version: u32,
    owner_id: String,
    file_name: String,
    pub(crate) frames: u64,
    pub(crate) checkpoints: u32,
    #[serde(rename = "lastEntrySHA256")]
    last_entry_sha256: Option<String>,
    capture_origin_nanoseconds: Option<i64>,
    producer_closed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Reservation {
    schema_version: u32,
    owner_id: String,
    file_name: String,
    device: u64,
    inode: u64,
    birth_nanoseconds: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Receipt {
    schema_version: u32,
    owner_id: String,
    sequence: u32,
    frame_start: u64,
    frames: u64,
    capture_origin_nanoseconds: i64,
    #[serde(rename = "pcmSHA256")]
    pcm_sha256: String,
    #[serde(rename = "previousEntrySHA256")]
    previous_entry_sha256: Option<String>,
    #[serde(default)]
    missing_ranges: Vec<[u64; 2]>,
    #[serde(default)]
    coverage_frames: [u64; 2],
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn identity_matches(file: &File, reservation: &Reservation) -> Result<(), String> {
    let meta = crate::native_fs::metadata(file).map_err(|e| e.to_string())?;
    let birth = meta
        .created()
        .map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.dev() != reservation.device
        || meta.ino() != reservation.inode
        || birth != reservation.birth_nanoseconds as u128
    {
        return Err("録音WAVの所有実体が変更されています".into());
    }
    Ok(())
}

/// Incremental read-only telemetry. Each newly committed PCM range is checked
/// once, independently of mutable manifest statistics or the producer's header.
#[derive(Default)]
pub(crate) struct Cursor {
    frames: u64,
    sequence: u32,
    chain: Option<String>,
    origin: Option<i64>,
    pub(crate) missing: Vec<[u64; 2]>,
    pub(crate) coverage: [u64; 2],
    pub(crate) coverage_modified: [Option<SystemTime>; 2],
    owner: Option<String>,
    identity: Option<Identity>,
    metadata_bytes: usize,
}
impl Cursor {
    pub(crate) fn read(
        &mut self,
        path: &Path,
        owner: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Tip>, String> {
        if !crate::input_levels::valid_owner(owner)
            || self.owner.as_deref().is_some_and(|v| v != owner)
        {
            return Err("録音WAVの所有者が不正です".into());
        }
        let directory = Directory::open(path)?;
        let Some((tip_bytes, tip_stamp)) = directory.read_json("single-wav.json")? else {
            return Ok(None);
        };
        let tip: Tip = serde_json::from_slice(&tip_bytes).map_err(|e| e.to_string())?;
        if tip.schema_version != 1
            || tip.owner_id != owner
            || tip.file_name != "recording.wav"
            || tip.checkpoints > mon_core::capture::wav::MAX_CHECKPOINTS
            || tip.checkpoints < self.sequence
            || tip.frames < self.frames
            || tip.frames > 9_007_199_254_740_991
            || (tip.frames == 0) != (tip.checkpoints == 0)
            || (tip.frames == 0) != tip.capture_origin_nanoseconds.is_none()
            || tip.capture_origin_nanoseconds.is_some_and(|n| n < 0)
            || tip
                .last_entry_sha256
                .as_deref()
                .is_some_and(|s| !hash_valid(s))
        {
            return Err("録音WAVの確定範囲が不正です".into());
        }
        let (reservation_bytes, reservation_stamp) = directory
            .read_json("single-wav-owner.json")?
            .ok_or("録音WAVの所有予約がありません")?;
        let reservation: Reservation =
            serde_json::from_slice(&reservation_bytes).map_err(|e| e.to_string())?;
        if reservation.schema_version != 1
            || reservation.owner_id != owner
            || reservation.file_name != tip.file_name
            || reservation.birth_nanoseconds < 0
        {
            return Err("録音WAVの所有予約が不正です".into());
        }
        let file = directory.required(&tip.file_name, false)?;
        identity_matches(&file, &reservation)?;
        let file_stamp = Stamp::read(&file)?;
        let identity = Identity::file(&file)?;
        if self.identity.as_ref().is_some_and(|old| old != &identity) {
            return Err("録音WAVが差し替えられています".into());
        }
        let mut frames = self.frames;
        let mut chain = self.chain.clone();
        let mut origin = self.origin;
        let mut missing = Vec::new();
        let mut coverage = self.coverage;
        let mut coverage_modified = self.coverage_modified;
        let mut metadata = self.metadata_bytes;
        for sequence in self.sequence + 1..=tip.checkpoints {
            if cancelled() {
                return Err("録音WAVの照合を停止しました".into());
            }
            let (bytes, stamp) = directory
                .read_json(&format!("wav-{sequence:08}.json"))?
                .ok_or("録音WAVの確定記録が不足しています")?;
            metadata += bytes.len();
            if bytes.len() > 65 * 1024 || metadata > 32 * 1024 * 1024 {
                return Err("録音WAVの確定記録が上限を超えています".into());
            }
            let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if receipt.schema_version != 1
                || receipt.owner_id != owner
                || receipt.sequence != sequence
                || receipt.frame_start != frames
                || receipt.frames == 0
                || receipt.frames > mon_core::capture::wav::MAX_CHECKPOINT_FRAMES
                || receipt.capture_origin_nanoseconds < 0
                || origin.is_some_and(|v| v != receipt.capture_origin_nanoseconds)
                || receipt.previous_entry_sha256 != chain
                || !hash_valid(&receipt.pcm_sha256)
                || receipt.missing_ranges.len() > 8192
                || receipt.coverage_frames.iter().any(|n| *n > receipt.frames)
            {
                return Err("録音WAVの確定記録と音声が一致しません".into());
            }
            let end = frames
                .checked_add(receipt.frames)
                .ok_or("録音WAVの範囲が大きすぎます")?;
            let mut last = frames;
            for range in &receipt.missing_ranges {
                if range[0] < last || range[1] <= range[0] || range[1] > end {
                    return Err("録音WAVの欠損範囲が不正です".into());
                }
                last = range[1];
            }
            let mut pcm = vec![0; receipt.frames as usize * 2];
            crate::native_fs::read_exact_at(&file, &mut pcm, 80 + frames * 2)
                .map_err(|e| e.to_string())?;
            if mon_core::hash::sha256_hex(&pcm) != receipt.pcm_sha256 {
                return Err("録音WAVの確定PCMが変更されています".into());
            }
            let receipt_file = directory.required(&format!("wav-{sequence:08}.json"), false)?;
            stamp.require(&receipt_file)?;
            let modified = receipt_file
                .metadata()
                .map_err(|error| error.to_string())?
                .modified()
                .ok();
            origin = Some(receipt.capture_origin_nanoseconds);
            chain = Some(mon_core::hash::sha256_hex(&bytes));
            frames = end;
            for (lane, count) in receipt.coverage_frames.into_iter().enumerate() {
                coverage[lane] += count;
                if count > 0 {
                    coverage_modified[lane] = modified;
                }
            }
            missing.extend(receipt.missing_ranges);
        }
        if frames != tip.frames
            || chain != tip.last_entry_sha256
            || origin != tip.capture_origin_nanoseconds
        {
            return Err("録音WAVの確定ポインターと音声が一致しません".into());
        }
        identity_matches(&directory.required(&tip.file_name, false)?, &reservation)?;
        reservation_stamp.require(&directory.required("single-wav-owner.json", false)?)?;
        tip_stamp.require(&directory.required("single-wav.json", false)?)?;
        file_stamp.require(&file)?;
        self.owner = Some(owner.into());
        self.identity = Some(identity);
        self.frames = frames;
        self.sequence = tip.checkpoints;
        self.chain = chain;
        self.origin = origin;
        self.metadata_bytes = metadata;
        self.missing.extend(missing);
        self.coverage = coverage;
        self.coverage_modified = coverage_modified;
        Ok(Some(tip))
    }
}

mod recovery;

/// Same-invocation evidence, never serialized or reused across process starts.
/// Completion checks the committed DB reference and stable files, not PCM again.
pub(super) struct CommittedAudio {
    directory: PathBuf,
    audio_name: String,
    audio_stamp: Stamp,
    manifest_directory: PathBuf,
    manifest_name: String,
    manifest_stamp: Stamp,
    manifest_hash: String,
    hash: String,
    frames: u64,
}
impl CommittedAudio {
    pub(super) fn complete(
        &self,
        storage: &Storage,
        session: &RecordingSession,
    ) -> Result<bool, String> {
        let plan = storage.canonical_audio_plan(&session.meeting_id)?;
        let chunk = &plan.tracks[0].chunks[0];
        if plan.manifest_hash != self.manifest_hash
            || chunk.hash != self.hash
            || chunk.frames as u64 != self.frames
        {
            return Err("録音WAVの登録結果が変更されています".into());
        }
        let paths = || -> Result<_, String> {
            let audio = storage.meeting_mix_path(&session.meeting_id, &plan.tracks[0])?;
            let manifest: String = storage.connection.lock().map_err(|_| "DBロックを取得できません")?.query_row("SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2 AND content_hash=?3 AND state='ready'",params![plan.manifest_asset_id,session.meeting_id,self.manifest_hash],|row|row.get(0)).map_err(|error|error.to_string())?;
            Ok((audio, storage.managed_path(Path::new(&manifest))?))
        };
        let expected_paths = (
            self.directory.join(&self.audio_name),
            self.manifest_directory.join(&self.manifest_name),
        );
        if paths()? != expected_paths {
            return Err("録音WAVの登録先が変更されています".into());
        }
        let directory = Directory::open(&self.directory)?;
        self.audio_stamp
            .require(&directory.required(&self.audio_name, false)?)?;
        self.manifest_stamp.require(
            &Directory::open(&self.manifest_directory)?.required(&self.manifest_name, false)?,
        )?;
        if storage.canonical_audio_plan(&session.meeting_id)? != plan || paths()? != expected_paths
        {
            return Err("録音WAVの登録結果が変更されています".into());
        }
        storage.recording_save_committed(&session.id, &session.meeting_id)
    }
}

fn valid_clock(document: &serde_json::Value) -> Option<i64> {
    let clock = &document["captureClock"];
    if !matches!(
        clock["domain"].as_str(),
        Some("mach_continuous_time" | "query_performance_counter")
    ) || clock["uncertaintyNs"]
        .as_i64()
        .is_none_or(|n| !(0..=9_007_199_254_740_991).contains(&n))
    {
        return None;
    }
    clock["originUtcMs"]
        .as_i64()
        .filter(|n| (0..=8_640_000_000_000_000).contains(n))
}
fn valid_ending(document: &serde_json::Value, duration: i64) -> bool {
    let end = &document["captureEnd"];
    let (Some(microphone), Some(system), Some(ending)) = (
        end["microphoneClosedUs"].as_i64(),
        end["systemClosedUs"].as_i64(),
        end["endUs"].as_i64(),
    ) else {
        return false;
    };
    end["method"] == "acquisition_closed"
        && microphone >= 0
        && system >= 0
        && ending > 0
        && ending == microphone.max(system)
        && ending.abs_diff(duration) <= 63
}

impl Storage {
    pub(super) fn has_single_wav(&self, session: &RecordingSession) -> Result<bool, String> {
        let base = self
            .root
            .join("meetings")
            .join(&session.meeting_id)
            .join("audio")
            .join(&session.id);
        Ok(Directory::open(&base)?
            .optional("single-wav.json", false)?
            .is_some())
    }

    pub(super) fn finalize_single_wav_recording(
        &self,
        session: &RecordingSession,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(RecordingSession, CommittedAudio), String> {
        self.require_data_writable()?;
        crate::operation_observation::stage("checking_audio", None, "bytes");
        let _owner = self.begin_meeting_work(&session.meeting_id, "recording", None)?;
        let relative = Path::new("meetings")
            .join(&session.meeting_id)
            .join("audio")
            .join(&session.id);
        let base = self.managed_path(&relative)?;
        let directory = Directory::open(&base)?;
        let source = recovery::inspect(self, &directory, &session.id, cancelled)?;
        let original = recovery::document(&directory, "manifest.json")?;
        let source_document = original.as_ref().and_then(|(value, _, _)| value.as_ref());
        if let Some(document) = source_document {
            if document.get("captureTest").is_some() {
                return Err("録音テストの音声を会議へ登録できません".into());
            }
            if document
                .get("recordingOwnerId")
                .is_some_and(|owner| owner != &session.id)
            {
                return Err("録音manifestの所属が一致しません".into());
            }
        }
        let (config_text, initial_state): (String, String) = self.connection.lock()
            .map_err(|_| "DBロックを取得できません")?.query_row(
                "SELECT r.capture_config_json,m.lifecycle FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.id=?1 AND r.meeting_id=?2 AND m.source_type='recording' AND r.audio_manifest_hash IS NULL AND NOT EXISTS(SELECT 1 FROM restore_provenance p WHERE p.meeting_id=m.id AND p.domain='meetings' AND p.entity_key=m.id)",
                params![session.id, session.meeting_id], |row| Ok((row.get(0)?,row.get(1)?)))
            .map_err(|error| error.to_string())?;
        if !matches!(initial_state.as_str(), "recording" | "draft") {
            return Err("録音WAVを確定できる会議状態ではありません".into());
        }
        let config: RecordingConfig =
            serde_json::from_str(&config_text).map_err(|error| error.to_string())?;
        config.validate()?;
        let tip = &source.tip;
        let size = tip
            .frames
            .checked_mul(2)
            .and_then(|n| n.checked_add(80))
            .ok_or("録音WAVが大きすぎます")?;
        let duration = i64::try_from(
            tip.frames
                .checked_mul(1_000_000)
                .ok_or("録音時間が大きすぎます")?
                / 16000,
        )
        .map_err(|error| error.to_string())?;
        let started = source_document
            .filter(|document| {
                document["singleWav"]["captureOriginNanoseconds"].as_i64()
                    == tip.capture_origin_nanoseconds
            })
            .and_then(valid_clock)
            .filter(|_| tip.capture_origin_nanoseconds.is_some());
        let normal = source.original_closed
            && source_document.is_some_and(|document| {
                document["singleWav"] == serde_json::to_value(tip).unwrap_or_default()
                    && valid_ending(document, duration)
            })
            && started
                .and_then(|value| value.checked_add(duration / 1000))
                .is_some_and(|value| (0..=8_640_000_000_000_000).contains(&value));
        let header = crate::audio::wave::streaming_header(tip.frames)?;
        let (audio_relative, audio_stamp, hash) = if source.truncated {
            // The only source includes potentially rescuable later audio: retain
            // it byte-for-byte and materialize only the independently proven prefix.
            let key = mon_core::hash::sha256_hex(
                    format!("{}:{}:{}", session.id, tip.frames, source.pcm_hash).as_bytes()
            );
            let leaf = format!("wav-recovery-{key}");
            let output = if base.join(&leaf).exists() {
                Directory::open(&base.join(&leaf))?
            } else {
                directory.create_child(&leaf)?
            };
            let mut pending = crate::storage::audio_publication::writer::Pending::open(
                &output.path,
                &key,
                tip.frames,
            )?;
            let produced_hash = if let Some(mut file) = pending.begin()? {
                source
                    .stamp
                    .require(&directory.required("recording.wav", false)?)?;
                file.write_all(&header).map_err(|error| error.to_string())?;
                let mut hasher = Sha256::new();
                hasher.update(&header);
                let mut pcm_hasher = Sha256::new();
                let mut offset = 0_u64;
                let mut buffer = [0_u8; 65536];
                while offset < tip.frames * 2 {
                    if cancelled() {
                        return Err("録音WAVの救出を停止しました".into());
                    }
                    let count = (tip.frames * 2 - offset).min(buffer.len() as u64) as usize;
                    crate::native_fs::read_exact_at(
                        &source.file,
                        &mut buffer[..count],
                        80 + offset,
                    )
                    .map_err(|error| error.to_string())?;
                    file.write_all(&buffer[..count])
                        .map_err(|error| error.to_string())?;
                    hasher.update(&buffer[..count]);
                    pcm_hasher.update(&buffer[..count]);
                    offset += count as u64;
                }
                file.sync_all().map_err(|error| error.to_string())?;
                source
                    .stamp
                    .require(&directory.required("recording.wav", false)?)?;
                if mon_core::hash::sha256_output_hex(pcm_hasher.finalize()) != source.pcm_hash {
                    return Err("録音WAVの救出中にPCMが変更されました".into());
                }
                Some(mon_core::hash::sha256_output_hex(hasher.finalize()))
            } else {
                None
            };
            let hash = produced_hash
                .or_else(|| pending.completed_hash().map(str::to_owned))
                .ok_or("救出WAVの確定証拠がありません")?;
            pending.complete(hash.clone())?;
            let path = pending.path();
            let file = pending.completed_file()?;
            (
                path.strip_prefix(&self.root)
                    .map_err(|error| error.to_string())?
                    .to_owned(),
                Stamp::read(&file)?,
                hash,
            )
        } else {
            source
                .stamp
                .require(&directory.required("recording.wav", false)?)?;
            let mut file = directory.required("recording.wav", true)?;
            source.stamp.require(&file)?;
            let mut actual = [0; 80];
            crate::native_fs::read_exact_at(&file, &mut actual, 0)
                .map_err(|error| error.to_string())?;
            let repair = actual.as_slice() != header
                || file.metadata().map_err(|error| error.to_string())?.len() != size;
            if repair {
                file.set_len(size).map_err(|error| error.to_string())?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|error| error.to_string())?;
                file.write_all(&header).map_err(|error| error.to_string())?;
                file.sync_all().map_err(|error| error.to_string())?;
            }
            let stamp = Stamp::read(&file)?;
            // Header repair deliberately requires a bound readback. A normal
            // save already computed the final digest in the receipt scan.
            let hash = if repair || source.full_hash.is_none() {
                crate::audio::inspect_recorded_wav_bound(
                    file.try_clone().map_err(|error| error.to_string())?,
                    cancelled,
                    &source.pcm_hash,
                )?
                .2
            } else {
                source.full_hash.clone().unwrap()
            };
            stamp.require(&directory.required("recording.wav", false)?)?;
            (relative.join("recording.wav"), stamp, hash)
        };
        crate::operation_observation::stage("verifying_audio", None, "bytes");
        let key = mon_core::hash::sha256_hex(
            serde_json::to_vec(&serde_json::json!({
                "owner": session.id,
                "config": config_text,
                "frames": tip.frames,
                "lastReceipt": tip.last_entry_sha256,
                "pcm": source.pcm_hash,
            }))
            .map_err(|error| error.to_string())?,
        );
        let mut metadata_issues = Vec::new();
        if source_document.is_none() {
            metadata_issues.push("manifest_unavailable");
        }
        if started.is_none() {
            metadata_issues.push("capture_clock_unconfirmed");
        }
        if !normal {
            metadata_issues.push("capture_end_unconfirmed");
        }
        if !source.diagnostics_known {
            metadata_issues.push("receipt_diagnostics_unconfirmed");
        }
        if source_document.is_some_and(|document| {
            document["captureMode"] != config.mode
                || document["backend"] != config.backend
                || document["targetPIDs"] != serde_json::json!(config.target_pids)
                || document["audioFormat"] != "wav"
        }) {
            metadata_issues.push("capture_description_mismatch");
        }
        if !metadata_issues.is_empty() {
            crate::internal_logs::detail(
                crate::internal_logs::current_context("recording"),
                "metadata_unconfirmed",
                "info",
                std::time::Duration::ZERO,
                &serde_json::json!({"issues":metadata_issues}).to_string(),
            );
        }
        let mut document = serde_json::json!({"schemaVersion":5,"audioFormat":"wav","clockMappingSource":"single_wav_v1",
            "inputDiagnosticsState":source_document.and_then(|document|document["inputDiagnosticsState"].as_str()).filter(|state|matches!(*state,"reported"|"unconfirmed"|"unavailable")).unwrap_or("unavailable"),
            "recordingOwnerId":session.id,"captureMode":config.mode,"backend":config.backend,"targetPIDs":config.target_pids,
            "microphoneDeviceUID":config.microphone_device_uid,"targetApplicationIdentity":config.target_application_identity,
            "state":if normal {"saved"}else{"interrupted"},"errors":[],"singleWav":tip,
            "captureClock":if started.is_some() {source_document.map(|value|value["captureClock"].clone()).unwrap_or_default()}else{serde_json::Value::Null},
            "captureEnd":if normal {source_document.map(|value|value["captureEnd"].clone()).unwrap_or_default()}else{serde_json::Value::Null},
            "wavPublication":{"schemaVersion":1,"policy":"meeting_live_wav_interval_gain_v1","frames":tip.frames,"sha256":hash,"missingRanges":source.missing,"acquisitionEndKnown":normal},
            "wavRecovery":{"schemaVersion":1,"inputHash":key,"partial":source.truncated,"diagnosticsKnown":source.diagnostics_known,"metadataIssues":metadata_issues,"failure":source.fault}});
        let recovery_result = document["wavRecovery"].clone();
        // Descriptive metadata is frozen only after selecting the same audio
        // evidence. Earlier unregistered documents never block another result.
        let mut manifest_name = "canonical-wav.json".to_owned();
        let mut previous = recovery::document(&directory, &manifest_name)?;
        if previous.as_ref().is_some_and(|(value, _, _)| {
            value
                .as_ref()
                .is_none_or(|value| value["wavRecovery"]["inputHash"] != key)
        }) {
            manifest_name = format!("recovered-wav-{key}.json");
            previous = recovery::document(&directory, &manifest_name)?;
        }
        let (canonical_stamp, canonical_bytes) = if let Some((value, stamp, bytes)) = previous {
            let value = value.ok_or("録音WAVの公開文書を確認できません")?;
            if value["recordingOwnerId"] != session.id
                || value["wavRecovery"]["inputHash"] != key
                || value["wavPublication"]["frames"] != tip.frames
                || value["wavPublication"]["sha256"] != hash
            {
                return Err("録音WAVの公開対象が変更されています".into());
            }
            document = value;
            (stamp, bytes)
        } else {
            directory.capture_json(&manifest_name, &document, None)?;
            let (value, stamp, bytes) = recovery::document(&directory, &manifest_name)?
                .ok_or("録音WAVの公開証拠がありません")?;
            if value.as_ref() != Some(&document) {
                return Err("録音WAVの公開証拠が変更されています".into());
            }
            (stamp, bytes)
        };
        let normal = source.original_closed
            && !source.truncated
            && document["singleWav"] == serde_json::to_value(tip).unwrap_or_default()
            && document["wavPublication"]["acquisitionEndKnown"] == true
            && valid_ending(&document, duration);
        let started = valid_clock(&document);
        let ended = if normal {
            started
                .and_then(|value| value.checked_add(duration / 1000))
                .filter(|value| (0..=8_640_000_000_000_000).contains(value))
        } else {
            None
        };
        let normal = normal && ended.is_some();
        let proof = CapturedFile {
            relative: relative.join(&manifest_name),
            size_bytes: canonical_bytes.len() as i64,
            hash: mon_core::hash::sha256_hex(&canonical_bytes),
        };
        let audio_proof = CapturedFile {
            relative: audio_relative.clone(),
            size_bytes: size as i64,
            hash: hash.clone(),
        };
        if cancelled() {
            return Err("録音WAVの登録を停止しました".into());
        }
        crate::operation_observation::stage("committing_audio", None, "files");
        let audio_directory = Directory::open(
            &self
                .root
                .join(audio_relative.parent().ok_or("録音WAVの保存先が不正です")?),
        )?;
        let audio_name = audio_relative
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("録音WAVの名前が不正です")?
            .to_owned();
        audio_stamp.require(&audio_directory.required(&audio_name, false)?)?;
        canonical_stamp.require(&directory.required(&manifest_name, false)?)?;
        let mut db = self
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let tx = db.transaction().map_err(|error| error.to_string())?;
        if load_session(&tx, &session.id)?.as_ref() != Some(session) {
            return Err("録音WAVの検証中に状態が変更されました".into());
        }
        let current:(String,String)=tx.query_row(
            "SELECT r.capture_config_json,m.lifecycle FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.id=?1 AND r.meeting_id=?2 AND m.source_type='recording' AND r.audio_manifest_hash IS NULL",
            params![session.id,session.meeting_id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|error|error.to_string())?;
        if current != (config_text, initial_state) {
            return Err("録音WAVの検証中に設定・会議状態が変更されました".into());
        }
        super::super::deletion::audio::require_source(&tx, &session.meeting_id)?;
        let now = now_ms()?;
        let manifest_id =
            insert_capture_asset(&tx, &session.meeting_id, &proof, "application/json", now)?;
        let audio_id =
            insert_capture_asset(&tx, &session.meeting_id, &audio_proof, "audio/wav", now)?;
        let track = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES(?1,?2,?3,'processed','wav',16000,1,'ready',?4,?5,?5)",params![track,session.meeting_id,session.id,serde_json::json!({"policy":super::super::meeting_audio::POLICY,"liveMixPolicy":"interval_gain_v1","wavRecovery":recovery_result}).to_string(),now]).map_err(|error|error.to_string())?;
        tx.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,?5,?6,16000,1,'ready',?7)",params![Uuid::new_v4().to_string(),session.meeting_id,track,audio_id,duration,tip.frames as i64,now]).map_err(|error|error.to_string())?;
        for range in &source.missing {
            let start = (range[0] * 1_000_000 / 16000) as i64;
            let end = (range[1] * 1_000_000 / 16000) as i64;
            if end > start {
                tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES(?1,?2,?3,?4,'meeting_input_missing',0)",params![Uuid::new_v4().to_string(),track,start,end]).map_err(|error|error.to_string())?;
            }
        }
        tx.execute("UPDATE recording_sessions SET state=?7,duration_us=?2,last_committed_us=?2,audio_manifest_hash=?3,started_at_utc_ms=?4,ended_at_utc_ms=?5,updated_at=?6 WHERE id=?1",params![session.id,duration,proof.hash,started,ended,now,if normal{"saved"}else{"interrupted"}]).map_err(|error|error.to_string())?;
        if tx.execute("UPDATE meetings SET lifecycle='recorded',duration_us=?2,started_at_utc_ms=?3,lock_version=lock_version+1,updated_at=?4 WHERE id=?1 AND source_type='recording' AND lifecycle IN ('recording','draft')",params![session.meeting_id,duration,started,now]).map_err(|error|error.to_string())?!=1 {return Err("録音WAVの会議状態を確定できません".into());}
        tx.execute("INSERT INTO meeting_audio_commits(meeting_id,track_id,manifest_asset_id,manifest_hash,frames,source_json,cleanup_json,phase,created_at) VALUES(?1,?2,?3,?4,?5,?6,'[]','cleaned',?7)",params![session.meeting_id,track,manifest_id,proof.hash,tip.frames as i64,serde_json::json!({"singleWav":tip,"liveMixPolicy":"interval_gain_v1","wavRecovery":recovery_result}).to_string(),now]).map_err(|error|error.to_string())?;
        super::super::analysis_flows::ensure_flow(&tx, &session.meeting_id)?;
        let saved = load_session(&tx, &session.id)?.ok_or("録音セッションがありません")?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok((
            saved,
            CommittedAudio {
                directory: audio_directory.path,
                audio_name,
                audio_stamp,
                manifest_directory: base,
                manifest_name: proof
                    .relative
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                manifest_stamp: canonical_stamp,
                manifest_hash: proof.hash,
                hash,
                frames: tip.frames,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn fixture(closed: bool) -> (tempfile::TempDir, Storage, RecordingSession, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let storage = Storage::open(root.path().join("store")).unwrap();
        let meeting = storage.create_meeting("WAV直接保存").unwrap();
        let config = RecordingConfig {
            mode: "system".into(),
            backend: "coreaudio".into(),
            target_pids: vec![],
            microphone_device_uid: None,
            target_application_identity: None,
        };
        let session = storage
            .prepare_recording(&meeting.id, &config, &Uuid::new_v4().to_string())
            .unwrap();
        storage.mark_recording_started(&session.id).unwrap();
        let session = load_session(&storage.connection.lock().unwrap(), &session.id)
            .unwrap()
            .unwrap();
        let base = storage
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("audio")
            .join(&session.id);
        std::fs::create_dir_all(&base).unwrap();
        let mut options = File::options();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(base.join("recording.wav")).unwrap();
        let frames = 32000_u64;
        file.write_all(
            &crate::audio::wave::streaming_header(if closed { frames } else { 0 }).unwrap(),
        )
        .unwrap();
        let mut pcm = vec![0; frames as usize * 2];
        for sample in pcm[..32000].chunks_exact_mut(2) {
            sample.copy_from_slice(&6400_i16.to_le_bytes());
        }
        file.write_all(&pcm).unwrap();
        if !closed {
            file.write_all(b"unadvertised-tail").unwrap();
        }
        file.sync_all().unwrap();
        let m = crate::native_fs::metadata(&file).unwrap();
        let owner = serde_json::json!({"schemaVersion":1,"ownerId":session.id,"fileName":"recording.wav","device":m.dev(),"inode":m.ino(),"birthNanoseconds":m.created().unwrap().duration_since(UNIX_EPOCH).unwrap().as_nanos() as i64});
        std::fs::write(
            base.join("single-wav-owner.json"),
            serde_json::to_vec(&owner).unwrap(),
        )
        .unwrap();
        let entry=serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"ownerId":session.id,"sequence":1,"frameStart":0,"frames":frames,"captureOriginNanoseconds":1_000_000_000_i64,"pcmSHA256":mon_core::hash::sha256_hex(&pcm),"previousEntrySHA256":null,"missingRanges":[[16000,32000]],"coverageFrames":[16000,16000]})).unwrap();
        std::fs::write(base.join("wav-00000001.json"), &entry).unwrap();
        let tip = serde_json::json!({"schemaVersion":1,"ownerId":session.id,"fileName":"recording.wav","frames":frames,"checkpoints":1,"lastEntrySHA256":mon_core::hash::sha256_hex(&entry),"captureOriginNanoseconds":1_000_000_000_i64,"producerClosed":closed});
        std::fs::write(
            base.join("single-wav.json"),
            serde_json::to_vec(&tip).unwrap(),
        )
        .unwrap();
        let manifest = serde_json::json!({"schemaVersion":5,"audioFormat":"wav","clockMappingSource":"single_wav_v1","inputDiagnosticsState":"reported","recordingOwnerId":session.id,"captureMode":"system","backend":"coreaudio","targetPIDs":[],"state":if closed {"saved"}else{"failed"},"errors":[],"captureClock":{"domain":"mach_continuous_time","originUtcMs":1234567890000_i64,"uncertaintyNs":1000000},"captureEnd":{"method":"acquisition_closed","microphoneClosedUs":2000000,"systemClosedUs":2000000,"endUs":2000000},"singleWav":tip});
        std::fs::write(
            base.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        for leaf in [
            "recording.wav",
            "single-wav-owner.json",
            "wav-00000001.json",
            "single-wav.json",
            "manifest.json",
        ] {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(base.join(leaf), std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            #[cfg(windows)]
            crate::windows_files::set_private(&base.join(leaf)).unwrap();
        }
        (root, storage, session, base)
    }

    #[test]
    fn canonical_live_wav_is_adopted_once_without_rewriting_pcm() {
        let (_root, s, session, base) = fixture(true);
        let before = std::fs::read(base.join("recording.wav")).unwrap();
        let id = Identity::file(&File::open(base.join("recording.wav")).unwrap()).unwrap();
        s.finalize_available_recording(&session.id, &|| false)
            .unwrap();
        s.finalize_available_recording(&session.id, &|| false)
            .unwrap();
        assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), before);
        id.require(&File::open(base.join("recording.wav")).unwrap())
            .unwrap();
        let plan = s
            .verify_canonical_audio(&session.meeting_id, &|| false)
            .unwrap();
        assert_eq!(plan.tracks.len(), 1);
        assert_eq!(plan.tracks[0].chunks[0].frames, 32000);
        assert!(s
            .recording_save_complete(&session.id, &session.meeting_id, &|| false)
            .unwrap());
    }

    #[test]
    fn recovery_repairs_only_the_owned_committed_prefix() {
        let (_root, s, session, base) = fixture(false);
        s.finalize_available_recording(&session.id, &|| false)
            .unwrap();
        assert_eq!(
            std::fs::metadata(base.join("recording.wav")).unwrap().len(),
            80 + 64000
        );
        let saved = load_session(&s.connection.lock().unwrap(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.state, "interrupted");
        assert_eq!(saved.ended_at_utc_ms, None);
        s.verify_canonical_audio(&session.meeting_id, &|| false)
            .unwrap();
    }

    #[test]
    fn foreign_manifest_and_capture_test_never_publish_audio() {
        for invalid in ["test_marker", "owner"] {
            let (_root, storage, session, base) = fixture(true);
            let path = base.join("manifest.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            match invalid {
                "test_marker" => value["captureTest"] = serde_json::Value::Null,
                "owner" => value["recordingOwnerId"] = Uuid::new_v4().to_string().into(),
                _ => unreachable!(),
            }
            std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
            let before = std::fs::read(base.join("recording.wav")).unwrap();
            assert!(
                storage
                    .finalize_available_recording(&session.id, &|| false)
                    .is_err(),
                "{invalid}"
            );
            assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), before);
            assert!(!base.join("canonical-wav.json").exists());
            assert!(storage.canonical_audio_plan(&session.meeting_id).is_err());
        }
    }

    #[test]
    fn concurrent_configuration_or_meeting_change_cannot_publish_a_verified_wav() {
        for configuration in [true, false] {
            let (_root, storage, session, base) = fixture(true);
            let changed = std::cell::Cell::new(false);
            let before = std::fs::read(base.join("recording.wav")).unwrap();
            let result = storage.finalize_available_recording(&session.id, &|| {
                if base.join("canonical-wav.json").exists() && !changed.replace(true) {
                    let db = storage.connection.lock().unwrap();
                    if configuration {
                        db.execute("UPDATE recording_sessions SET capture_config_json=json_set(capture_config_json,'$.microphoneDeviceUID','changed-after-verification') WHERE id=?1", [&session.id]).unwrap();
                    } else {
                        db.execute("UPDATE meetings SET lifecycle='draft',lock_version=lock_version+1 WHERE id=?1", [&session.meeting_id]).unwrap();
                    }
                }
                false
            });
            assert!(changed.get());
            assert!(result.is_err());
            assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), before);
            assert!(storage.canonical_audio_plan(&session.meeting_id).is_err());
        }
    }

    #[test]
    fn closed_pcm_changed_at_the_final_callback_is_retained_without_registration() {
        let (_root, storage, session, base) = fixture(true);
        let changed = std::cell::Cell::new(false);
        let result = storage.finalize_available_recording(&session.id, &|| {
            if base.join("canonical-wav.json").exists() && !changed.replace(true) {
                let mut file = File::options()
                    .write(true)
                    .open(base.join("recording.wav"))
                    .unwrap();
                file.seek(SeekFrom::Start(80)).unwrap();
                file.write_all(&[0x33]).unwrap();
            }
            false
        });
        assert!(changed.get());
        assert!(result.is_err());
        assert_eq!(std::fs::read(base.join("recording.wav")).unwrap()[80], 0x33);
        assert!(storage.canonical_audio_plan(&session.meeting_id).is_err());
    }

    #[test]
    fn inode_replacement_and_committed_pcm_tampering_never_authorize_repair() {
        for replacement in [false, true] {
            let (_root, s, session, base) = fixture(false);
            let path = base.join("recording.wav");
            if replacement {
                let bytes = std::fs::read(&path).unwrap();
                std::fs::rename(&path, base.join("original.wav")).unwrap();
                std::fs::write(&path, bytes).unwrap();
            } else {
                let mut f = File::options().write(true).open(&path).unwrap();
                f.seek(SeekFrom::Start(81)).unwrap();
                f.write_all(&[99]).unwrap();
            }
            let before = std::fs::read(&path).unwrap();
            assert!(s
                .finalize_available_recording(&session.id, &|| false)
                .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert!(s.canonical_audio_plan(&session.meeting_id).is_err());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires the built Swift native test bundle; make check-single-wav"]
    fn swift_generated_single_wav_fixture_can_be_published_and_reopened() {
        let (_root, s, session, base) = fixture(true);
        for name in [
            "recording.wav",
            "single-wav-owner.json",
            "single-wav.json",
            "wav-00000001.json",
            "manifest.json",
        ] {
            std::fs::remove_file(base.join(name)).unwrap();
        }
        let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/macos-capture");
        let result = std::process::Command::new("swift")
            .args([
                "test",
                "--skip-build",
                "--disable-sandbox",
                "--package-path",
            ])
            .arg(&package)
            .args(["--filter", "LiveWavSessionTests/testExportSingleWavFixture"])
            .env("MON_SINGLE_WAV_FIXTURE_DIR", &base)
            .env("MON_SINGLE_WAV_OWNER", &session.id)
            .env("MON_SINGLE_WAV_BACKEND", "coreaudio")
            .env(
                "SWIFTPM_MODULECACHE_OVERRIDE",
                "/tmp/mon-swift-module-cache",
            )
            .env("CLANG_MODULE_CACHE_PATH", "/tmp/mon-clang-module-cache")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Swift fixture failed: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let original = std::fs::read(base.join("recording.wav")).unwrap();
        s.finalize_available_recording(&session.id, &|| false)
            .unwrap();
        assert_eq!(std::fs::read(base.join("recording.wav")).unwrap(), original);
        s.verify_canonical_audio(&session.meeting_id, &|| false)
            .unwrap();
        assert!(s
            .recording_save_complete(&session.id, &session.meeting_id, &|| false)
            .unwrap());
    }
}

#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod compatibility_tests;
