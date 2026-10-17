use super::*;
use crate::audio::{tests::caf_fixture, VerifiedAudio};
use crate::audio_clock::{tests::segment, ClockChunk, ClockSegment};
use crate::playback::{MeetingAudio, PreparedAudio};
use sha2::{Digest, Sha256};
use std::io::Write;

fn audio(
    rate: u32,
    channels: u32,
    samples: &[f32],
    maps: &[ClockSegment],
    split: usize,
) -> PreparedAudio {
    let mut spans = Vec::new();
    for block in samples.chunks(split * channels as usize) {
        let pcm: Vec<u8> = block
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let bytes = caf_fixture(rate, channels, 3, 32, 4, &pcm);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        let verified = VerifiedAudio::open(
            file,
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        spans.push(AudioSpan {
            frames: verified.format.frames,
            first_frame: 0,
            audio: verified,
        });
    }
    let start_us = maps[0].timeline_start_us;
    let end_us = maps.last().unwrap().timeline_end_us;
    let layout = ClockLayout::new(
        &[ClockChunk {
            frames: (samples.len() / channels as usize) as i64,
            start_us,
            end_us,
            segments: maps,
        }],
        rate,
    )
    .unwrap();
    PreparedAudio {
        sample_rate: rate,
        channels,
        start_us,
        end_us,
        track_kind: "imported".into(),
        alignment: "unverified".into(),
        spans,
        clock: Some(ClockAudio::new(layout, start_us, end_us, rate).unwrap()),
    }
}

fn values(pcm: &[u8]) -> Vec<f32> {
    pcm.chunks_exact(4)
        .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()))
        .collect()
}

fn collect(source: &mut PreparedAudio) -> Vec<u8> {
    let mut pcm = Vec::new();
    let mut first = 0;
    while first < source.total_frames() {
        let count = PREVIEW_FRAMES.min(source.total_frames() - first);
        pcm.extend(source.read_pcm(first, count).unwrap());
        first += count;
    }
    pcm
}

#[test]
fn identity_map_is_sample_exact_across_channels_and_physical_file_boundaries() {
    let samples: Vec<f32> = (0..72000)
        .flat_map(|index| [(index as f32 / 71.0).sin() * 0.5, 0.25])
        .collect();
    let maps = [segment(0, 72000, 250000, 1750000)];
    let mut source = audio(48000, 2, &samples, &maps, 17003);
    assert_eq!(source.total_frames(), 72000);
    assert_eq!(
        collect(&mut source),
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>()
    );
}

#[test]
fn drift_and_piecewise_slopes_keep_packet_seek_and_requested_slice_identical() {
    let samples: Vec<f32> = (0..96000)
        .map(|index| (index as f32 / 57.0).sin() * 0.5)
        .collect();
    let maps = [
        segment(0, 48000, 0, 1100000),
        segment(48000, 96000, 1100000, 2000000),
    ];
    let mut source = audio(48000, 1, &samples, &maps, 17003);
    let full = collect(&mut source);
    for first in [0, 1, 1023, 1024, 16999, 52799, 52800, 65535, 65536, 95999] {
        let count = 97.min(source.total_frames() - first);
        assert_eq!(
            source.read_pcm(first, count).unwrap(),
            full[first as usize * 4..(first + count) as usize * 4],
            "{first}"
        );
    }
    let layout = source.clock.as_ref().unwrap().layout.clone();
    source.start_us = 123457;
    source.end_us = 1900013;
    source.clock = Some(ClockAudio::new(layout, source.start_us, source.end_us, 48000).unwrap());
    let first = frame_at(source.start_us, 48000).unwrap();
    let last = frame_at(source.end_us, 48000).unwrap();
    assert_eq!(
        collect(&mut source),
        full[first as usize * 4..last as usize * 4]
    );
}

#[test]
fn affine_compression_filters_aliases_without_changing_dc_or_channel_order() {
    let samples: Vec<f32> = (0..48000)
        .flat_map(|index| [if index % 2 == 0 { 0.75 } else { -0.75 }, 0.25])
        .collect();
    let mut source = audio(48000, 2, &samples, &[segment(0, 48000, 0, 500000)], 9001);
    assert_eq!(source.total_frames(), 24000);
    let pcm = values(&collect(&mut source));
    for frame in pcm.chunks_exact(2).skip(100).take(23800) {
        assert!(frame[0].abs() < 0.005, "{}", frame[0]);
        assert!((frame[1] - 0.25).abs() < 1e-6);
    }
}

#[test]
fn interpolated_full_scale_pcm_stays_within_the_strict_preview_packet_contract() {
    let samples: Vec<f32> = (0..48000)
        .map(|index| if index < 24000 { -1.0 } else { 1.0 })
        .collect();
    let mut source = audio(48000, 1, &samples, &[segment(0, 48000, 0, 1100000)], 9001);
    let pcm = values(&collect(&mut source));
    assert!(pcm
        .iter()
        .all(|sample| sample.is_finite() && sample.abs() <= 1.0));
    assert_eq!(pcm[0], -1.0);
    assert_eq!(*pcm.last().unwrap(), 1.0);
}

#[test]
fn meeting_reads_mapped_audio_directly_on_48k_grid_without_native_phase_rounding() {
    let samples: Vec<f32> = (0..16000)
        .flat_map(|index| [(index as f32 / 23.0).sin() * 0.4, 0.25])
        .collect();
    let maps = [segment(0, 16000, 123457, 1234567)];
    let mut source = audio(16000, 2, &samples, &maps, 7001);
    let first = frame_at(source.start_us, 48000).unwrap();
    let last = frame_at(source.end_us, 48000).unwrap();
    let expected = source
        .clock
        .as_mut()
        .unwrap()
        .read(&mut source.spans, 2, 48000, first, last - first)
        .unwrap();
    let mut meeting = MeetingAudio::new(vec![source], 1234567).unwrap();
    let output = values(&meeting.read_pcm(first, last - first).unwrap());
    for (frame, value) in values(&expected).chunks_exact(2).zip(output) {
        assert_eq!(
            (f64::from(frame[0]) * 0.5 + f64::from(frame[1]) * 0.5) as f32,
            value
        );
    }
    let silence = values(&meeting.read_pcm(0, first).unwrap());
    assert!(silence.iter().all(|value| *value == 0.0));
}

#[test]
fn mapped_pcm_is_shared_by_streamed_track_and_asr_window_wav_writers() {
    use crate::transcription::wave::{write_track, write_window};
    let samples: Vec<f32> = (0..96000)
        .map(|index| (index as f32 / 61.0).sin() * 0.5)
        .collect();
    let mut source = audio(48000, 1, &samples, &[segment(0, 96000, 0, 2100000)], 22001);
    let root = tempfile::tempdir().unwrap();
    let track = root.path().join("track.wav");
    let window = root.path().join("window.wav");
    let frames = source.total_frames();
    assert_eq!(
        write_track(&mut source, &track, &crate::test_support::never_cancelled).unwrap(),
        write_window(
            &mut source,
            0,
            frames,
            &window,
            &crate::test_support::never_cancelled
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read(track).unwrap(),
        std::fs::read(window).unwrap()
    );
}

#[test]
fn extreme_admitted_slope_has_bounded_input_halo_and_works_at_meeting_rate() {
    let samples = vec![0.25; 384000];
    let source = audio(384000, 1, &samples, &[segment(0, 384000, 0, 15625)], 60000);
    let mut meeting = MeetingAudio::new(vec![source], 15625).unwrap();
    assert_eq!(meeting.total_frames, 750);
    assert!(values(&meeting.read_pcm(0, 750).unwrap())
        .iter()
        .all(|sample| (*sample - 0.25).abs() < 1e-6));
}

#[test]
fn mapped_reads_preserve_range_and_physical_file_integrity_rejections() {
    let mut source = audio(
        48000,
        1,
        &vec![0.25; 48000],
        &[segment(0, 48000, 0, 1100000)],
        12000,
    );
    assert!(source.read_pcm(source.total_frames(), 1).is_err());
    assert!(source.read_pcm(0, PREVIEW_FRAMES + 1).is_err());
    assert!(source.read_pcm(0, 0).is_err());
    source.spans[0].frames -= 1;
    assert!(source.read_pcm(source.total_frames() - 1, 1).is_err());
}

#[test]
fn gap_free_slices_clamp_filter_halo_to_their_run_and_never_blend_across_missing_time() {
    let samples: Vec<f32> = (0..48000)
        .map(|index| if index < 24000 { 0.25 } else { -0.5 })
        .collect();
    let mut source = audio(48000, 1, &samples, &[segment(0, 48000, 0, 1100000)], 12000);
    let left = [segment(0, 24000, 0, 550000)];
    let right = [segment(0, 24000, 650000, 1200000)];
    let layout = ClockLayout::new(
        &[
            ClockChunk {
                frames: 24000,
                start_us: 0,
                end_us: 550000,
                segments: &left,
            },
            ClockChunk {
                frames: 24000,
                start_us: 650000,
                end_us: 1200000,
                segments: &right,
            },
        ],
        48000,
    )
    .unwrap();
    let single_chunk = ClockLayout::new(
        &[ClockChunk {
            frames: 48000,
            start_us: 0,
            end_us: 1200000,
            segments: &[
                segment(0, 24000, 0, 550000),
                segment(24000, 48000, 650000, 1200000),
            ],
        }],
        48000,
    )
    .unwrap();
    assert!(single_chunk.range(0, 1200000, 48000).is_err());
    assert_eq!(
        single_chunk.range(650000, 1200000, 48000).unwrap(),
        layout.range(650000, 1200000, 48000).unwrap()
    );
    assert!(ClockAudio::new(layout.clone(), 0, 1200000, 48000).is_err());
    source.end_us = 550000;
    source.clock = Some(ClockAudio::new(layout.clone(), 0, 550000, 48000).unwrap());
    assert!(values(&collect(&mut source))
        .iter()
        .all(|value| (*value - 0.25).abs() < 1e-6));
    assert!(source
        .clock
        .as_mut()
        .unwrap()
        .read(&mut source.spans, 1, 48000, 25000, 8000)
        .is_err());
    source.start_us = 650000;
    source.end_us = 1200000;
    source.clock = Some(ClockAudio::new(layout, 650000, 1200000, 48000).unwrap());
    assert!(values(&collect(&mut source))
        .iter()
        .all(|value| (*value + 0.5).abs() < 1e-6));
    source.clock = Some(ClockAudio::new(single_chunk, 650000, 1200000, 48000).unwrap());
    assert!(values(&collect(&mut source))
        .iter()
        .all(|value| (*value + 0.5).abs() < 1e-6));
}
