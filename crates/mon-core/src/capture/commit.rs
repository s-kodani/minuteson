//! One raw-prefix commit state machine for all producers. Preparing a commit
//! performs no IO and never mutates the accepted prefix. Native sinks adopt
//! `next` only after PCM, receipt and the returned tip are stored successfully.
use super::{limits, request::valid_owner};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawPrefix {
    pub segments: u32,
    pub has_hash: u32,
    pub frames: u64,
    pub metadata_bytes: u64,
    pub clock_maps: u64,
    pub last_end_nanoseconds: u64,
    pub last_hash: [u8; 32],
    pub owner: [u8; 36],
    pub lane: u32,
}
impl Default for RawPrefix {
    fn default() -> Self {
        Self {
            segments: 0,
            has_hash: 0,
            frames: 0,
            metadata_bytes: 0,
            clock_maps: 0,
            last_end_nanoseconds: 0,
            last_hash: [0; 32],
            owner: [0; 36],
            lane: 0,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Receipt {
    schema_version: u32,
    #[serde(default)]
    sample_encoding: Option<String>,
    owner_id: String,
    lane: String,
    sequence: u32,
    audio_name: String,
    source_frame_start: u64,
    frames: u64,
    sample_rate: u32,
    channels: u32,
    capture_origin_nanoseconds: u64,
    clock_segments: Vec<ClockMap>,
    clock_nanoseconds: Vec<Endpoints>,
    audio_bytes: u64,
    #[serde(rename = "audioSHA256")]
    audio_sha256: String,
    #[serde(rename = "previousEntrySHA256")]
    previous_entry_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ClockMap {
    source_frame_start: i64,
    source_frame_end: i64,
    timeline_start_us: i64,
    timeline_end_us: i64,
    mapping_json: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Endpoints {
    start_nanoseconds: i64,
    end_nanoseconds: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    schema_version: u32,
    method: String,
}
impl Receipt {
    fn valid_clock(&self) -> bool {
        let (mut frame, mut time, mut last_ns) = (0, 0, 0);
        for (map, clock) in self.clock_segments.iter().zip(&self.clock_nanoseconds) {
            if map.source_frame_start != frame
                || map.source_frame_end <= frame
                || map.source_frame_end > self.frames as i64
                || map.timeline_start_us < time
                || (frame == 0 && map.timeline_start_us != 0)
                || map.timeline_end_us <= map.timeline_start_us
                || map.timeline_end_us > limits::SAFE_INTEGER as i64
                || map.mapping_json.len() > 4096
            {
                return false;
            }
            let Ok(mapping) = serde_json::from_str::<Mapping>(&map.mapping_json) else {
                return false;
            };
            if mapping.schema_version != 1 || mapping.method != "linear" {
                return false;
            }
            let input = (map.source_frame_end - frame) as u128 * 1_000_000;
            let output = (map.timeline_end_us - map.timeline_start_us) as u128
                * u128::from(self.sample_rate);
            if input > output * 64 || output > input * 64 {
                return false;
            }
            if clock.start_nanoseconds < self.capture_origin_nanoseconds as i64
                || clock.end_nanoseconds <= clock.start_nanoseconds
                || last_ns > clock.start_nanoseconds
                || (clock.start_nanoseconds - self.capture_origin_nanoseconds as i64) / 1000
                    != map.timeline_start_us
                || (clock.end_nanoseconds - self.capture_origin_nanoseconds as i64) / 1000
                    != map.timeline_end_us
            {
                return false;
            }
            frame = map.source_frame_end;
            time = map.timeline_end_us;
            last_ns = clock.end_nanoseconds;
        }
        frame == self.frames as i64
            && (time as u128 * u128::from(self.sample_rate)).div_ceil(1_000_000)
                <= u128::from(limits::SAFE_INTEGER)
            && time
                .checked_mul(1000)
                .and_then(|n| n.checked_add(self.capture_origin_nanoseconds as i64))
                .is_some()
    }
}
pub struct PreparedCommit {
    pub next: RawPrefix,
    pub tip: Vec<u8>,
}
const INVALID: &str = "raw segment commit does not extend the accepted prefix";
fn hex(hash: &[u8]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl RawPrefix {
    pub fn hash(&self) -> Option<String> {
        (self.has_hash == 1).then(|| hex(&self.last_hash))
    }
    pub fn valid(&self) -> bool {
        self.segments <= limits::MAX_SEGMENTS
            && self.frames <= limits::SAFE_INTEGER
            && self.metadata_bytes <= limits::MAX_METADATA_BYTES - limits::TIP_RESERVE_BYTES
            && self.clock_maps <= limits::MAX_CLOCK_MAPS
            && if self.segments == 0 {
                self.has_hash == 0
                    && self.frames == 0
                    && self.metadata_bytes == 0
                    && self.clock_maps == 0
                    && self.last_end_nanoseconds == 0
                    && self.lane == 0
            } else {
                self.has_hash == 1
                    && self.frames > 0
                    && self.metadata_bytes > 0
                    && self.clock_maps > 0
                    && self.last_end_nanoseconds > 0
                    && self.last_end_nanoseconds <= i64::MAX as u64
                    && matches!(self.lane, 1 | 2)
                    && std::str::from_utf8(&self.owner)
                        .ok()
                        .is_some_and(valid_owner)
            }
    }
    pub fn tip(&self, owner: &str, lane: &str, closed: bool) -> Result<Vec<u8>, &'static str> {
        if !self.valid() || !valid_owner(owner) || !matches!(lane, "microphone" | "system") {
            return Err(INVALID);
        }
        if self.segments > 0
            && (self.owner.as_slice() != owner.as_bytes()
                || self.lane != if lane == "microphone" { 1 } else { 2 })
        {
            return Err(INVALID);
        }
        serde_json::to_vec(&serde_json::json!({"schemaVersion":2,"ownerId":owner,"lane":lane,
            "sealedSegments":self.segments,"sealedFrames":self.frames,"lastEntrySHA256":self.hash(),"producerClosed":closed}))
            .map_err(|_|INVALID)
    }
    pub fn prepare(&self, bytes: &[u8]) -> Result<PreparedCommit, &'static str> {
        if bytes.len() > limits::MAX_ENTRY_BYTES as usize || !self.valid() {
            return Err(INVALID);
        }
        let entry: Receipt = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
        if !valid_owner(&entry.owner_id)
            || !matches!(entry.lane.as_str(), "microphone" | "system")
            || entry.sequence != self.segments + 1
            || entry.source_frame_start != self.frames
            || entry.frames == 0
            || !(8000..=384000).contains(&entry.sample_rate)
            || !(1..=8).contains(&entry.channels)
            || entry.frames > limits::SAFE_INTEGER - self.frames
            || entry.frames > u64::from(entry.sample_rate) * 17 / 4
            || entry.audio_name != format!("raw-{}-{:08}.caf", entry.lane, entry.sequence)
            || super::legacy_pcm::audio_bytes(
                entry.schema_version,
                entry.sample_encoding.as_deref(),
                entry.sample_rate,
                entry.channels,
                entry.frames,
            ) != Some(entry.audio_bytes)
            || !valid_hash(&entry.audio_sha256)
            || entry.previous_entry_sha256 != self.hash()
            || entry.clock_segments.len() != entry.clock_nanoseconds.len()
            || entry.clock_segments.len() > 128
            || entry.capture_origin_nanoseconds > i64::MAX as u64
            || entry.capture_origin_nanoseconds < self.last_end_nanoseconds
            || (self.segments > 0
                && (self.owner.as_slice() != entry.owner_id.as_bytes()
                    || self.lane != if entry.lane == "microphone" { 1 } else { 2 }))
            || !limits::permits_commit(
                self.segments,
                self.metadata_bytes,
                self.clock_maps,
                bytes.len() as u64,
                entry.clock_segments.len() as u64,
            )
        {
            return Err(INVALID);
        }
        if !entry.valid_clock() {
            return Err(INVALID);
        }
        let mut owner = [0; 36];
        owner.copy_from_slice(entry.owner_id.as_bytes());
        let next = RawPrefix {
            segments: entry.sequence,
            has_hash: 1,
            frames: self.frames + entry.frames,
            metadata_bytes: self.metadata_bytes + bytes.len() as u64,
            clock_maps: self.clock_maps + entry.clock_segments.len() as u64,
            last_end_nanoseconds: entry
                .clock_nanoseconds
                .last()
                .ok_or(INVALID)?
                .end_nanoseconds as u64,
            last_hash: Sha256::digest(bytes).into(),
            owner,
            lane: if entry.lane == "microphone" { 1 } else { 2 },
        };
        let tip = next.tip(&entry.owner_id, &entry.lane, false)?;
        Ok(PreparedCommit { next, tip })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(prefix: &RawPrefix) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"schemaVersion":2,
      "ownerId":"735027d4-a03e-43d3-830a-21e767dfaf88","lane":"microphone","sequence":prefix.segments+1,
      "audioName":format!("raw-microphone-{:08}.caf",prefix.segments+1),"sourceFrameStart":prefix.frames,
      "frames":1,"sampleRate":48000,"channels":1,"captureOriginNanoseconds":prefix.last_end_nanoseconds+1000000,
      "clockSegments":[{"sourceFrameStart":0,"sourceFrameEnd":1,"timelineStartUs":0,"timelineEndUs":21,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"}],
      "clockNanoseconds":[{"startNanoseconds":prefix.last_end_nanoseconds+1000000,"endNanoseconds":prefix.last_end_nanoseconds+1021000}],
      "audioBytes":72,"audioSHA256":"a".repeat(64),"previousEntrySHA256":prefix.hash()})).unwrap()
    }
    #[test]
    fn prepare_never_advances_on_failure_or_without_sink_commit() {
        let prefix = RawPrefix::default();
        let bytes = entry(&prefix);
        let plan = prefix.prepare(&bytes).unwrap();
        assert_eq!(prefix, RawPrefix::default());
        assert_eq!(plan.next.segments, 1);
        assert_eq!(plan.next.frames, 1);
        assert_eq!(
            plan.next.last_hash,
            <[u8; 32]>::from(Sha256::digest(&bytes))
        );
        assert!(plan.next.prepare(&bytes).is_err());
        let second = plan.next.prepare(&entry(&plan.next)).unwrap();
        assert_eq!(second.next.segments, 2);
        let mut corrupt = plan.next;
        corrupt.last_hash[0] ^= 1;
        assert!(corrupt.prepare(&entry(&plan.next)).is_err());
        let exhausted = RawPrefix {
            segments: limits::MAX_SEGMENTS,
            ..plan.next
        };
        assert!(exhausted.prepare(&entry(&exhausted)).is_err());
    }

    #[test]
    fn compact_receipts_bind_pcm16_geometry_without_reinterpreting_legacy_float() {
        let prefix = RawPrefix::default();
        let mut value: serde_json::Value = serde_json::from_slice(&entry(&prefix)).unwrap();
        value["schemaVersion"] = 3.into();
        value["sampleEncoding"] = "pcm16".into();
        value["sampleRate"] = 16000.into();
        value["audioBytes"] = 70.into();
        assert!(prefix.prepare(&serde_json::to_vec(&value).unwrap()).is_ok());
        for (key, wrong) in [
            ("sampleRate", serde_json::json!(48000)),
            ("channels", serde_json::json!(2)),
            ("audioBytes", serde_json::json!(72)),
            ("sampleEncoding", serde_json::json!("float32")),
            ("schemaVersion", serde_json::json!(2)),
        ] {
            let mut bad = value.clone();
            bad[key] = wrong;
            assert!(
                prefix.prepare(&serde_json::to_vec(&bad).unwrap()).is_err(),
                "{key}"
            );
        }
    }
}
