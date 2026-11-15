//! Incremental live evidence. Inspect each newly committed CAF once rather
//! than re-reading the entire recording on every UI poll. Final publication
//! still independently verifies the complete immutable prefix.
use super::*;
use mon_core::capture::commit::RawPrefix;
use std::time::SystemTime;

struct Last {
    json: Stamp,
    audio: Stamp,
    extendable: bool,
    modified: SystemTime,
}
#[derive(Default)]
pub(crate) struct LivePrefix {
    prefix: RawPrefix,
    last: Option<Last>,
    closed: bool,
    invalidated: bool,
}
impl LivePrefix {
    pub(crate) fn read(
        &mut self,
        directory: &File,
        owner: &str,
        lane: Lane,
    ) -> Result<Option<(u64, SystemTime)>, String> {
        if self.invalidated {
            return Err("録音の確定範囲が変更されました".into());
        }
        let name = lane.tip_name();
        let Some(file) = open_at(directory, &name)? else {
            return if self.prefix.segments == 0 {
                Ok(None)
            } else {
                Err("録音の確定記録を確認できません".into())
            };
        };
        let (bytes, tip_stamp) = read_json(directory, &name, file, &|| false)?;
        let tip: Tip = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if tip.schema_version != 2
            || tip.owner_id != owner
            || tip.lane != lane.name()
            || tip.sealed_segments > MAX_SEGMENTS
            || tip.sealed_frames > SAFE
            || (tip.sealed_segments == 0
                && (tip.sealed_frames != 0 || tip.last_entry_sha256.is_some()))
            || (tip.sealed_segments > 0
                && (tip.sealed_frames == 0
                    || !tip.last_entry_sha256.as_deref().is_some_and(valid_hash)))
            || tip.sealed_segments < self.prefix.segments
            || (self.closed
                && (!tip.producer_closed || tip.sealed_segments != self.prefix.segments))
        {
            self.invalidated = true;
            return Err("録音の確定範囲・所属・連番が不正です".into());
        }
        if let Some(last) = &self.last {
            let result = last
                .json
                .require(&required(
                    directory,
                    &lane.name_at(self.prefix.segments, "json"),
                )?)
                .and_then(|_| {
                    last.audio.require(&required(
                        directory,
                        &lane.name_at(self.prefix.segments, "caf"),
                    )?)
                });
            if let Err(error) = result {
                self.invalidated = true;
                return Err(error);
            }
            if tip.sealed_segments > self.prefix.segments && !last.extendable {
                self.invalidated = true;
                return Err("録音の途中の分割が確定条件を満たしていません".into());
            }
        }
        let mut next = self.prefix;
        let mut last = None;
        for sequence in self.prefix.segments + 1..=tip.sealed_segments {
            let json_name = lane.name_at(sequence, "json");
            let (receipt, json) = read_json(
                directory,
                &json_name,
                required(directory, &json_name)?,
                &|| false,
            )?;
            let previous_hash = next.hash();
            let entry: Entry = serde_json::from_slice(&receipt).map_err(|e| e.to_string())?;
            entry.validate(owner, lane, sequence, next.frames, previous_hash.as_deref())?;
            let extendable = entry.frames >= u64::from(entry.sample_rate) * 4
                || (126..=128).contains(&entry.clock_segments.len())
                || (entry.schema_version == 3
                    && entry.frames
                        > u64::from(entry.sample_rate) * 4
                            - mon_core::media::voice::MAX_OUTPUT_FRAMES as u64);
            if sequence < tip.sealed_segments && !extendable {
                return Err("録音の途中の分割が確定条件を満たしていません".into());
            }
            let prepared = next.prepare(&receipt)?;
            let file = required(directory, &entry.audio_name)?;
            let audio = Stamp::of(&file)?;
            if audio.size != entry.audio_bytes {
                return Err("録音の分割音声のサイズが一致しません".into());
            }
            let (pcm, size, digest) = crate::audio::inspect_recorded_caf(
                file.try_clone().map_err(|e| e.to_string())?,
                &|| false,
            )?;
            if size != entry.audio_bytes
                || digest != entry.audio_sha256
                || pcm.frames != entry.frames
                || pcm.sample_rate != entry.sample_rate
                || pcm.channels != entry.channels
                || !pcm.is_raw_recording_segment()
                || (entry.schema_version == 3) != pcm.is_compact_recording_segment()
            {
                return Err("録音の分割音声と確定証拠が一致しません".into());
            }
            audio.require(&file)?;
            audio.require(&required(directory, &entry.audio_name)?)?;
            json.require(&required(directory, &json_name)?)?;
            next = prepared.next;
            let modified = file
                .metadata()
                .and_then(|m| m.modified())
                .map_err(|e| e.to_string())?;
            last = Some(Last {
                json,
                audio,
                extendable,
                modified,
            });
        }
        if next.frames != tip.sealed_frames || next.hash() != tip.last_entry_sha256 {
            self.invalidated = true;
            return Err("録音の確定記録と検証済み音声が一致しません".into());
        }
        // An advancing atomic tip is a retryable race, not a new observation.
        tip_stamp.require(&required(directory, &name)?)?;
        self.prefix = next;
        if let Some(last) = last {
            self.last = Some(last);
        }
        self.closed = tip.producer_closed;
        Ok(self.last.as_ref().map(|last| (next.frames, last.modified)))
    }
}
