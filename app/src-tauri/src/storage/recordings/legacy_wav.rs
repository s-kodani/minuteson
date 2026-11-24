//! One-way rescue of a verified legacy native prefix. Its original gain and
//! affine clocks feed the existing bounded mixer directly, without FLAC.
use super::*;
use crate::audio_clock::{ClockChunk, ClockLayout};
use crate::playback::{
    AudioSpan, ClockAudio, MeetingAudio, PlaybackMissingRange, PreparedAudio, SpanAudio,
};
use crate::recording_segments::{Lane, Prefix};
use crate::storage::audio_publication::{cleanup, writer::Pending};
use crate::storage::publication_files::{Directory, LegacyNativeManifestStamp, Stamp};
use std::io::{BufWriter, Read, Write};
use std::sync::Arc;

fn read_canonical_document(directory: &Directory) -> Result<Option<Vec<u8>>, String> {
    let name = "legacy-canonical-wav.json";
    let Some(mut file) = directory.optional(name, false)? else {
        return Ok(None);
    };
    let stamp = Stamp::read(&file)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(crate::capture_manifest::MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    crate::capture_manifest::validate_size(&bytes)?;
    stamp.require(&file)?;
    stamp.require(&directory.required(name, false)?)?;
    Ok(Some(bytes))
}

/// An old native save may have registered its FLAC tracks just before the
/// application stopped. Prefer the still-verifiable capture prefix directly;
/// an existing canonical commit or a different provenance is never rewritten.
pub(super) fn migrate_registered(
    storage: &Storage,
    session: &RecordingSession,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<RecordingSession>, String> {
    if session.state != "saved" {
        return Ok(None);
    }
    let base = Path::new("meetings")
        .join(&session.meeting_id)
        .join("audio")
        .join(&session.id);
    let relative = base
        .join("manifest.json")
        .to_string_lossy()
        .replace('\\', "/");
    let admitted: Option<(String, String, String)> = {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let canonical: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM meeting_audio_commits WHERE meeting_id=?1)",
                [&session.meeting_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if canonical {
            return Ok(None);
        }
        db.query_row("SELECT r.audio_manifest_hash,r.capture_mode,r.capture_config_json FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.id=?1 AND r.meeting_id=?2 AND m.source_type='recording' AND EXISTS(SELECT 1 FROM file_assets f WHERE f.meeting_id=m.id AND f.relative_path=?3 AND f.content_hash=r.audio_manifest_hash AND f.media_type='application/json' AND f.state='ready')",
            params![session.id,session.meeting_id,relative],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|error|error.to_string())?
    };
    let Some((manifest_hash, mode, config_text)) = admitted else {
        return Ok(None);
    };
    let proof = storage.checked_capture_file(&base.join("manifest.json"), None)?;
    if proof.hash != manifest_hash {
        return Err("登録済み旧録音のmanifestが変更されています".into());
    }
    let mut manifest = storage.read_capture_manifest(&proof)?;
    if manifest.schema_version != crate::current_formats::LEGACY_CAPTURE
        || manifest.state != "saved"
        || !manifest.errors.is_empty()
        || (manifest.microphone.raw_segments.is_none() && manifest.system.raw_segments.is_none())
    {
        return Ok(None);
    }
    let config: RecordingConfig =
        serde_json::from_str(&config_text).map_err(|error| error.to_string())?;
    config.validate()?;
    if manifest.capture_mode != mode
        || manifest.capture_mode != config.mode
        || manifest.backend != config.backend
        || manifest.target_pids != config.target_pids
        || config
            .microphone_device_uid
            .as_ref()
            .is_some_and(|uid| manifest.microphone_device_uid.as_ref() != Some(uid))
        || config
            .target_application_identity
            .as_ref()
            .is_some_and(|identity| manifest.target_application_identity.as_ref() != Some(identity))
    {
        return Err("登録済み旧録音と開始条件が一致しません".into());
    }
    let mic_missing = manifest.validated_unreceived_microphone(&session.id)?;
    let system_missing = manifest.validated_unreceived_system(&session.id)?;
    if mic_missing && system_missing {
        return Err(NO_CAPTURED_AUDIO.into());
    }
    require_unreceived_caf_absent(&storage.root.join(&base), mic_missing, system_missing)?;
    if !mic_missing {
        manifest.microphone.validated_format()?;
    }
    if !system_missing {
        manifest.system.validated_format()?;
    }
    let prefixes = match storage.verify_completed_raw_prefixes(&base, &manifest, &session.id) {
        Ok(prefixes) => prefixes,
        Err(error) if !cancelled() => {
            // Already registered FLAC remains an independent, hashed input.
            // The existing publication path must verify it before use; this
            // branch does not mint a capability from a damaged raw prefix.
            crate::internal_logs::detail(
                crate::internal_logs::current_context("recording"),
                "legacy_raw_rescue_unavailable",
                "warning",
                std::time::Duration::ZERO,
                &error,
            );
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if prefixes.is_empty() {
        return Ok(None);
    }
    let (mic, system) = flac::canonical_maps(&manifest, &prefixes)?;
    if !mic_missing {
        manifest.microphone.clock_segments = Some(mic);
    }
    if !system_missing {
        manifest.system.clock_segments = Some(system);
    }
    let capture_clock = manifest.validated_capture_clock()?;
    let empty = clock::CaptureTimeline {
        start_us: 0,
        end_us: 0,
        segments: &[],
    };
    let mic = if mic_missing {
        empty
    } else {
        manifest
            .microphone
            .validated_timeline(capture_clock.is_some())?
    };
    let system = if system_missing {
        clock::CaptureTimeline {
            start_us: 0,
            end_us: 0,
            segments: &[],
        }
    } else {
        manifest
            .system
            .validated_timeline(capture_clock.is_some())?
    };
    let first = if mic_missing {
        system.start_us
    } else if system_missing {
        mic.start_us
    } else {
        mic.start_us.min(system.start_us)
    };
    if capture_clock.is_some() && first != 0 {
        return Err("登録済み旧録音の共通時刻原点が不正です".into());
    }
    let ending = manifest.validated_capture_end(&session.id, &mic, &system)?;
    let duration = ending.map_or_else(|| mic.end_us.max(system.end_us), |end| end.end_us);
    let started = capture_clock
        .map(|clock| clock.origin_utc_ms)
        .or(session.started_at_utc_ms);
    let ended = if let (Some(clock), Some(end)) = (capture_clock, ending) {
        Some(
            clock
                .origin_utc_ms
                .checked_add(end.end_us / 1000)
                .filter(|value| (0..=8_640_000_000_000_000).contains(value))
                .ok_or("旧録音の終了時刻が範囲外です")?,
        )
    } else {
        None
    };
    let origin = prefixes
        .iter()
        .filter(|prefix| prefix.frames() > 0)
        .map(Prefix::first_capture_nanoseconds)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .min()
        .ok_or(NO_CAPTURED_AUDIO)?;
    let directory = Directory::open(&storage.managed_path(&base)?)?;
    let (bytes, stamp) = directory
        .read_legacy_native_manifest()?
        .ok_or("旧録音manifestがありません")?;
    if mon_core::hash::sha256_hex(&bytes) != proof.hash {
        return Err("旧録音manifestが検証中に変更されました".into());
    }
    let document = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    stamp.require(&directory)?;
    save(
        storage,
        session,
        &base,
        document,
        &prefixes,
        origin,
        duration,
        started,
        ended,
        None,
        Some(&proof),
        cancelled,
    )
    .map(Some)
}

/// The caller owns the closed recording and has already admitted its native
/// manifest (normal completion) or exact partial-review generation.
#[allow(clippy::too_many_arguments)]
pub(super) fn save(
    storage: &Storage,
    session: &RecordingSession,
    base: &Path,
    mut document: serde_json::Value,
    prefixes: &[Prefix],
    origin_ns: i64,
    duration_us: i64,
    started: Option<i64>,
    ended: Option<i64>,
    expected_meeting: Option<(&str, i64)>,
    expected_manifest: Option<&CapturedFile>,
    cancelled: &impl Fn() -> bool,
) -> Result<RecordingSession, String> {
    storage.require_data_writable()?;
    if duration_us <= 0 || origin_ns < 0 || prefixes.is_empty() || prefixes.len() > 2 {
        return Err("旧録音のWAV救出条件が不正です".into());
    }
    let base_path = storage.managed_path(base)?;
    let directory = Directory::open(&base_path)?;
    let (config, initial_state, initial_version): (String, String, i64) = storage.connection.lock()
        .map_err(|_| "DBロックを取得できません")?
        .query_row("SELECT r.capture_config_json,m.lifecycle,m.lock_version FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.id=?1 AND r.meeting_id=?2",
            params![session.id,session.meeting_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))
        .map_err(|error| error.to_string())?;
    if expected_meeting
        .is_some_and(|expected| expected != (initial_state.as_str(), initial_version))
    {
        return Err("旧録音の会議状態が確認後に変更されました".into());
    }
    let partial = !document["partialRecovery"].is_null();
    // A previous application may already have registered its derived FLAC
    // inputs. They are cleanup candidates, never the source of this rescue.
    let registered_source = if !partial {
        let db = storage
            .connection
            .lock()
            .map_err(|_| "DBロックを取得できません")?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM audio_tracks WHERE meeting_id=?1 AND session_id=?2 AND kind IN ('mic','system'))",
            params![session.meeting_id,session.id],|row|row.get(0)).map_err(|error|error.to_string())?;
        exists
            .then(|| crate::storage::audio_publication::inputs::read(&db, &session.meeting_id))
            .transpose()?
    } else {
        None
    };
    let original_manifest: Option<LegacyNativeManifestStamp> = if partial {
        None
    } else {
        let expected = expected_manifest.ok_or("旧録音manifestの検証証拠がありません")?;
        let (bytes, stamp) = directory
            .read_legacy_native_manifest()?
            .ok_or("旧録音manifestがありません")?;
        if expected.relative != base.join("manifest.json")
            || u64::try_from(expected.size_bytes).ok() != Some(bytes.len() as u64)
            || expected.hash != mon_core::hash::sha256_hex(&bytes)
            || serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|error| error.to_string())?
                != document
        {
            return Err("旧録音manifestが検証後に変更されています".into());
        }
        Some(stamp)
    };
    let generations = prefixes
        .iter()
        .map(Prefix::content_hash)
        .collect::<Result<Vec<_>, _>>()?;
    let local_generations = prefixes
        .iter()
        .map(Prefix::review_hash)
        .collect::<Result<Vec<_>, _>>()?;
    let key = mon_core::hash::sha256_hex(
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "policy": "legacy_capture_constant_gain_v1",
            "owner": session.id,
            "origin": origin_ns,
            "durationUs": duration_us,
            "prefixes": generations,
            "localGenerations": local_generations,
            "partialReview": document["partialRecovery"]["reviewHash"],
        }))
        .map_err(|error| error.to_string())?,
    );
    let frames = (duration_us as u64)
        .checked_mul(16000)
        .ok_or("旧録音の時間が範囲外です")?
        .div_ceil(1_000_000);
    if frames == 0 || frames > i64::MAX as u64 {
        return Err("旧録音のフレーム数が不正です".into());
    }
    let mut sources = Vec::new();
    let mut gaps = Vec::new();
    let mut coverage = Vec::new();
    for prefix in prefixes {
        prefix.verify_at(&base_path, cancelled)?;
        if prefix.frames() == 0 {
            continue;
        }
        if cancelled() {
            return Err("旧録音のWAV救出を停止しました".into());
        }
        let prefix = Arc::new(prefix.try_clone()?);
        let groups = prefix.flac_groups_with_seconds(origin_ns, 240)?;
        let span = SpanAudio::legacy_prefix(prefix.clone(), 0, prefix.entries().len())?;
        let rate = span.format.sample_rate;
        let channels = span.format.channels;
        let chunks: Vec<_> = groups
            .iter()
            .map(|group| ClockChunk {
                frames: group.frames as i64,
                start_us: group.clock_segments[0].timeline_start_us,
                end_us: group.clock_segments.last().unwrap().timeline_end_us,
                segments: &group.clock_segments,
            })
            .collect();
        let layout = ClockLayout::new(&chunks, rate)?;
        let kind = match prefix.lane() {
            Lane::Microphone => "mic",
            Lane::System => "system",
        };
        for pair in layout.spans.windows(2) {
            if pair[0].end_us < pair[1].start_us {
                gaps.push(PlaybackMissingRange::new(
                    kind,
                    pair[0].end_us,
                    pair[1].start_us,
                    "capture_timestamp_gap",
                    duration_us,
                )?);
            }
        }
        coverage.extend(layout.spans.iter().map(|span| (span.start_us, span.end_us)));
        let start_us = layout.start_us;
        let end_us = layout.end_us;
        let clock = ClockAudio::for_meeting(layout, rate)?;
        sources.push(PreparedAudio {
            sample_rate: rate,
            channels,
            start_us,
            end_us,
            track_kind: kind.into(),
            alignment: "unverified".into(),
            spans: vec![AudioSpan {
                frames: span.format.frames,
                first_frame: 0,
                audio: span,
            }],
            clock: Some(clock),
        });
    }
    if sources.is_empty() {
        return Err(NO_CAPTURED_AUDIO.into());
    }
    let mut audio = MeetingAudio::with_rate(sources, duration_us, gaps, 16000)?;
    let mut wave_directory =
        Directory::open(&storage.managed_path(&Path::new("meetings").join(&session.meeting_id))?)?;
    for leaf in ["audio", "meeting-wave", key.as_str()] {
        wave_directory = if wave_directory.path.join(leaf).symlink_metadata().is_ok() {
            Directory::open(&wave_directory.path.join(leaf))?
        } else {
            wave_directory.create_child(leaf)?
        };
    }
    let mut pending = Pending::open(&wave_directory.path, &key, frames)?;
    let hash = if let Some(hash) = pending.completed_hash() {
        hash.to_owned()
    } else {
        let mut writer = BufWriter::new(pending.begin()?.ok_or("旧録音WAVの保存所有がありません")?);
        let header = crate::audio::wave::header(frames)?;
        writer
            .write_all(&header)
            .map_err(|error| error.to_string())?;
        let mut hash = Sha256::new();
        hash.update(&header);
        let mut first = 0;
        while first < frames {
            if cancelled() {
                return Err("旧録音のWAV救出を停止しました".into());
            }
            let count = (frames - first).min(8192);
            let pcm = audio.read_analysis_pcm(first, count)?;
            let mut bytes = Vec::with_capacity(pcm.len() / 2);
            for sample in pcm.chunks_exact(4) {
                let value = f32::from_le_bytes(sample.try_into().unwrap());
                bytes.extend(((value.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
            }
            hash.update(&bytes);
            writer
                .write_all(&bytes)
                .map_err(|error| error.to_string())?;
            first += count;
            crate::operation_observation::report("mixing_audio", first, frames, "frames");
        }
        writer.flush().map_err(|error| error.to_string())?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|error| error.to_string())?;
        mon_core::hash::sha256_output_hex(hash.finalize())
    };
    pending.complete(hash.clone())?;
    audio.verify_sources()?;
    drop(audio);
    let (_, size, verified_hash) = crate::audio::inspect_recorded_wav(
        File::open(pending.path()).map_err(|error| error.to_string())?,
        cancelled,
    )?;
    if verified_hash != hash {
        return Err("旧録音WAVの保存内容が一致しません".into());
    }
    document["wavPublication"] = serde_json::json!({"schemaVersion":1,"policy":"legacy_capture_constant_gain_v1","frames":frames,"sha256":hash,"inputHash":key});
    let manifest_bytes = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
    if let Some(old) = read_canonical_document(&wave_directory)? {
        if old != manifest_bytes {
            return Err("旧録音WAVの公開条件が変更されています".into());
        }
    } else {
        wave_directory.capture_json("legacy-canonical-wav.json", &document, None)?;
    }
    let manifest = CapturedFile {
        relative: wave_directory
            .path
            .strip_prefix(&storage.root)
            .map_err(|_| "旧録音WAVの記録先が管理領域外です")?
            .join("legacy-canonical-wav.json"),
        size_bytes: manifest_bytes.len() as i64,
        hash: mon_core::hash::sha256_hex(&manifest_bytes),
    };
    let wave = CapturedFile {
        relative: pending
            .path()
            .strip_prefix(&storage.root)
            .map_err(|_| "旧録音WAVの保存先が管理領域外です")?
            .to_path_buf(),
        size_bytes: size as i64,
        hash,
    };
    let mut retired = Vec::new();
    let mut corrupt_assets: Vec<(String, String, i64, String, Stamp)> = Vec::new();
    for prefix in prefixes {
        // A partial rescue preserves the complete original workspace, including
        // verified entries and the unreadable/unadvertised suffix. The new WAV
        // is independently usable; source retirement is not a save condition.
        for entry in prefix.entries().filter(|_| !partial) {
            let path = base.join(format!(
                "raw-{}-{:08}.caf",
                prefix.lane().name(),
                entry.sequence
            ));
            retired.push(
                cleanup::snapshot(
                    storage,
                    &session.meeting_id,
                    &path.to_string_lossy().replace('\\', "/"),
                    cancelled,
                )?
                .0,
            );
        }
        prefix.verify_at(&base_path, cancelled)?;
    }
    if !partial {
        // The admitted closed native manifest owns these two continuous
        // working copies. Their verified raw prefixes already supplied the
        // final WAV, so retire only the positively received fixed-name lanes.
        // Partial/unreceived lanes grant no authority over a leftover CAF.
        for lane in ["microphone", "system"] {
            if !document[lane]["frames"]
                .as_u64()
                .is_some_and(|frames| frames > 0)
                || !prefixes
                    .iter()
                    .any(|prefix| prefix.lane().name() == lane && prefix.frames() > 0)
            {
                continue;
            }
            let leaf = format!("{lane}.caf");
            let snapshot = || -> Result<Option<cleanup::RetiredAudio>, String> {
                let Some(file) = directory.optional(&leaf, false)? else {
                    return Ok(None);
                };
                let stamp = Stamp::read(&file)?;
                let relative = base.join(&leaf).to_string_lossy().replace('\\', "/");
                let (owned, _) =
                    cleanup::snapshot(storage, &session.meeting_id, &relative, cancelled)?;
                stamp.require(&file)?;
                stamp.require(&directory.required(&leaf, false)?)?;
                Ok(Some(owned))
            };
            match snapshot() {
                Ok(Some(owned)) => retired.push(owned),
                Ok(None) => {}
                Err(error) if !cancelled() => crate::internal_logs::detail(
                    crate::internal_logs::current_context("recording"),
                    "legacy_continuous_cleanup_deferred",
                    "warning",
                    std::time::Duration::ZERO,
                    &error,
                ),
                Err(error) => return Err(error),
            }
        }
    }
    let sessions = std::collections::BTreeSet::from([session.id.clone()]);
    if registered_source.is_none() {
        match flac::inspect_workspaces(storage, &session.meeting_id, &sessions, cancelled) {
            Ok(workspaces) => {
                for relative in workspaces.audio_paths() {
                    if cancelled() {
                        return Err("旧録音の音声整理確認を停止しました".into());
                    }
                    let path = storage.managed_path(Path::new(relative))?;
                    let parent = Directory::open(path.parent().ok_or("旧FLACの管理先が不正です")?)?;
                    let name = path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .ok_or("旧FLACのファイル名が不正です")?;
                    let file = parent.required(name, false)?;
                    // Revalidate the native producer's physical stamp before
                    // adding this file to the existing retirement ledger.
                    if workspaces.hint(relative, &file)?.as_deref() != Some("recording_flac") {
                        return Err("旧FLACの所有記録が変更されています".into());
                    }
                    retired.push(
                        cleanup::snapshot(storage, &session.meeting_id, relative, cancelled)?.0,
                    );
                    workspaces.hint(relative, &parent.required(name, false)?)?;
                }
            }
            Err(error) if !cancelled() => {
                // Original prefix verification above still fails normally. A
                // damaged derived plan only leaves its unproven files retained.
                crate::internal_logs::detail(
                    crate::internal_logs::current_context("recording"),
                    "legacy_flac_cleanup_deferred",
                    "warning",
                    std::time::Duration::ZERO,
                    &error,
                );
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(source) = &registered_source {
        let mut candidates = retired.clone();
        match cleanup::inventory(
            storage,
            &session.meeting_id,
            Some(source),
            &pending.path(),
            &mut candidates,
            cancelled,
        ) {
            Ok(()) => retired = candidates,
            Err(error) if !cancelled() => {
                // A damaged former output must not force decoding or
                // rebuilding FLAC before the raw capture can be rescued.
                crate::internal_logs::detail(
                    crate::internal_logs::current_context("recording"),
                    "legacy_registered_cleanup_deferred",
                    "warning",
                    std::time::Duration::ZERO,
                    &error,
                );
                // Retaining unusable bytes must not leave them advertised as
                // ready sources. Only a successful, pinned physical read can
                // prove a size/hash mismatch; an I/O error grants no such fact.
                let mut checked = std::collections::BTreeSet::new();
                for chunk in source.tracks.iter().flat_map(|track| &track.chunks) {
                    if !checked.insert(chunk.asset_id.clone()) {
                        continue;
                    }
                    let inspect = || -> Result<Option<(String, Stamp)>, String> {
                        let relative: String = storage.connection.lock().map_err(|_|"DBロックを取得できません")?.query_row(
                            "SELECT relative_path FROM file_assets WHERE id=?1 AND meeting_id=?2 AND state='ready' AND content_hash=?3 AND size_bytes=?4",
                            params![chunk.asset_id,session.meeting_id,chunk.hash,chunk.size],|row|row.get(0)).map_err(|error|error.to_string())?;
                        let path = storage.managed_path(Path::new(&relative))?;
                        let directory =
                            Directory::open(path.parent().ok_or("旧音声の管理先が不正です")?)?;
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or("旧音声の名前が不正です")?;
                        let stamp = Stamp::read(&directory.required(name, false)?)?;
                        let (actual, _) =
                            cleanup::snapshot(storage, &session.meeting_id, &relative, cancelled)?;
                        stamp.require(&directory.required(name, false)?)?;
                        Ok((u64::try_from(chunk.size).ok() != Some(actual.bytes)
                            || chunk.hash != actual.hash)
                            .then_some((relative, stamp)))
                    };
                    match inspect() {
                        Ok(Some((relative, stamp))) => corrupt_assets.push((
                            chunk.asset_id.clone(),
                            relative,
                            chunk.size,
                            chunk.hash.clone(),
                            stamp,
                        )),
                        Ok(None) => {}
                        Err(error) if !cancelled() => crate::internal_logs::detail(
                            crate::internal_logs::current_context("recording"),
                            "legacy_retained_asset_unverified",
                            "warning",
                            std::time::Duration::ZERO,
                            &error,
                        ),
                        Err(error) => return Err(error),
                    }
                }
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(stamp) = &original_manifest {
        stamp.require(&directory)?;
    }
    // Only time where no input had any captured samples becomes a canonical
    // missing interval; received zero-valued samples remain valid coverage.
    coverage.sort_unstable();
    let mut missing = Vec::new();
    let mut cursor = 0;
    for (start, end) in coverage {
        if cursor < start {
            missing.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < duration_us {
        missing.push((cursor, duration_us));
    }
    if partial {
        let expected = document["partialRecovery"]["reviewHash"]
            .as_str()
            .ok_or("旧録音の部分救出確認がありません")?;
        if storage
            .review_partial_recording_checked(&session.id, cancelled)?
            .summary
            .review_hash
            != expected
        {
            return Err("旧録音の部分救出対象がWAV作成中に変更されました".into());
        }
    }
    if cancelled() {
        return Err("旧録音WAVの登録を停止しました".into());
    }
    for (_, relative, _, _, stamp) in &corrupt_assets {
        let path = storage.managed_path(Path::new(relative))?;
        let directory = Directory::open(path.parent().ok_or("旧音声の管理先が不正です")?)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("旧音声の名前が不正です")?;
        stamp.require(&directory.required(name, false)?)?;
    }
    if let Some(stamp) = &original_manifest {
        stamp.require(&directory)?;
    }
    let mut db = storage
        .connection
        .lock()
        .map_err(|_| "DBロックを取得できません")?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let (current_config,current_state,current_version):(String,String,i64)=tx.query_row(
        "SELECT r.capture_config_json,m.lifecycle,m.lock_version FROM recording_sessions r JOIN meetings m ON m.id=r.meeting_id WHERE r.id=?1 AND r.meeting_id=?2",
        params![session.id,session.meeting_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(|error|error.to_string())?;
    if load_session(&tx, &session.id)?.as_ref() != Some(session)
        || current_config != config
        || current_state != initial_state
        || current_version != initial_version
    {
        return Err("旧録音WAVの作成中に録音・会議状態が変更されました".into());
    }
    if let Some(source) = &registered_source {
        if crate::storage::audio_publication::inputs::read(&tx, &session.meeting_id)? != *source {
            return Err("旧録音の登録済み入力がWAV作成中に変更されました".into());
        }
    }
    for (asset, relative, size, hash, _) in &corrupt_assets {
        if tx.execute("UPDATE file_assets SET state='corrupt' WHERE id=?1 AND meeting_id=?2 AND relative_path=?3 AND size_bytes=?4 AND content_hash=?5 AND state='ready'",
            params![asset,session.meeting_id,relative,size,hash]).map_err(|error|error.to_string())?!=1 {
            return Err("旧録音の破損した入力の登録が変更されています".into());
        }
    }
    let now = now_ms()?;
    let manifest_id =
        insert_capture_asset(&tx, &session.meeting_id, &manifest, "application/json", now)?;
    let audio_id = insert_capture_asset(&tx, &session.meeting_id, &wave, "audio/wav", now)?;
    let track = Uuid::new_v4().to_string();
    let mut track_config = serde_json::json!({"policy":super::super::meeting_audio::POLICY,"legacyMixPolicy":"constant_gain_v1"});
    if partial {
        track_config["partial_recovery"] = document["partialRecovery"].clone();
    }
    tx.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES(?1,?2,?3,'processed','wav',16000,1,'ready',?4,?5,?5)",
        params![track,session.meeting_id,session.id,track_config.to_string(),now]).map_err(|error|error.to_string())?;
    if let Some(source) = &registered_source {
        for input in &source.tracks {
            tx.execute("INSERT INTO track_inputs(meeting_id,derived_track_id,input_track_id,role) VALUES(?1,?2,?3,'meeting_mix')",
                params![session.meeting_id,track,input.id]).map_err(|error|error.to_string())?;
        }
    }
    tx.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,0,0,?5,?6,16000,1,'ready',?7)",
        params![Uuid::new_v4().to_string(),session.meeting_id,track,audio_id,(frames*1_000_000/16000) as i64,frames as i64,now]).map_err(|error|error.to_string())?;
    for (start, end) in missing {
        tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code,recovered) VALUES(?1,?2,?3,?4,'meeting_input_missing',0)",
        params![Uuid::new_v4().to_string(),track,start,end]).map_err(|error|error.to_string())?;
    }
    tx.execute("UPDATE recording_sessions SET state=?2,duration_us=?3,last_committed_us=?3,audio_manifest_hash=?4,started_at_utc_ms=?5,ended_at_utc_ms=?6,updated_at=?7 WHERE id=?1",
        params![session.id,if partial {"interrupted"} else {"saved"},duration_us,manifest.hash,started,if partial {None}else{ended},now]).map_err(|error|error.to_string())?;
    tx.execute("UPDATE meetings SET lifecycle='recorded',duration_us=?2,started_at_utc_ms=?3,lock_version=lock_version+1,updated_at=?4 WHERE id=?1 AND lock_version=?5",
        params![session.meeting_id,duration_us,started,now,initial_version]).map_err(|error|error.to_string())?;
    tx.execute("INSERT INTO meeting_audio_commits(meeting_id,track_id,manifest_asset_id,manifest_hash,frames,source_json,cleanup_json,phase,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'committed',?8)",
        params![session.meeting_id,track,manifest_id,manifest.hash,frames as i64,serde_json::json!({"legacyRecording":session.id,"inputHash":key,"policy":"legacy_capture_constant_gain_v1","partialRecovery":document["partialRecovery"]}).to_string(),serde_json::to_string(&retired).map_err(|error|error.to_string())?,now]).map_err(|error|error.to_string())?;
    super::super::analysis_flows::ensure_flow(&tx, &session.meeting_id)?;
    let saved = load_session(&tx, &session.id)?.ok_or("旧録音WAVの保存結果がありません")?;
    tx.commit().map_err(|error| error.to_string())?;
    drop(db);
    storage.publish_single_wav(&session.meeting_id, cancelled, &|_, _| {})?;
    Ok(saved)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn damaged_legacy_suffix_saves_only_healthy_leading_audio_and_keeps_every_source() {
        for problem in ["receipt", "pcm", "clock"] {
            let fixture = super::super::partial_test_fixture();
            for lane in ["microphone", "system"] {
                let receipt = fixture.base.join(format!("raw-{lane}-00000002.json"));
                match problem {
                    "receipt" => std::fs::write(&receipt, b"{broken").unwrap(),
                    "pcm" => std::fs::write(
                        fixture.base.join(format!("raw-{lane}-00000002.caf")),
                        b"corrupt audio",
                    )
                    .unwrap(),
                    "clock" => {
                        let mut entry: serde_json::Value =
                            serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
                        entry["clockNanoseconds"][0]["endNanoseconds"] = 0.into();
                        std::fs::write(&receipt, serde_json::to_vec(&entry).unwrap()).unwrap();
                    }
                    _ => unreachable!(),
                }
            }
            let originals: Vec<_> = std::fs::read_dir(&fixture.base)
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    let bytes = std::fs::read(&path).unwrap();
                    (path, bytes)
                })
                .collect();
            let reviewed = fixture
                .storage
                .review_partial_recording(&fixture.session.id)
                .unwrap();
            assert!(reviewed.document()["partialRecovery"]["truncated"] == true);
            assert_eq!(reviewed.summary.duration_us, 4_050_000);
            assert!(reviewed
                .summary
                .lanes
                .iter()
                .all(|lane| lane.frames == 32_000));
            let saved = fixture
                .storage
                .recover_partial_recording(&reviewed, true)
                .unwrap();
            assert_eq!(saved.state, "interrupted");
            assert_eq!(saved.ended_at_utc_ms, None);
            assert_eq!(saved.duration_us, 4_050_000);
            let first = fixture
                .storage
                .verify_canonical_audio(&saved.meeting_id, &|| false)
                .unwrap();
            assert_eq!(first.tracks[0].chunks[0].frames, 64_800);
            for (path, bytes) in originals {
                assert_eq!(std::fs::read(path).unwrap(), bytes, "{problem}");
            }
            fixture
                .storage
                .finalize_available_recording(&saved.id, &|| false)
                .unwrap();
            assert_eq!(
                fixture
                    .storage
                    .canonical_audio_plan(&saved.meeting_id)
                    .unwrap(),
                first
            );
        }
    }

    #[test]
    fn restored_legacy_session_cannot_mint_native_prefix_recovery_authority() {
        let fixture = super::super::partial_test_fixture();
        fixture.storage.connection.lock().unwrap().execute(
            "INSERT INTO restore_provenance(id,meeting_id,domain,entity_key,source_archive,source_scope,source_key,restore_batch,restored_at) VALUES(?1,?2,'meetings',?2,'archive','meetings','original-meeting','batch',0)",
            params![Uuid::new_v4().to_string(), fixture.session.meeting_id],
        ).unwrap();
        assert!(fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .err()
            .unwrap()
            .contains("復元した会議"));
        assert!(fixture.base.join("raw-microphone-00000001.caf").exists());
    }

    #[test]
    fn failed_partial_new_source_generation_keeps_previous_pending_wav_unchanged() {
        use std::os::unix::fs::OpenOptionsExt;
        let fixture = super::super::partial_test_fixture();
        let review = fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .unwrap();
        fixture.reject_partial_commit(true);
        assert!(fixture
            .storage
            .recover_partial_recording(&review, true)
            .is_err());
        fixture.reject_partial_commit(false);
        let pending_root = fixture
            .storage
            .root
            .join("meetings")
            .join(&fixture.session.meeting_id)
            .join("audio/meeting-wave");
        let old_directory = std::fs::read_dir(&pending_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let plan: serde_json::Value = serde_json::from_slice(
            &std::fs::read(old_directory.join("wave-publication.json")).unwrap(),
        )
        .unwrap();
        let old_wave = old_directory.join(plan["name"].as_str().unwrap());
        let old_bytes = std::fs::read(&old_wave).unwrap();
        let old_stamp = Stamp::read(&File::open(&old_wave).unwrap()).unwrap();

        // Keep the original owned root. Identical verified bytes at fresh raw
        // inodes constitute a new generation, not a qualified storage move.
        let mut retained = Vec::new();
        for lane in ["microphone", "system"] {
            let tip: serde_json::Value = serde_json::from_slice(
                &std::fs::read(fixture.base.join(format!("raw-{lane}.json"))).unwrap(),
            )
            .unwrap();
            for sequence in 1..=tip["sealedSegments"].as_u64().unwrap() {
                let path = fixture.base.join(format!("raw-{lane}-{sequence:08}.caf"));
                let bytes = std::fs::read(&path).unwrap();
                let previous = fixture
                    .base
                    .join(format!("retained-{lane}-{sequence:08}.caf"));
                std::fs::rename(&path, &previous).unwrap();
                let stamp = Stamp::read(&File::open(&previous).unwrap()).unwrap();
                let mut file = File::options()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .unwrap();
                file.write_all(&bytes).unwrap();
                file.sync_all().unwrap();
                retained.push((previous, bytes, stamp));
            }
        }
        assert!(fixture
            .storage
            .recover_partial_recording(&review, true)
            .is_err());
        let fresh = fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .unwrap();
        assert_ne!(fresh.summary.review_hash, review.summary.review_hash);
        let saved = fixture
            .storage
            .recover_partial_recording(&fresh, true)
            .unwrap();
        let canonical = fixture
            .storage
            .verify_canonical_audio(&saved.meeting_id, &|| false)
            .unwrap();
        let new_wave = fixture
            .storage
            .meeting_mix_path(&saved.meeting_id, &canonical.tracks[0])
            .unwrap();
        assert_ne!(new_wave, old_wave);
        old_stamp.require(&File::open(&old_wave).unwrap()).unwrap();
        assert_eq!(std::fs::read(old_wave).unwrap(), old_bytes);
        assert_eq!(std::fs::read_dir(&pending_root).unwrap().count(), 2);
        for (path, bytes, stamp) in retained {
            stamp.require(&File::open(&path).unwrap()).unwrap();
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn registered_legacy_save_rescues_raw_without_recreating_flac() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        for damaged_derived in [false, true] {
            let fixture = super::super::partial_test_fixture();
            let write = |path: &Path, value: &serde_json::Value| {
                let mut file = File::create(path).unwrap();
                file.write_all(&serde_json::to_vec(value).unwrap()).unwrap();
                file.sync_all().unwrap();
            };
            let manifest_path = fixture.base.join("manifest.json");
            let mut document: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
            document["state"] = serde_json::json!("saved");
            document["captureEnd"] = serde_json::json!({"method":"acquisition_closed",
                "microphoneClosedUs":5_450_000,"systemClosedUs":5_600_000,"endUs":5_600_000});
            write(&manifest_path, &document);
            for lane in ["microphone", "system"] {
                let path = fixture.base.join(format!("raw-{lane}.json"));
                let mut tip: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                tip["producerClosed"] = serde_json::json!(true);
                write(&path, &tip);
            }
            let base = fixture.base.strip_prefix(&fixture.storage.root).unwrap();
            let manifest_file = fixture
                .storage
                .checked_capture_file(&base.join("manifest.json"), None)
                .unwrap();
            let manifest = fixture
                .storage
                .read_capture_manifest(&manifest_file)
                .unwrap();
            let prefixes = fixture
                .storage
                .verify_completed_raw_prefixes(base, &manifest, &fixture.session.id)
                .unwrap();
            // Reproduce the older application's committed input rows. FLAC is
            // generated only by this fixture; the migration must use raw PCM.
            let publication = flac::Publication::prepare_completed(
                &fixture.storage,
                base,
                &manifest_file.hash,
                9_000_000_000_000,
                &prefixes,
                &|| false,
            )
            .unwrap();
            {
                let mut db = fixture.storage.connection.lock().unwrap();
                let tx = db.transaction().unwrap();
                insert_capture_asset(
                    &tx,
                    &fixture.session.meeting_id,
                    &manifest_file,
                    "application/json",
                    0,
                )
                .unwrap();
                publication
                    .insert_completed(
                        &tx,
                        &fixture.session.meeting_id,
                        &fixture.session.id,
                        "{\"timeline_alignment\":\"unverified\"}",
                        5_600_000,
                        0,
                    )
                    .unwrap();
                tx.execute("UPDATE recording_sessions SET state='saved',duration_us=5600000,last_committed_us=5600000,audio_manifest_hash=?2,started_at_utc_ms=1700000000000,ended_at_utc_ms=1700000005600 WHERE id=?1",
                    params![fixture.session.id,manifest_file.hash]).unwrap();
                tx.execute("UPDATE meetings SET lifecycle='recorded',duration_us=5600000,started_at_utc_ms=1700000000000 WHERE id=?1",
                    [&fixture.session.meeting_id]).unwrap();
                tx.commit().unwrap();
            }
            drop(publication);
            let derived: Vec<String> = {
                let db = fixture.storage.connection.lock().unwrap();
                let mut query = db.prepare("SELECT relative_path FROM file_assets WHERE meeting_id=?1 AND media_type='audio/flac'").unwrap();
                let rows = query
                    .query_map([&fixture.session.meeting_id], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                rows
            };
            assert!(!derived.is_empty());
            if damaged_derived {
                File::create(fixture.storage.root.join(&derived[0]))
                    .unwrap()
                    .write_all(b"unusable previous derived output")
                    .unwrap();
            }
            let session = load_session(
                &fixture.storage.connection.lock().unwrap(),
                &fixture.session.id,
            )
            .unwrap()
            .unwrap();
            std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o644))
                .unwrap();
            let directory = Directory::open(&fixture.base).unwrap();
            let (original_manifest, manifest_stamp) =
                directory.read_legacy_native_manifest().unwrap().unwrap();
            let saved = migrate_registered(&fixture.storage, &session, &|| false)
                .unwrap()
                .unwrap();
            assert_eq!(saved.state, "saved");
            assert_eq!(saved.ended_at_utc_ms, Some(1_700_000_005_600));
            assert_eq!(std::fs::read(&manifest_path).unwrap(), original_manifest);
            assert_eq!(
                std::fs::metadata(&manifest_path).unwrap().mode() & 0o777,
                0o644
            );
            manifest_stamp.require(&directory).unwrap();
            let published: String = fixture.storage.connection.lock().unwrap().query_row(
                "SELECT f.relative_path FROM meeting_audio_commits a JOIN file_assets f ON f.id=a.manifest_asset_id WHERE a.meeting_id=?1",
                [&saved.meeting_id],|row|row.get(0)).unwrap();
            assert!(fixture.storage.root.join(published).exists());
            let canonical = fixture
                .storage
                .verify_canonical_audio(&saved.meeting_id, &|| false)
                .unwrap();
            assert_eq!(canonical.tracks.len(), 1);
            assert_eq!(canonical.tracks[0].chunks.len(), 1);
            for (index, relative) in derived.into_iter().enumerate() {
                assert_eq!(
                    fixture.storage.root.join(&relative).exists(),
                    damaged_derived
                );
                let state: String = fixture
                    .storage
                    .connection
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT state FROM file_assets WHERE meeting_id=?1 AND relative_path=?2",
                        params![saved.meeting_id, relative],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    state,
                    if damaged_derived && index == 0 {
                        "corrupt"
                    } else if damaged_derived {
                        "ready"
                    } else {
                        "missing"
                    }
                );
            }
            for lane in ["microphone", "system"] {
                assert!(!fixture.base.join(format!("{lane}.caf")).exists());
            }
            #[cfg(target_os = "macos")]
            drop(
                fixture
                    .storage
                    .prepare_relocation_snapshot(&|| false)
                    .unwrap(),
            );
            assert!(!fixture.base.join("raw-microphone-00000001.caf").exists());
            assert!(migrate_registered(&fixture.storage, &saved, &|| false)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn partial_legacy_prefix_goes_directly_to_wav_with_old_gain_and_keeps_originals() {
        let fixture = super::super::partial_test_fixture();
        let reviewed = fixture
            .storage
            .review_partial_recording(&fixture.session.id)
            .unwrap();
        let saved = fixture
            .storage
            .recover_partial_recording(&reviewed, true)
            .unwrap();
        assert_eq!(saved.state, "interrupted");
        assert_eq!(saved.ended_at_utc_ms, None);
        let canonical = fixture
            .storage
            .verify_canonical_audio(&fixture.session.meeting_id, &|| false)
            .unwrap();
        assert_eq!(canonical.tracks.len(), 1);
        assert_eq!(canonical.tracks[0].chunks.len(), 1);
        let chunk = &canonical.tracks[0].chunks[0];
        let relative: String = fixture
            .storage
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT relative_path FROM file_assets WHERE id=?1",
                [&chunk.asset_id],
                |row| row.get(0),
            )
            .unwrap();
        let mut file = File::open(fixture.storage.root.join(relative)).unwrap();
        let layout = crate::audio::wave::inspect(&mut file, chunk.size as u64).unwrap();
        let mut sample = [0; 2];
        crate::native_fs::read_exact_at(&file, &mut sample, layout.data_offset + 200 * 2).unwrap();
        // PC audio starts 50ms later. Legacy constant headroom still halves
        // this initial mic-only interval; the new live gain policy must not
        // retroactively alter a rescued old recording.
        assert_eq!(i16::from_le_bytes(sample), -2048);
        crate::native_fs::read_exact_at(&file, &mut sample, layout.data_offset + 1600 * 2).unwrap();
        assert_eq!(i16::from_le_bytes(sample), -4096);
        assert!(!fixture.base.join("partial-flac.json").exists());
        assert!(!fixture.base.join(".final-flac.lock").exists());
        for lane in ["microphone", "system"] {
            assert!(fixture
                .base
                .join(format!("raw-{lane}-00000001.caf"))
                .exists());
            assert!(fixture
                .base
                .join(format!("raw-{lane}-00000002.caf"))
                .exists());
            // Rescue retains both adopted segments and the malformed,
            // unadvertised tail, together with the continuous working file.
            assert!(fixture
                .base
                .join(format!("raw-{lane}-00000003.caf"))
                .exists());
            assert!(fixture.base.join(format!("{lane}.caf")).exists());
        }
        assert!(fixture
            .storage
            .recording_save_complete(&saved.id, &saved.meeting_id, &|| false)
            .unwrap());
    }
}
