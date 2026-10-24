//! Normal native-stop FLAC publication. Durable plans bind original evidence,
//! output UUIDs and staging inodes; a DB rollback never removes raw audio.
use super::*;
use crate::audio::flac::{encode_segment_file, PcmSource, SavedFlac};
use crate::audio::VerifiedAudio;
use crate::audio_clock::ClockSegment;
#[cfg(windows)]
use crate::bootstrap::windows_io as libc;
#[cfg(windows)]
use crate::bootstrap::windows_io::{AsRawFd, FromRawFd};
use crate::recording_segments::{FlacGroup, Prefix};
use std::io::Seek;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

mod files;
mod inventory;
#[cfg(any(target_os = "macos", windows))]
pub(in crate::storage) mod relocation;
use files::{Directory, Identity, Stamp};
pub(in crate::storage) use inventory::{inspect_workspaces, WorkspaceFiles};
#[cfg(all(test, unix))]
mod dense_tests;
#[cfg(all(test, unix))]
pub(super) mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    input_hash: String,
    recording_directory: Identity,
    tracks: Vec<TrackPlan>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackPlan {
    id: String,
    directory: Identity,
    chunks: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioProof {
    size: u64,
    sha256: String,
    frames: u64,
    sample_rate: u32,
    channels: u32,
}
impl From<SavedFlac> for AudioProof {
    fn from(saved: SavedFlac) -> Self {
        Self {
            size: saved.size,
            sha256: saved.sha256,
            frames: saved.frames,
            sample_rate: saved.sample_rate,
            channels: saved.channels,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema_version: u32,
    input_hash: String,
    chunk_id: String,
    staging_name: String,
    staging_identity: Option<Identity>,
    audio: Option<AudioProof>,
}

struct Chunk {
    id: String,
    file: CapturedFile,
    audio: AudioProof,
    maps: Vec<ClockSegment>,
    stamp: Stamp,
    receipt: Stamp,
}
struct Track {
    id: String,
    kind: &'static str,
    directory: Directory,
    chunks: Vec<Chunk>,
}
pub(super) struct Publication {
    root: Directory,
    lock: File,
    lock_stamp: Stamp,
    plan_stamp: Stamp,
    plan_name: &'static str,
    documents: Vec<(String, Stamp)>,
    tracks: Vec<Track>,
    #[cfg(any(target_os = "macos", windows))]
    recovery_rebind: Option<relocation::RecoveryRebind>,
}
const PLAN: &str = "final-flac.json";
const PARTIAL_PLAN: &str = "partial-flac.json";
const LOCK: &str = ".final-flac.lock";

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn uuid(value: &str) -> bool {
    crate::input_levels::valid_owner(value)
}
fn groups(prefix: &Prefix, origin: i64, track: &CaptureTrack) -> Result<Vec<FlacGroup>, String> {
    let groups = prefix.flac_groups(origin)?;
    if groups.is_empty() || groups.len() > 1024 {
        return Err("最終FLAC区間数が対応範囲外です".into());
    }
    let mut maps = Vec::new();
    let mut frames = 0;
    for group in &groups {
        for map in &group.clock_segments {
            let mut map = map.clone();
            map.source_frame_start += frames;
            map.source_frame_end += frames;
            maps.push(map);
        }
        frames += group.frames as i64;
    }
    let global = track.validated_timeline(true)?;
    verify_maps(&maps, global.segments, track.sample_rate.unwrap() as u32)?;
    if frames != track.frames {
        return Err("原音声のフレーム数と録音manifestが一致しません".into());
    }
    Ok(groups)
}

/// Both compressors describe the same physical source frames. Keep every
/// gap exactly; permit only the combined, specified one-frame compression
/// error of their independent affine fits, never nominal-rate replacement.
fn verify_maps(raw: &[ClockSegment], global: &[ClockSegment], rate: u32) -> Result<(), String> {
    fn gaps(maps: &[ClockSegment]) -> Vec<(i64, i64, i64)> {
        maps.windows(2)
            .filter_map(|pair| {
                (pair[0].timeline_end_us < pair[1].timeline_start_us).then_some((
                    pair[0].source_frame_end,
                    pair[0].timeline_end_us,
                    pair[1].timeline_start_us,
                ))
            })
            .collect()
    }
    if raw.is_empty()
        || global.is_empty()
        || raw[0].timeline_start_us != global[0].timeline_start_us
        || raw.last().unwrap().timeline_end_us != global.last().unwrap().timeline_end_us
        || raw.last().unwrap().source_frame_end != global.last().unwrap().source_frame_end
        || gaps(raw) != gaps(global)
    {
        return Err("原音声と録音全体の取得時刻・欠損が一致しません".into());
    }
    fn at(map: &ClockSegment, frame: i64) -> (i128, i128) {
        let length = i128::from(map.source_frame_end - map.source_frame_start);
        (
            i128::from(map.timeline_start_us) * length
                + i128::from(frame - map.source_frame_start)
                    * i128::from(map.timeline_end_us - map.timeline_start_us),
            length,
        )
    }
    let tolerance = i128::from(2_000_000_u32.div_ceil(rate) + 2);
    let (mut r, mut g) = (0, 0);
    while r < raw.len() && g < global.len() {
        let first = raw[r].source_frame_start.max(global[g].source_frame_start);
        let last = raw[r].source_frame_end.min(global[g].source_frame_end);
        if last <= first {
            return Err("原音声と録音全体のフレーム対応が不正です".into());
        }
        for frame in [first, last] {
            let (a, ad) = at(&raw[r], frame);
            let (b, bd) = at(&global[g], frame);
            if (a * bd - b * ad).abs() > tolerance * ad * bd {
                return Err("原音声と録音全体の時計変換が一致しません".into());
            }
        }
        if raw[r].source_frame_end == last {
            r += 1;
        }
        if global[g].source_frame_end == last {
            g += 1;
        }
    }
    if r != raw.len() || g != global.len() {
        return Err("原音声の時計対応が録音全体を覆っていません".into());
    }
    Ok(())
}

impl Publication {
    pub(super) fn prepare(
        storage: &Storage,
        base: &Path,
        manifest_hash: &str,
        manifest: &CaptureManifest,
        prefixes: &[Prefix],
    ) -> Result<Self, String> {
        if prefixes.len() != 2 {
            return Err("原音声の確定証拠が両入力で揃っていません".into());
        }
        let audio_prefixes: Vec<_> = prefixes
            .iter()
            .filter(|prefix| prefix.frames() > 0)
            .collect();
        if audio_prefixes.len() != 2
            && !(manifest.system_audio_unreceived
                && audio_prefixes.len() == 1
                && audio_prefixes[0].lane() == crate::recording_segments::Lane::Microphone)
            && !(cfg!(windows)
                && manifest.microphone_audio_unreceived
                && audio_prefixes.len() == 1
                && audio_prefixes[0].lane() == crate::recording_segments::Lane::System)
        {
            return Err("原音声の未取得範囲と録音manifestが一致しません".into());
        }
        let origin = audio_prefixes
            .iter()
            .map(|prefix| prefix.first_capture_nanoseconds())
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min()
            .unwrap();
        let all_groups = audio_prefixes
            .iter()
            .map(|prefix| {
                groups(
                    prefix,
                    origin,
                    match prefix.lane() {
                        crate::recording_segments::Lane::Microphone => &manifest.microphone,
                        crate::recording_segments::Lane::System => &manifest.system,
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let descriptions: Vec<_> = all_groups
            .iter()
            .map(|groups| {
                groups
                    .iter()
                    .map(|group| {
                        serde_json::json!([
                            group.first_entry,
                            group.entries,
                            group.frames,
                            group.clock_segments
                        ])
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let input_hash = hash(
            &serde_json::to_vec(&serde_json::json!({"version":1,
            "manifest":manifest_hash, "raw":prefixes.iter().map(Prefix::tip_hash).collect::<Vec<_>>(),
            "origin":origin, "groups":descriptions}))
            .map_err(|e| e.to_string())?,
        );
        Self::prepare_groups(
            storage,
            base,
            PLAN,
            input_hash,
            &audio_prefixes,
            all_groups,
            &|| false,
            #[cfg(any(target_os = "macos", windows))]
            None,
        )
    }

    pub(super) fn prepare_recovery(
        storage: &Storage,
        base: &Path,
        review_hash: &str,
        content_hash: &str,
        document: &serde_json::Value,
        origin: i64,
        prefixes: &[Prefix],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        partial::check_cancel(cancelled)?;
        if prefixes.is_empty() || prefixes.len() > 2 {
            return Err("部分救出の原音声がありません".into());
        }
        let all_groups = prefixes
            .iter()
            .map(|prefix| prefix.flac_groups(origin))
            .collect::<Result<Vec<_>, _>>()?;
        if all_groups
            .iter()
            .any(|groups| groups.is_empty() || groups.len() > 1024)
        {
            return Err("部分救出のFLAC区間数が対応範囲外です".into());
        }
        for groups in &all_groups {
            let maps: usize = groups.iter().map(|group| group.clock_segments.len()).sum();
            let bytes: usize = groups
                .iter()
                .flat_map(|group| &group.clock_segments)
                .map(|map| map.mapping_json.len())
                .sum();
            if maps > crate::audio_clock::MAX_CLOCK_SEGMENTS
                || bytes > crate::audio_clock::MAX_CLOCK_MAPPING_BYTES
            {
                return Err("部分救出の時刻対応が保存・読戻しの容量上限を超えています".into());
            }
        }
        let input_hash = partial::input_hash(review_hash, origin, prefixes)?;
        #[cfg(any(target_os = "macos", windows))]
        let recovery_rebind = relocation::recovery_input(storage, base, content_hash)?;
        #[cfg(any(target_os = "macos", windows))]
        if let Some(proof) = &recovery_rebind {
            proof.verify_document_before_conversion(
                &Directory::open(&storage.root.join(base))?,
                document,
            )?;
        }
        #[cfg(any(target_os = "macos", windows))]
        let input_hash = recovery_rebind
            .as_ref()
            .map(|p| p.input_hash().to_owned())
            .unwrap_or(input_hash);
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = (content_hash, document);
        Self::prepare_groups(
            storage,
            base,
            PARTIAL_PLAN,
            input_hash,
            &prefixes.iter().collect::<Vec<_>>(),
            all_groups,
            cancelled,
            #[cfg(any(target_os = "macos", windows))]
            recovery_rebind,
        )
    }

    fn prepare_groups(
        storage: &Storage,
        base: &Path,
        plan_name: &'static str,
        input_hash: String,
        prefixes: &[&Prefix],
        all_groups: Vec<Vec<FlacGroup>>,
        cancelled: &impl Fn() -> bool,
        #[cfg(any(target_os = "macos", windows))] recovery_rebind: Option<
            relocation::RecoveryRebind,
        >,
    ) -> Result<Self, String> {
        partial::check_cancel(cancelled)?;
        storage.require_data_writable()?;
        storage.managed_anchor.verify()?;
        let root = Directory::open(&storage.root.join(base))?;
        let lock = match root.optional(LOCK, true)? {
            Some(file) => file,
            None => root.create(LOCK)?,
        };
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("この録音のFLAC確定は別の処理で実行中です".into());
        }
        let lock_stamp = Stamp::read(&lock)?;
        lock_stamp.require(&root.required(LOCK, false)?)?;
        if root.read_json(plan_name)?.is_none() {
            let parent = Directory::open(root.path.parent().ok_or("録音管理先がありません")?)?;
            let mut tracks = Vec::new();
            for groups in &all_groups {
                let id = Uuid::new_v4().to_string();
                let directory = parent.create_child(&id)?;
                tracks.push(TrackPlan {
                    id,
                    directory: directory.identity,
                    chunks: groups.iter().map(|_| Uuid::new_v4().to_string()).collect(),
                });
            }
            let plan = Plan {
                schema_version: 1,
                input_hash: input_hash.clone(),
                recording_directory: root.identity.clone(),
                tracks,
            };
            root.json(plan_name, &plan, None)?;
        }
        // Value and stamp must describe one exact immutable generation.
        let (plan_bytes, plan_stamp) = root
            .read_json(plan_name)?
            .ok_or("FLAC確定計画がありません")?;
        #[cfg(any(target_os = "macos", windows))]
        if let Some(proof) = &recovery_rebind {
            proof.verify_plan(&plan_bytes)?;
        }
        let mut plan: Plan = serde_json::from_slice(&plan_bytes).map_err(|e| e.to_string())?;
        #[cfg(any(target_os = "macos", windows))]
        relocation::effective_plan(storage, &root.path, plan_name, &plan_bytes, &mut plan)?;
        if plan.schema_version != 1
            || plan.input_hash != input_hash
            || plan.recording_directory != root.identity
            || plan.tracks.len() != prefixes.len()
        {
            return Err("FLAC確定計画と原音声・録音manifestが一致しません".into());
        }
        let mut tracks = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for (lane, track_plan) in plan.tracks.into_iter().enumerate() {
            if !uuid(&track_plan.id)
                || !ids.insert(track_plan.id.clone())
                || track_plan.chunks.len() != all_groups[lane].len()
            {
                return Err("FLAC確定計画の識別子・区間数が不正です".into());
            }
            let directory = Directory::open(&root.path.parent().unwrap().join(&track_plan.id))?;
            if directory.identity != track_plan.directory {
                return Err("FLAC保存先の実体が変更されています".into());
            }
            let mut chunks = Vec::new();
            for (id, group) in track_plan.chunks.into_iter().zip(&all_groups[lane]) {
                if !uuid(&id) || !ids.insert(id.clone()) {
                    return Err("FLAC区間の識別子が不正です".into());
                }
                storage.require_data_writable()?;
                storage.managed_anchor.verify()?;
                root.verify()?;
                lock_stamp.require(&root.required(LOCK, false)?)?;
                plan_stamp.require(&root.required(plan_name, false)?)?;
                prefixes[lane].verify_at(&root.path, cancelled)?;
                chunks.push(convert(
                    storage,
                    &directory,
                    &prefixes[lane],
                    group,
                    &input_hash,
                    id,
                    cancelled,
                )?);
            }
            tracks.push(Track {
                id: track_plan.id,
                kind: match prefixes[lane].lane() {
                    crate::recording_segments::Lane::Microphone => "mic",
                    crate::recording_segments::Lane::System => "system",
                },
                directory,
                chunks,
            });
        }
        let result = Self {
            root,
            lock,
            lock_stamp,
            plan_stamp,
            plan_name,
            documents: Vec::new(),
            tracks,
            #[cfg(any(target_os = "macos", windows))]
            recovery_rebind,
        };
        result.verify(storage)?;
        Ok(result)
    }

    pub(super) fn recovery_document(
        &mut self,
        storage: &Storage,
        value: &serde_json::Value,
    ) -> Result<CapturedFile, String> {
        self.verify(storage)?;
        let name = "partial-recovery.json";
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if self.root.read_json(name)?.is_none() {
            self.root.json(name, value, None)?;
        }
        let (current, mut stamp) = self
            .root
            .read_json(name)?
            .ok_or("部分救出の記録がありません")?;
        if current != bytes {
            #[cfg(any(target_os = "macos", windows))]
            if let Some(proof) = &self.recovery_rebind {
                proof.require_document(&self.root, &current, value)?;
                self.root.json(name, value, Some(&stamp))?;
                #[cfg(test)]
                crash_point("partial-relocation-document");
                let (saved, saved_stamp) = self
                    .root
                    .read_json(name)?
                    .ok_or("部分救出の移動後記録がありません")?;
                if saved != bytes {
                    return Err("部分救出の移動後記録が一致しません".into());
                }
                stamp = saved_stamp;
            } else {
                return Err("部分救出の記録と確認済み範囲が一致しません".into());
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            return Err("部分救出の記録と確認済み範囲が一致しません".into());
        }
        self.documents.push((name.into(), stamp));
        Ok(CapturedFile {
            relative: self
                .root
                .path
                .join(name)
                .strip_prefix(&storage.root)
                .map_err(|e| e.to_string())?
                .to_owned(),
            size_bytes: bytes.len() as i64,
            hash: hash(&bytes),
        })
    }
    pub(super) fn verify(&self, storage: &Storage) -> Result<(), String> {
        storage.require_data_writable()?;
        storage.managed_anchor.verify()?;
        self.root.verify()?;
        self.lock_stamp.require(&self.lock)?;
        self.lock_stamp.require(&self.root.required(LOCK, false)?)?;
        self.plan_stamp
            .require(&self.root.required(self.plan_name, false)?)?;
        for (name, stamp) in &self.documents {
            stamp.require(&self.root.required(name, false)?)?;
        }
        for track in &self.tracks {
            track.directory.verify()?;
            for chunk in &track.chunks {
                chunk.stamp.require(
                    &track
                        .directory
                        .required(&format!("{}.flac", chunk.id), false)?,
                )?;
                chunk.receipt.require(
                    &track
                        .directory
                        .required(&format!(".{}.json", chunk.id), false)?,
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn insert(
        &self,
        tx: &rusqlite::Transaction<'_>,
        meeting: &str,
        session: &str,
        config: &str,
        duration: i64,
        boundary_gaps: bool,
        now: i64,
    ) -> Result<(), String> {
        for track in &self.tracks {
            let audio = &track.chunks[0].audio;
            tx.execute("INSERT INTO audio_tracks(id,meeting_id,session_id,kind,format,sample_rate,channels,state,processing_config_json,created_at,updated_at) VALUES(?1,?2,?3,?4,'flac',?5,?6,'ready',?7,?8,?8)",
                params![track.id, meeting, session, track.kind, audio.sample_rate, audio.channels, config, now]).map_err(|e| e.to_string())?;
            let mut previous = None;
            for (sequence, chunk) in track.chunks.iter().enumerate() {
                let asset = insert_capture_asset(tx, meeting, &chunk.file, "audio/flac", now)?;
                let first = chunk.maps[0].timeline_start_us;
                let last = chunk.maps.last().unwrap().timeline_end_us;
                tx.execute("INSERT INTO audio_chunks(id,meeting_id,track_id,file_asset_id,sequence,start_us,end_us,frame_count,sample_rate,channels,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'ready',?11)",
                    params![chunk.id, meeting, track.id, asset, sequence as i64, first, last,
                        chunk.audio.frames as i64, chunk.audio.sample_rate, chunk.audio.channels, now]).map_err(|e| e.to_string())?;
                for map in &chunk.maps {
                    if let Some(end) = previous.filter(|end| *end < map.timeline_start_us) {
                        tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code) VALUES(?1,?2,?3,?4,'capture_timestamp_gap')",
                            params![Uuid::new_v4().to_string(), track.id, end, map.timeline_start_us]).map_err(|e| e.to_string())?;
                    }
                    tx.execute("INSERT INTO track_clock_segments(id,track_id,chunk_id,source_frame_start,source_frame_end,timeline_start_us,timeline_end_us,mapping_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                        params![Uuid::new_v4().to_string(), track.id, chunk.id, map.source_frame_start, map.source_frame_end,
                            map.timeline_start_us, map.timeline_end_us, map.mapping_json]).map_err(|e| e.to_string())?;
                    previous = Some(map.timeline_end_us);
                }
            }
            if boundary_gaps {
                for (start, end, reason) in [
                    (
                        0,
                        track.chunks[0].maps[0].timeline_start_us,
                        if self.plan_name == PARTIAL_PLAN {
                            "partial_prefix_start_gap"
                        } else {
                            "capture_start_gap"
                        },
                    ),
                    (
                        previous.unwrap(),
                        duration,
                        if self.plan_name == PARTIAL_PLAN {
                            "partial_prefix_end_gap"
                        } else {
                            "capture_stop_gap"
                        },
                    ),
                ] {
                    if start < end {
                        tx.execute("INSERT INTO audio_gaps(id,track_id,start_us,end_us,reason_code) VALUES(?1,?2,?3,?4,?5)",
                        params![Uuid::new_v4().to_string(), track.id, start, end, reason]).map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn convert(
    storage: &Storage,
    directory: &Directory,
    prefix: &Prefix,
    group: &FlacGroup,
    input_hash: &str,
    id: String,
    requested_cancel: &impl Fn() -> bool,
) -> Result<Chunk, String> {
    let cancelled = || requested_cancel() || storage.data_write_blocked();
    partial::check_cancel(&cancelled)?;
    let final_name = format!("{id}.flac");
    let receipt_name = format!(".{id}.json");
    let (mut receipt, mut receipt_stamp) =
        if let Some((bytes, stamp)) = directory.read_json(&receipt_name)? {
            let mut checkpoint: Checkpoint =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            #[cfg(any(target_os = "macos", windows))]
            if relocation::effective_checkpoint(
                storage,
                &directory.path,
                &receipt_name,
                &bytes,
                &mut checkpoint,
            )? {
                // A copied checkpoint still names the old inode. Persist the
                // new, independently placed identity before any truncation,
                // encoding or rename so another crash needs no JSON adoption.
                let staging = directory.optional(&checkpoint.staging_name, false)?;
                let published = directory.optional(&final_name, false)?;
                if staging.is_some() && published.is_some() {
                    return Err("移動したFLACの途中名と最終名が重複しています".into());
                }
                let file = staging
                    .or_else(|| published.filter(|_| checkpoint.audio.is_some()))
                    .ok_or("移動したFLACの所有実体がありません")?;
                checkpoint
                    .staging_identity
                    .as_ref()
                    .ok_or("移動したFLACの実体記録がありません")?
                    .require(&file)?;
                stamp.require(&directory.required(&receipt_name, false)?)?;
                storage.require_data_writable()?;
                directory.json(&receipt_name, &checkpoint, Some(&stamp))?;
                #[cfg(test)]
                crash_point("relocation-rebound");
                (
                    checkpoint,
                    directory
                        .read_json(&receipt_name)?
                        .ok_or("移動したFLACの記録がありません")?
                        .1,
                )
            } else {
                (checkpoint, stamp)
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            (checkpoint, stamp)
        } else {
            if directory.optional(&final_name, false)?.is_some() {
                return Err("所有証拠のない最終FLACは採用できません".into());
            }
            let receipt = Checkpoint {
                schema_version: 1,
                input_hash: input_hash.into(),
                chunk_id: id.clone(),
                staging_name: format!(".{id}.{}.part", Uuid::new_v4()),
                staging_identity: None,
                audio: None,
            };
            directory.json(&receipt_name, &receipt, None)?;
            #[cfg(test)]
            crash_point("reserved");
            let stamp = directory.read_json(&receipt_name)?.unwrap().1;
            (receipt, stamp)
        };
    if receipt.schema_version != 1 || receipt.input_hash != input_hash || receipt.chunk_id != id {
        return Err("FLAC区間のチェックポイントが原音声と一致しません".into());
    }
    let staging_uuid = receipt
        .staging_name
        .strip_prefix(&format!(".{id}."))
        .and_then(|name| name.strip_suffix(".part"));
    if !staging_uuid.is_some_and(uuid) {
        return Err("FLAC一時名の予約記録が不正です".into());
    }
    if receipt.staging_identity.is_none() {
        if receipt.audio.is_some() || directory.optional(&final_name, false)?.is_some() {
            return Err("実体が未確定のFLACを採用することはできません".into());
        }
        // A crash between exclusive creation and identity commit leaves only
        // an empty, unclaimed object: encoding has not started. Preserve it,
        // reserve a fresh name, and never adopt/truncate that unknown inode.
        if directory.optional(&receipt.staging_name, false)?.is_some() {
            receipt.staging_name = format!(".{id}.{}.part", Uuid::new_v4());
            directory.json(&receipt_name, &receipt, Some(&receipt_stamp))?;
            receipt_stamp = directory.read_json(&receipt_name)?.unwrap().1;
        }
        let file = directory.create(&receipt.staging_name)?;
        crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
        crate::native_fs::sync_handle(&directory.file).map_err(|e| e.to_string())?;
        #[cfg(test)]
        crash_point("created-before-identity");
        receipt.staging_identity = Some(Identity::file(&file)?);
        directory.json(&receipt_name, &receipt, Some(&receipt_stamp))?;
        receipt_stamp = directory.read_json(&receipt_name)?.unwrap().1;
        #[cfg(test)]
        crash_point("allocated");
    }
    let partial = &receipt.staging_name;
    let staging_identity = receipt.staging_identity.as_ref().unwrap();
    if receipt.audio.is_none() {
        if directory.optional(&final_name, false)?.is_some() {
            return Err("未確定のFLACが最終名へ移されています".into());
        }
        let mut file = directory.required(&partial, true)?;
        staging_identity.require(&file)?;
        receipt_stamp.require(&directory.required(&receipt_name, false)?)?;
        storage.require_data_writable()?;
        file.set_len(0).map_err(|e| e.to_string())?;
        file.rewind().map_err(|e| e.to_string())?;
        let mut source = prefix.source(group.first_entry, group.entries, &cancelled)?;
        let saved =
            encode_segment_file(&mut source, 0, group.frames, &mut file, &cancelled, &|_| {})?;
        staging_identity.require(&directory.required(partial, false)?)?;
        receipt.audio = Some(saved.into());
        directory.json(&receipt_name, &receipt, Some(&receipt_stamp))?;
        receipt_stamp = directory.read_json(&receipt_name)?.unwrap().1;
        #[cfg(test)]
        crash_point("encoded");
    }
    let audio = receipt.audio.unwrap();
    if audio.frames != group.frames
        || audio.sample_rate != prefix.entries().nth(group.first_entry).unwrap().sample_rate
        || audio.channels != prefix.entries().nth(group.first_entry).unwrap().channels
    {
        return Err("FLAC保存形式と原音声の区間が一致しません".into());
    }
    if directory.optional(&final_name, false)?.is_none() {
        let file = directory.required(&partial, false)?;
        staging_identity.require(&file)?;
        verify_audio(&file, &audio, &cancelled)?;
        receipt_stamp.require(&directory.required(&receipt_name, false)?)?;
        directory.rename(&partial, &final_name, true)?;
    } else if directory.optional(&partial, false)?.is_some() {
        return Err("FLACの一時名と最終名が重複しています".into());
    }
    let file = directory.required(&final_name, false)?;
    staging_identity.require(&file)?;
    let stamp = Stamp::read(&file)?;
    verify_derived_audio(storage, &file, &audio, prefix, group, &cancelled)?;
    stamp.require(&file)?;
    stamp.require(&directory.required(&final_name, false)?)?;
    receipt_stamp.require(&directory.required(&receipt_name, false)?)?;
    // Also re-sync an already published object after restart; existence alone
    // is not proof that its previous directory sync completed.
    crate::native_fs::sync_handle(&file).map_err(|e| e.to_string())?;
    crate::native_fs::sync_handle(&directory.file).map_err(|e| e.to_string())?;
    #[cfg(test)]
    crash_point("published");
    let relative = directory
        .path
        .strip_prefix(&storage.root)
        .map_err(|e| e.to_string())?
        .join(&final_name);
    Ok(Chunk {
        id,
        file: CapturedFile {
            relative,
            size_bytes: i64::try_from(audio.size).map_err(|_| "FLAC容量が範囲外です")?,
            hash: audio.sha256.clone(),
        },
        audio,
        maps: group.clock_segments.clone(),
        stamp,
        receipt: receipt_stamp,
    })
}
fn verify_audio(
    file: &File,
    proof: &AudioProof,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let verified = VerifiedAudio::open_cancellable(
        file.try_clone().map_err(|e| e.to_string())?,
        proof.size,
        &proof.sha256,
        cancelled,
    )?;
    if !verified.is_flac()
        || verified.format.frames != proof.frames
        || verified.format.sample_rate != proof.sample_rate
        || verified.format.channels != proof.channels
    {
        return Err("FLACの実音声とチェックポイントが一致しません".into());
    }
    Ok(())
}

/// JSON is not authentication that arbitrary valid FLAC came from this input.
/// Reconcile all decoded samples with pinned raw PCM and its defined 24-bit
/// quantization, including when resuming a previously published checkpoint.
fn verify_derived_audio(
    storage: &Storage,
    file: &File,
    proof: &AudioProof,
    prefix: &Prefix,
    group: &FlacGroup,
    requested_cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    let cancelled = || requested_cancel() || storage.data_write_blocked();
    let mut verified = VerifiedAudio::open_cancellable(
        file.try_clone().map_err(|e| e.to_string())?,
        proof.size,
        &proof.sha256,
        &cancelled,
    )?;
    if !verified.is_flac()
        || verified.format.frames != group.frames
        || verified.format.sample_rate != proof.sample_rate
        || verified.format.channels != proof.channels
    {
        return Err("最終FLACの形式が原音声と一致しません".into());
    }
    let mut source = prefix.source(group.first_entry, group.entries, &cancelled)?;
    let mut first = 0;
    while first < group.frames {
        partial::check_cancel(&cancelled)?;
        storage.require_data_writable()?;
        let count = (group.frames - first).min(4096);
        let raw = source.read_frames(first, count)?;
        let decoded = verified.read_frames(first, count)?;
        if decoded.len() != raw.len() {
            return Err("最終FLACのPCM長が原音声と一致しません".into());
        }
        for (raw, decoded) in raw.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            let value = f32::from_le_bytes(raw.try_into().unwrap());
            if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
                return Err("原音声PCMの値が不正です".into());
            }
            let quantized = (f64::from(value) * 8_388_608.0)
                .round()
                .clamp(-8_388_608.0, 8_388_607.0) as i32;
            if (quantized as f32 / 8_388_608.0).to_le_bytes() != decoded {
                return Err("最終FLACの実PCMが原音声と一致しません".into());
            }
        }
        first += count;
    }
    source.verify_unchanged()?;
    PcmSource::verify_unchanged(&verified)
}

#[cfg(test)]
pub(super) fn crash_point(phase: &str) {
    if std::env::var("MON_FINAL_FLAC_CRASH_PHASE").as_deref() == Ok(phase)
        && std::env::var("MON_FINAL_FLAC_CRASH_CHILD").as_deref() == Ok("synthetic-only")
    {
        #[cfg(unix)]
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
        #[cfg(windows)]
        std::process::abort();
        panic!("synthetic crash injection did not terminate its owned child");
    }
}
