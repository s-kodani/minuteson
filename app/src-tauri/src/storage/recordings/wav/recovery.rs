//! Closed-producer inspection. Audio authority is independent of optional
//! capture diagnostics; a broken byte-range proof ends the recoverable prefix.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Advertised {
    schema_version: u32,
    owner_id: String,
    file_name: String,
    checkpoints: u32,
    #[serde(rename = "lastEntrySHA256")]
    last_entry_sha256: Option<String>,
    #[serde(default)]
    frames: serde_json::Value,
    #[serde(default)]
    capture_origin_nanoseconds: serde_json::Value,
    #[serde(default)]
    producer_closed: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioReceipt {
    schema_version: u32,
    owner_id: String,
    sequence: u32,
    frame_start: u64,
    frames: u64,
    #[serde(rename = "pcmSHA256")]
    pcm_sha256: String,
    #[serde(rename = "previousEntrySHA256")]
    previous_entry_sha256: Option<String>,
    #[serde(default)]
    capture_origin_nanoseconds: serde_json::Value,
    #[serde(default)]
    missing_ranges: serde_json::Value,
    #[serde(default)]
    coverage_frames: serde_json::Value,
}

pub(super) struct Verified {
    pub file: File,
    pub stamp: Stamp,
    pub tip: Tip,
    pub original_closed: bool,
    pub truncated: bool,
    pub missing: Vec<[u64; 2]>,
    pub diagnostics_known: bool,
    pub pcm_hash: String,
    pub full_hash: Option<String>,
    pub fault: Option<String>,
}

pub(super) fn inspect(
    storage: &Storage,
    directory: &Directory,
    owner: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Verified, String> {
    let (tip_bytes, tip_stamp) = directory
        .read_json("single-wav.json")?
        .ok_or("録音WAVの確定範囲がありません")?;
    let advertised: Advertised =
        serde_json::from_value(crate::capture_manifest::decode_unique(&tip_bytes)?)
            .map_err(|error| error.to_string())?;
    if advertised.schema_version != 1
        || advertised.owner_id != owner
        || advertised.file_name != mon_core::capture::wav::AUDIO_NAME
        || advertised.checkpoints > mon_core::capture::wav::MAX_CHECKPOINTS
        || (advertised.checkpoints == 0 && advertised.last_entry_sha256.is_some())
        || (advertised.checkpoints > 0
            && !advertised
                .last_entry_sha256
                .as_deref()
                .is_some_and(hash_valid))
    {
        return Err("録音WAVの確定範囲・所属が不正です".into());
    }
    let (reservation_bytes, reservation_stamp) = directory
        .read_json("single-wav-owner.json")?
        .ok_or("録音WAVの所有予約がありません")?;
    let mut reservation: Reservation =
        serde_json::from_slice(&reservation_bytes).map_err(|error| error.to_string())?;
    if reservation.schema_version != 1
        || reservation.owner_id != owner
        || reservation.file_name != advertised.file_name
        || reservation.birth_nanoseconds < 0
    {
        return Err("録音WAVの所有予約が不正です".into());
    }
    #[cfg(any(target_os = "macos", windows))]
    if let Some(aliases) = &storage.wav_history {
        if let Some((device, inode, birth)) = aliases.effective_identity(
            owner,
            &mon_core::hash::sha256_hex(&reservation_bytes),
            (
                reservation.device,
                reservation.inode,
                reservation.birth_nanoseconds,
            ),
        )? {
            reservation.device = device;
            reservation.inode = inode;
            reservation.birth_nanoseconds = birth;
        }
    }
    let file = directory.required(&advertised.file_name, false)?;
    identity_matches(&file, &reservation)?;
    let stamp = Stamp::read(&file)?;
    crate::operation_observation::file_check();
    let claimed_frames = advertised
        .frames
        .as_u64()
        .filter(|frames| *frames <= 9_007_199_254_740_991);
    let mut full_hash = Sha256::new();
    full_hash.update(crate::audio::wave::streaming_header(
        claimed_frames.unwrap_or(0),
    )?);
    let mut pcm_hash = Sha256::new();
    let mut frames = 0_u64;
    let mut checkpoints = 0;
    let mut chain = None;
    let mut origin = None;
    let mut origin_known = true;
    let mut missing = Vec::new();
    let mut diagnostics_known = true;
    let mut metadata_bytes = 0_u64;
    let mut fault = None;
    for sequence in 1..=advertised.checkpoints {
        if cancelled() {
            return Err("録音WAVの照合を停止しました".into());
        }
        let checked = (|| -> Result<_, String> {
            let name = format!("wav-{sequence:08}.json");
            let (bytes, receipt_stamp) = directory
                .read_json(&name)?
                .ok_or("録音WAVの確定記録が不足しています")?;
            metadata_bytes = metadata_bytes
                .checked_add(bytes.len() as u64)
                .ok_or("録音WAVの確定記録が大きすぎます")?;
            if bytes.len() as u64 > mon_core::capture::limits::MAX_ENTRY_BYTES
                || metadata_bytes > mon_core::capture::limits::MAX_METADATA_BYTES
            {
                return Err("録音WAVの確定記録が上限を超えています".into());
            }
            let receipt: AudioReceipt =
                serde_json::from_value(crate::capture_manifest::decode_unique(&bytes)?)
                    .map_err(|error| error.to_string())?;
            let end = frames
                .checked_add(receipt.frames)
                .filter(|end| *end <= 9_007_199_254_740_991)
                .ok_or("録音WAVの範囲が大きすぎます")?;
            let next_hash = mon_core::hash::sha256_hex(&bytes);
            if receipt.schema_version != 1
                || receipt.owner_id != owner
                || receipt.sequence != sequence
                || receipt.frame_start != frames
                || receipt.frames == 0
                || receipt.frames > mon_core::capture::wav::MAX_CHECKPOINT_FRAMES
                || receipt.previous_entry_sha256 != chain
                || !hash_valid(&receipt.pcm_sha256)
                || (sequence == advertised.checkpoints
                    && Some(&next_hash) != advertised.last_entry_sha256.as_ref())
            {
                return Err("録音WAVの確定記録と音声が一致しません".into());
            }
            let mut pcm = vec![0; receipt.frames as usize * 2];
            crate::native_fs::read_exact_at(&file, &mut pcm, 80 + frames * 2)
                .map_err(|error| error.to_string())?;
            if mon_core::hash::sha256_hex(&pcm) != receipt.pcm_sha256 {
                return Err("録音WAVの確定PCMが変更されています".into());
            }
            crate::operation_observation::read(pcm.len());
            receipt_stamp.require(&directory.required(&name, false)?)?;
            Ok((receipt, pcm, end, next_hash))
        })();
        let (receipt, pcm, end, next_hash) = match checked {
            Ok(value) => value,
            Err(error) => {
                fault = Some(error);
                break;
            }
        };
        let receipt_origin = receipt
            .capture_origin_nanoseconds
            .as_i64()
            .filter(|value| *value >= 0);
        if receipt_origin.is_none() || origin.is_some_and(|old| Some(old) != receipt_origin) {
            origin_known = false;
        }
        if origin.is_none() {
            origin = receipt_origin;
        }
        let ranges = serde_json::from_value::<Vec<[u64; 2]>>(receipt.missing_ranges).ok();
        let mut previous_end = frames;
        if let Some(ranges) = ranges.filter(|ranges| ranges.len() <= 8192) {
            if ranges.iter().all(|[start, stop]| {
                let valid = *start >= previous_end && *stop > *start && *stop <= end;
                previous_end = *stop;
                valid
            }) {
                missing.extend(ranges);
            } else {
                diagnostics_known = false;
            }
        } else {
            diagnostics_known = false;
        }
        if !serde_json::from_value::<[u64; 2]>(receipt.coverage_frames)
            .is_ok_and(|counts| counts.iter().all(|count| *count <= receipt.frames))
        {
            diagnostics_known = false;
        }
        pcm_hash.update(&pcm);
        full_hash.update(&pcm);
        frames = end;
        checkpoints = sequence;
        chain = Some(next_hash);
    }
    // Reading metadata must never authorize bytes that changed underneath us.
    stamp.require(&directory.required(&advertised.file_name, false)?)?;
    reservation_stamp.require(&directory.required("single-wav-owner.json", false)?)?;
    tip_stamp.require(&directory.required("single-wav.json", false)?)?;
    if frames == 0 {
        return Err(
            if advertised.checkpoints == 0 && claimed_frames == Some(0) {
                NO_CAPTURED_AUDIO.into()
            } else {
                fault.unwrap_or_else(|| "検証できる録音音声がありません".into())
            },
        );
    }
    let truncated = checkpoints < advertised.checkpoints;
    let origin = if origin_known && advertised.capture_origin_nanoseconds.as_i64() == origin {
        origin
    } else {
        None
    };
    let original_closed = advertised.producer_closed == true
        && !truncated
        && claimed_frames == Some(frames)
        && origin.is_some();
    Ok(Verified {
        file,
        stamp,
        tip: Tip {
            schema_version: 1,
            owner_id: owner.into(),
            file_name: advertised.file_name,
            frames,
            checkpoints,
            last_entry_sha256: chain,
            capture_origin_nanoseconds: origin,
            producer_closed: original_closed,
        },
        original_closed,
        truncated,
        missing,
        diagnostics_known: diagnostics_known && origin.is_some() && claimed_frames == Some(frames),
        pcm_hash: mon_core::hash::sha256_output_hex(pcm_hash.finalize()),
        full_hash: (claimed_frames == Some(frames)).then(|| mon_core::hash::sha256_output_hex(full_hash.finalize())),
        fault,
    })
}

/// Missing/invalid descriptive metadata does not invalidate independently
/// owned PCM. Unsafe file types/ownership still fail at Directory::optional.
pub(super) fn document(
    directory: &Directory,
    name: &str,
) -> Result<Option<(Option<serde_json::Value>, Stamp, Vec<u8>)>, String> {
    let Some(mut file) = directory.optional(name, false)? else {
        return Ok(None);
    };
    let stamp = Stamp::read(&file)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(crate::capture_manifest::MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let value = crate::capture_manifest::decode_unique(&bytes)
        .ok()
        .filter(|value| value.is_object());
    stamp.require(&directory.required(name, false)?)?;
    Ok(Some((value, stamp, bytes)))
}
