//! Single-WAV recording primitives. Native workers own acquisition, ordered
//! delivery and durable IO; none of these state machines performs file IO.
use super::{limits, request::valid_owner};
use crate::media::resampling::Filter;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

pub const RATE: u32 = 16_000;
pub const WAV_HEADER_BYTES: usize = 80;
pub const AUDIO_NAME: &str = "recording.wav";
pub const TIP_NAME: &str = "single-wav.json";
pub const MIX_POLICY: &str = "meeting_live_wav_pcm16_16000_interval_gain_v1";
const INVALID: &str = "invalid single WAV recording evidence";
const MAX_BLOCK: usize = 4096;
const MAX_HALO: usize = 1100;

/// Reserve ds64's complete footprint from the start, keeping PCM at offset 80
/// when an initially ordinary RIFF crosses the 32-bit size limit.
pub fn header(frames: u64) -> Result<[u8; WAV_HEADER_BYTES], &'static str> {
    let bytes = frames.checked_mul(2).ok_or("WAV length overflow")?;
    let total = bytes.checked_add(80).ok_or("WAV length overflow")?;
    let riff = total - 8;
    let rf64 = riff > u64::from(u32::MAX);
    let mut result = [0u8; WAV_HEADER_BYTES];
    result[..4].copy_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    result[4..8].copy_from_slice(&(if rf64 { u32::MAX } else { riff as u32 }).to_le_bytes());
    result[8..12].copy_from_slice(b"WAVE");
    result[12..16].copy_from_slice(if rf64 { b"ds64" } else { b"JUNK" });
    result[16..20].copy_from_slice(&28u32.to_le_bytes());
    if rf64 {
        result[20..28].copy_from_slice(&riff.to_le_bytes());
        result[28..36].copy_from_slice(&bytes.to_le_bytes());
        result[36..44].copy_from_slice(&frames.to_le_bytes());
    }
    result[48..52].copy_from_slice(b"fmt ");
    result[52..56].copy_from_slice(&16u32.to_le_bytes());
    result[56..58].copy_from_slice(&1u16.to_le_bytes());
    result[58..60].copy_from_slice(&1u16.to_le_bytes());
    result[60..64].copy_from_slice(&RATE.to_le_bytes());
    result[64..68].copy_from_slice(&(RATE * 2).to_le_bytes());
    result[68..70].copy_from_slice(&2u16.to_le_bytes());
    result[70..72].copy_from_slice(&16u16.to_le_bytes());
    result[72..76].copy_from_slice(b"data");
    result[76..80].copy_from_slice(&(if rf64 { u32::MAX } else { bytes as u32 }).to_le_bytes());
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WavReceipt {
    pub schema_version: u32,
    pub owner_id: String,
    pub sequence: u32,
    pub frame_start: u64,
    pub frames: u64,
    pub capture_origin_nanoseconds: i64,
    #[serde(rename = "pcmSHA256")]
    pub pcm_sha256: String,
    #[serde(rename = "previousEntrySHA256")]
    pub previous_entry_sha256: Option<String>,
    /// Absolute output-frame intervals with no captured input. Receiving PCM
    /// whose values are zero does not create an interval here.
    #[serde(default)]
    pub missing_ranges: Vec<[u64; 2]>,
    /// Captured output-frame counts for microphone and system respectively.
    #[serde(default)]
    pub coverage_frames: [u64; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WavTip {
    pub schema_version: u32,
    pub owner_id: String,
    pub file_name: String,
    pub frames: u64,
    pub checkpoints: u32,
    #[serde(rename = "lastEntrySHA256")]
    pub last_entry_sha256: Option<String>,
    pub capture_origin_nanoseconds: Option<i64>,
    pub producer_closed: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavPrefix {
    pub frames: u64,
    pub checkpoints: u32,
    pub metadata_bytes: u64,
    pub origin_nanoseconds: i64,
    pub last_hash: [u8; 32],
    pub owner: [u8; 36],
}

impl Default for WavPrefix {
    fn default() -> Self {
        Self {
            frames: 0,
            checkpoints: 0,
            metadata_bytes: 0,
            origin_nanoseconds: 0,
            last_hash: [0; 32],
            owner: [0; 36],
        }
    }
}

pub struct PreparedWavCommit {
    pub next: WavPrefix,
    pub tip: Vec<u8>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl WavPrefix {
    pub fn hash(&self) -> Option<String> {
        (self.checkpoints > 0).then(|| hex(&self.last_hash))
    }

    fn valid(&self) -> bool {
        if self.checkpoints == 0 {
            return *self == Self::default();
        }
        self.frames > 0
            && self.frames <= limits::SAFE_INTEGER
            && self.checkpoints <= limits::MAX_SEGMENTS
            && self.metadata_bytes <= limits::MAX_METADATA_BYTES - limits::TIP_RESERVE_BYTES
            && self.origin_nanoseconds >= 0
            && std::str::from_utf8(&self.owner).is_ok_and(valid_owner)
    }

    pub fn tip(&self, owner: &str, closed: bool) -> Result<Vec<u8>, &'static str> {
        if !self.valid()
            || !valid_owner(owner)
            || (self.checkpoints > 0 && self.owner.as_slice() != owner.as_bytes())
        {
            return Err(INVALID);
        }
        serde_json::to_vec(&WavTip {
            schema_version: 1,
            owner_id: owner.to_owned(),
            file_name: AUDIO_NAME.into(),
            frames: self.frames,
            checkpoints: self.checkpoints,
            last_entry_sha256: self.hash(),
            capture_origin_nanoseconds: (self.checkpoints > 0).then_some(self.origin_nanoseconds),
            producer_closed: closed,
        })
        .map_err(|_| INVALID)
    }

    /// This only prepares evidence. Adopt `next` after PCM, receipt and tip
    /// have been synchronized in that order by the native save worker.
    pub fn prepare(&self, bytes: &[u8]) -> Result<PreparedWavCommit, &'static str> {
        if !self.valid() || bytes.len() > limits::MAX_ENTRY_BYTES as usize {
            return Err(INVALID);
        }
        let receipt: WavReceipt = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
        let frames = self.frames.checked_add(receipt.frames).ok_or(INVALID)?;
        let metadata_bytes = self
            .metadata_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(INVALID)?;
        let mut previous_end = receipt.frame_start;
        for [start, end] in &receipt.missing_ranges {
            if *start < previous_end || *end <= *start || *end > frames {
                return Err(INVALID);
            }
            previous_end = *end;
        }
        if receipt.schema_version != 1
            || !valid_owner(&receipt.owner_id)
            || receipt.sequence != self.checkpoints + 1
            || receipt.frame_start != self.frames
            || receipt.frames == 0
            || receipt.frames > u64::from(RATE) * 4 + MAX_BLOCK as u64
            || frames > limits::SAFE_INTEGER
            || receipt.capture_origin_nanoseconds < 0
            || receipt
                .coverage_frames
                .iter()
                .any(|count| *count > receipt.frames)
            || !valid_hash(&receipt.pcm_sha256)
            || receipt.previous_entry_sha256 != self.hash()
            || self.checkpoints >= limits::MAX_SEGMENTS
            || metadata_bytes > limits::MAX_METADATA_BYTES - limits::TIP_RESERVE_BYTES
            || (self.checkpoints > 0
                && (self.owner.as_slice() != receipt.owner_id.as_bytes()
                    || self.origin_nanoseconds != receipt.capture_origin_nanoseconds))
        {
            return Err(INVALID);
        }
        header(frames)?;
        let mut owner = [0; 36];
        owner.copy_from_slice(receipt.owner_id.as_bytes());
        let next = Self {
            frames,
            checkpoints: receipt.sequence,
            metadata_bytes,
            origin_nanoseconds: receipt.capture_origin_nanoseconds,
            last_hash: Sha256::digest(bytes).into(),
            owner,
        };
        let tip = next.tip(&receipt.owner_id, false)?;
        Ok(PreparedWavCommit { next, tip })
    }
}

struct Span {
    first: u64,
    start: i64,
    end: i64,
    samples: Vec<i16>,
}

#[derive(Default)]
struct Lane {
    spans: VecDeque<Span>,
    received: u64,
    sequence: u64,
    epoch: u64,
    last_end: Option<i64>,
    through: i64,
    filter: Option<((usize, i64), Filter)>,
}

/// Ordered normalized PCM enters on one DSP owner; completed mixed blocks
/// leave through a borrowed callback for the independent native save worker.
pub struct LiveMixer {
    lanes: [Lane; 2],
    origin: Option<i64>,
    emitted: u64,
    buffered_samples: usize,
    buffered_spans: usize,
    failed: bool,
    finished: bool,
}

impl Default for LiveMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveMixer {
    pub fn new() -> Self {
        Self {
            lanes: [Lane::default(), Lane::default()],
            origin: None,
            emitted: 0,
            buffered_samples: 0,
            buffered_spans: 0,
            failed: false,
            finished: false,
        }
    }

    pub fn push(
        &mut self,
        lane: u32,
        epoch: u64,
        sequence: u64,
        samples: &[i16],
        start: i64,
        end: i64,
    ) -> Result<(), &'static str> {
        let result = self.push_inner(lane, epoch, sequence, samples, start, end);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(
        &mut self,
        lane: u32,
        epoch: u64,
        sequence: u64,
        samples: &[i16],
        start: i64,
        end: i64,
    ) -> Result<(), &'static str> {
        if self.failed
            || self.finished
            || lane > 1
            || samples.is_empty()
            || samples.len() > MAX_BLOCK
            || start < 0
            || end <= start
        {
            return Err("invalid live WAV PCM");
        }
        let origin = self.origin.unwrap_or(start);
        if self.emitted > 0
            && i128::from(start) * i128::from(RATE)
                < i128::from(origin) * i128::from(RATE) + i128::from(self.emitted) * 1_000_000_000
        {
            return Err("live WAV PCM predates mixed frontier");
        }
        let width = (end - start) as u128 * u128::from(RATE);
        let signal = samples.len() as u128 * 1_000_000_000;
        if signal > width * 64
            || width > signal * 64
            || self
                .buffered_samples
                .checked_add(samples.len())
                .is_none_or(|n| n * 2 > limits::MAX_STORAGE_PENDING_BYTES as usize)
            || self.buffered_spans >= limits::MAX_STORAGE_PENDING_JOBS as usize
        {
            return Err("live WAV PCM capacity exceeded");
        }
        let source = &mut self.lanes[lane as usize];
        if sequence
            != source
                .sequence
                .checked_add(1)
                .ok_or("live WAV sequence overflow")?
            || epoch < source.epoch
            || source.last_end.is_some_and(|previous| start < previous)
        {
            return Err("live WAV input order mismatch");
        }
        let received = source
            .received
            .checked_add(samples.len() as u64)
            .ok_or("live WAV length overflow")?;
        source.spans.push_back(Span {
            first: source.received,
            start,
            end,
            samples: samples.to_vec(),
        });
        source.received = received;
        source.sequence = sequence;
        source.epoch = epoch;
        source.last_end = Some(end);
        source.through = source.through.max(end);
        self.origin = Some(if self.emitted == 0 {
            origin.min(start)
        } else {
            origin
        });
        self.buffered_samples += samples.len();
        self.buffered_spans += 1;
        Ok(())
    }

    /// A barrier is sequenced after all PCM it covers, including converter
    /// tails. It is an observation of admitted data, not an OS timing promise.
    pub fn advance(&mut self, lane: u32, sequence: u64, through: i64) -> Result<(), &'static str> {
        if self.failed || self.finished || lane > 1 || through < 0 {
            self.failed = true;
            return Err("invalid live WAV barrier");
        }
        let source = &mut self.lanes[lane as usize];
        if sequence != source.sequence || through < source.through {
            self.failed = true;
            return Err("live WAV barrier overtook PCM");
        }
        source.through = through;
        Ok(())
    }

    pub fn drain(
        &mut self,
        through: i64,
        write: &mut impl FnMut(&[i16], u64, i64, u32) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        if self.failed || self.finished || through < 0 {
            return Err("invalid live WAV drain");
        }
        let Some(origin) = self.origin else {
            return Ok(());
        };
        let safe = through
            .min(self.lanes[0].through)
            .min(self.lanes[1].through);
        let limit = ((safe.saturating_sub(origin).max(0) as u128) * u128::from(RATE)
            / 1_000_000_000) as u64;
        let result = self.emit_to(limit, false, write);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub fn finish(
        &mut self,
        write: &mut impl FnMut(&[i16], u64, i64, u32) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        if self.failed {
            return Err("live WAV mixer failed");
        }
        if self.finished {
            return Ok(());
        }
        if let Some(origin) = self.origin {
            let end = self
                .lanes
                .iter()
                .filter_map(|lane| lane.last_end)
                .max()
                .unwrap_or(origin);
            let safe = self.lanes[0].through.min(self.lanes[1].through).max(end);
            let limit = ((safe.saturating_sub(origin).max(0) as u128) * u128::from(RATE))
                .div_ceil(1_000_000_000) as u64;
            if let Err(error) = self.emit_to(limit, true, write) {
                self.failed = true;
                return Err(error);
            }
        }
        self.finished = true;
        Ok(())
    }

    fn emit_to(
        &mut self,
        limit: u64,
        finalizing: bool,
        write: &mut impl FnMut(&[i16], u64, i64, u32) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let origin = self.origin.ok_or("live WAV origin missing")?;
        if limit > limits::SAFE_INTEGER {
            return Err("live WAV output overflow");
        }
        while self.emitted < limit {
            let first = self.emitted;
            let mut pcm = Vec::with_capacity(MAX_BLOCK);
            let mut coverage = None;
            while first + (pcm.len() as u64) < limit && pcm.len() < MAX_BLOCK {
                let frame = first + pcm.len() as u64;
                let time = origin as u128 * u128::from(RATE) + u128::from(frame) * 1_000_000_000;
                let left = self.lanes[0].sample(time, finalizing)?;
                let right = self.lanes[1].sample(time, finalizing)?;
                if matches!(left, Sample::Pending) || matches!(right, Sample::Pending) {
                    break;
                }
                let mask = u32::from(matches!(left, Sample::Value(_)))
                    | (u32::from(matches!(right, Sample::Value(_))) << 1);
                if coverage.is_some_and(|old| old != mask) {
                    break;
                }
                coverage = Some(mask);
                let mut sum = 0.0;
                let mut count = 0;
                for value in [left, right] {
                    if let Sample::Value(value) = value {
                        sum += value;
                        count += 1;
                    }
                }
                let value = if count == 0 {
                    0.0
                } else {
                    sum / f64::from(count)
                };
                pcm.push(
                    value
                        .round()
                        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16,
                );
            }
            if pcm.is_empty() {
                break;
            }
            write(&pcm, first, origin, coverage.unwrap())?;
            self.emitted += pcm.len() as u64;
            let next_time =
                origin as u128 * u128::from(RATE) + u128::from(self.emitted) * 1_000_000_000;
            for lane in &mut self.lanes {
                while lane.spans.len() > 1 {
                    let first = &lane.spans[0];
                    // Keep a bounded left filter halo across packet boundaries,
                    // even when much later input has already been queued.
                    if first.end as u128 * u128::from(RATE) + MAX_HALO as u128 * 1_000_000_000
                        > next_time
                    {
                        break;
                    }
                    let removed = lane.spans.pop_front().unwrap();
                    self.buffered_samples -= removed.samples.len();
                    self.buffered_spans -= 1;
                }
            }
        }
        Ok(())
    }
}

enum Sample {
    Missing,
    Pending,
    Value(f64),
}

impl Lane {
    fn sample(&mut self, time: u128, finalizing: bool) -> Result<Sample, &'static str> {
        let scale = u128::from(RATE);
        let Some(index) = self
            .spans
            .iter()
            .position(|span| span.start as u128 * scale <= time && time < span.end as u128 * scale)
        else {
            return Ok(Sample::Missing);
        };
        let span = &self.spans[index];
        let count = span.samples.len();
        let width = span.end - span.start;
        let denominator = width as u128 * scale;
        let position = (time - span.start as u128 * scale) * count as u128;
        let base = (position / denominator) as usize;
        if count as u128 * 1_000_000_000 == denominator && position.is_multiple_of(denominator) {
            return Ok(Sample::Value(f64::from(span.samples[base])));
        }
        let key = (count, width);
        if self.filter.as_ref().is_none_or(|(old, _)| *old != key) {
            self.filter = Some((
                key,
                Filter::for_step(count as u128 * 1_000_000_000, denominator)
                    .map_err(|_| "live WAV clock ratio unsupported")?,
            ));
        }
        let filter = &self.filter.as_ref().unwrap().1;
        let half = usize::try_from(filter.half).map_err(|_| "live WAV filter overflow")?;
        if half > MAX_HALO {
            return Err("live WAV filter capacity exceeded");
        }
        let raw = span.first + base as u64;
        let mut run_first = index;
        while run_first > 0 && self.spans[run_first - 1].end == self.spans[run_first].start {
            run_first -= 1;
        }
        let mut run_last = index;
        while run_last + 1 < self.spans.len()
            && self.spans[run_last].end == self.spans[run_last + 1].start
        {
            run_last += 1;
        }
        let first = self.spans[run_first].first;
        let last_span = &self.spans[run_last];
        let last = last_span.first + last_span.samples.len() as u64;
        if !finalizing
            && raw + half as u64 >= last
            && run_last + 1 == self.spans.len()
            && self.through <= last_span.end
        {
            return Ok(Sample::Pending);
        }
        let from = raw.saturating_sub(half as u64).max(first);
        let to = (raw + half as u64 + 2).min(last);
        let mut input = [0_f32; MAX_HALO * 2 + 3];
        let mut at = 0;
        for source in self
            .spans
            .iter()
            .skip(run_first)
            .take(run_last - run_first + 1)
        {
            let start = from
                .saturating_sub(source.first)
                .min(source.samples.len() as u64) as usize;
            let end = to
                .saturating_sub(source.first)
                .min(source.samples.len() as u64) as usize;
            for value in &source.samples[start..end] {
                input[at] = f32::from(*value) / 32768.0;
                at += 1;
            }
        }
        if at == 0 {
            return Err("live WAV filter samples missing");
        }
        let numerator = position + u128::from(span.first) * denominator;
        Ok(Sample::Value(
            f64::from(filter.sample_at(&input[..at], numerator, denominator, from)) * 32768.0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: &str = "8fded017-0c11-4caf-aab3-ce668fd67275";

    #[test]
    fn fixed_header_switches_to_rf64_without_moving_pcm() {
        let small = header(160).unwrap();
        assert_eq!(&small[..4], b"RIFF");
        assert_eq!(&small[12..16], b"JUNK");
        assert_eq!(&small[72..76], b"data");
        assert_eq!(u32::from_le_bytes(small[76..80].try_into().unwrap()), 320);
        let large = header(u64::from(u32::MAX)).unwrap();
        assert_eq!(&large[..4], b"RF64");
        assert_eq!(&large[12..16], b"ds64");
        assert_eq!(
            u64::from_le_bytes(large[36..44].try_into().unwrap()),
            u64::from(u32::MAX)
        );
        assert!(header(u64::MAX).is_err());
    }

    fn receipt(prefix: &WavPrefix, count: u64) -> Vec<u8> {
        serde_json::to_vec(&WavReceipt {
            schema_version: 1,
            owner_id: OWNER.into(),
            sequence: prefix.checkpoints + 1,
            frame_start: prefix.frames,
            frames: count,
            capture_origin_nanoseconds: 1_000_000,
            pcm_sha256: "ab".repeat(32),
            previous_entry_sha256: prefix.hash(),
            missing_ranges: vec![],
            coverage_frames: [0, 0],
        })
        .unwrap()
    }

    #[test]
    fn checkpoint_preparation_is_atomic_and_binds_order_owner_and_bytes() {
        let empty = WavPrefix::default();
        let bytes = receipt(&empty, 64000);
        let prepared = empty.prepare(&bytes).unwrap();
        assert_eq!(empty, WavPrefix::default());
        assert_eq!(prepared.next.frames, 64000);
        assert_eq!(prepared.next.hash(), Some(hex(&Sha256::digest(&bytes))));
        assert!(prepared.next.prepare(&bytes).is_err());
        let tip: WavTip = serde_json::from_slice(&prepared.next.tip(OWNER, true).unwrap()).unwrap();
        assert!(tip.producer_closed);
        let mut second: WavReceipt = serde_json::from_slice(&receipt(&prepared.next, 10)).unwrap();
        second.owner_id = "a7098979-5715-4b38-a4ec-85709da792ea".into();
        assert!(prepared
            .next
            .prepare(&serde_json::to_vec(&second).unwrap())
            .is_err());
        second.owner_id = OWNER.into();
        second.previous_entry_sha256 = Some("cd".repeat(32));
        assert!(prepared
            .next
            .prepare(&serde_json::to_vec(&second).unwrap())
            .is_err());
    }

    #[test]
    fn interval_gain_preserves_single_input_and_counts_received_silence() {
        let mut mixer = LiveMixer::new();
        mixer
            .push(0, 1, 1, &[12000; 8], 1_000_000, 1_500_000)
            .unwrap();
        mixer.push(1, 1, 1, &[0; 4], 1_250_000, 1_500_000).unwrap();
        let mut output = Vec::new();
        mixer
            .finish(&mut |pcm, first, origin, _| {
                assert_eq!(first, output.len() as u64);
                assert_eq!(origin, 1_000_000);
                output.extend_from_slice(pcm);
                Ok(())
            })
            .unwrap();
        assert_eq!(output, [12000, 12000, 12000, 12000, 6000, 6000, 6000, 6000]);
    }

    #[test]
    fn barrier_cannot_overtake_pcm_and_late_pcm_is_never_retimed() {
        let mut mixer = LiveMixer::new();
        mixer
            .push(0, 1, 1, &[9000; 16], 1_000_000, 2_000_000)
            .unwrap();
        assert!(mixer.advance(0, 2, 2_000_000).is_err());
        let mut mixer = LiveMixer::new();
        mixer
            .push(0, 1, 1, &[9000; 16], 1_000_000, 2_000_000)
            .unwrap();
        mixer.advance(1, 0, 2_000_000).unwrap();
        let mut output = Vec::new();
        mixer
            .drain(2_000_000, &mut |pcm, _, _, _| {
                output.extend_from_slice(pcm);
                Ok(())
            })
            .unwrap();
        assert_eq!(output, [9000; 16]);
        assert!(mixer
            .push(1, 1, 1, &[1000; 16], 1_000_000, 2_000_000)
            .is_err());
        assert_eq!(output, [9000; 16]);
    }

    #[test]
    fn sink_failure_is_terminal_and_does_not_advertise_another_block() {
        let mut mixer = LiveMixer::new();
        mixer.push(0, 1, 1, &[1000; 4096], 0, 256_000_000).unwrap();
        assert!(mixer.finish(&mut |_, _, _, _| Err("disk failed")).is_err());
        let mut called = false;
        assert!(mixer
            .finish(&mut |_, _, _, _| {
                called = true;
                Ok(())
            })
            .is_err());
        assert!(!called);
    }

    #[test]
    fn clock_filter_is_independent_of_pcm_packet_boundaries() {
        let input: Vec<i16> = (0..1280)
            .map(|n| ((n * 37) % 30000 - 15000) as i16)
            .collect();
        let run = |chunk: usize| {
            let mut mixer = LiveMixer::new();
            let mut result = Vec::new();
            let mut sequence = 0;
            for first in (0..input.len()).step_by(chunk) {
                sequence += 1;
                let end = (first + chunk).min(input.len());
                let start_ns = 1_000_000_000 + first as i64 * 62_562;
                let end_ns = 1_000_000_000 + end as i64 * 62_562;
                mixer
                    .push(0, 1, sequence, &input[first..end], start_ns, end_ns)
                    .unwrap();
                mixer.advance(1, 0, end_ns).unwrap();
                mixer
                    .drain(end_ns, &mut |pcm, _, _, mask| {
                        assert_eq!(mask, 1);
                        result.extend_from_slice(pcm);
                        Ok(())
                    })
                    .unwrap();
            }
            mixer
                .finish(&mut |pcm, _, _, mask| {
                    assert_eq!(mask, 1);
                    result.extend_from_slice(pcm);
                    Ok(())
                })
                .unwrap();
            result
        };
        assert_eq!(run(1280), run(320));
        assert_eq!(run(1280), run(80));
    }

    #[test]
    fn output_gaps_have_explicit_coverage_without_confusing_zero_pcm() {
        let mut mixer = LiveMixer::new();
        mixer.push(0, 1, 1, &[0; 4], 1_000_000, 1_250_000).unwrap();
        mixer
            .push(0, 1, 2, &[8000; 4], 1_500_000, 1_750_000)
            .unwrap();
        let mut blocks = Vec::new();
        mixer
            .finish(&mut |pcm, first, _, mask| {
                blocks.push((first, pcm.to_vec(), mask));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            blocks,
            vec![
                (0, vec![0; 4], 1),
                (4, vec![0; 4], 0),
                (8, vec![8000; 4], 1)
            ]
        );

        let empty = WavPrefix::default();
        let mut entry: WavReceipt = serde_json::from_slice(&receipt(&empty, 12)).unwrap();
        entry.missing_ranges = vec![[4, 8]];
        assert!(empty.prepare(&serde_json::to_vec(&entry).unwrap()).is_ok());
        entry.missing_ranges = vec![[4, 13]];
        assert!(empty.prepare(&serde_json::to_vec(&entry).unwrap()).is_err());
    }

    #[test]
    fn an_empty_peer_does_not_delay_known_pcm_after_a_sequenced_barrier() {
        let mut mixer = LiveMixer::new();
        mixer
            .push(0, 1, 1, &[16000; 4], 2_000_000, 2_250_000)
            .unwrap();
        let mut output = Vec::new();
        mixer
            .drain(2_250_000, &mut |pcm, _, _, _| {
                output.extend_from_slice(pcm);
                Ok(())
            })
            .unwrap();
        assert!(output.is_empty());
        mixer.advance(1, 0, 2_250_000).unwrap();
        mixer
            .drain(2_250_000, &mut |pcm, _, _, _| {
                output.extend_from_slice(pcm);
                Ok(())
            })
            .unwrap();
        assert_eq!(output, [16000; 4]);
        let mut empty = LiveMixer::new();
        empty.advance(0, 0, 10_000_000_000).unwrap();
        empty.advance(1, 0, 10_000_000_000).unwrap();
        empty
            .finish(&mut |_, _, _, _| panic!("no captured PCM must produce no WAV frames"))
            .unwrap();
    }

    #[test]
    fn finishing_a_backlog_keeps_every_packet_and_cross_packet_filter_halo() {
        let input: Vec<i16> = (0..16384)
            .map(|n| ((n * 29) % 28000 - 14000) as i16)
            .collect();
        for step_ns in [62_500, 62_562] {
            let run = |streaming: bool| {
                let mut mixer = LiveMixer::new();
                let mut output = Vec::new();
                for (index, pcm) in input.chunks(4096).enumerate() {
                    let first = index as i64 * 4096;
                    let start = 1_000_000 + first * step_ns;
                    let end = start + pcm.len() as i64 * step_ns;
                    mixer.push(0, 1, index as u64 + 1, pcm, start, end).unwrap();
                    mixer.advance(1, 0, end).unwrap();
                    if streaming {
                        mixer
                            .drain(end, &mut |samples, _, _, mask| {
                                assert_eq!(mask, 1);
                                output.extend_from_slice(samples);
                                Ok(())
                            })
                            .unwrap();
                    }
                }
                mixer
                    .finish(&mut |samples, _, _, mask| {
                        assert_eq!(mask, 1);
                        output.extend_from_slice(samples);
                        Ok(())
                    })
                    .unwrap();
                output
            };
            assert_eq!(run(false), run(true));
            if step_ns == 62_500 {
                assert_eq!(run(false), input);
            }
        }
    }
}
