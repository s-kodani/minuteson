use super::*;
use crate::audio::{tests::caf_fixture, VerifiedAudio};
use crate::playback::{AudioSpan, PlaybackStore};
use sha2::{Digest, Sha256};
use std::io::{Seek, SeekFrom, Write};

mod markdown_interval;

fn audio(
    rate: u32,
    channels: u32,
    start_us: i64,
    kind: &str,
    samples: &[f32],
    split: usize,
) -> PreparedAudio {
    let mut spans = Vec::new();
    for segment in samples.chunks(split * channels as usize) {
        let pcm: Vec<u8> = segment
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let bytes = caf_fixture(rate, channels, 3, 32, 4, &pcm);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        let audio = VerifiedAudio::open(
            file,
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        let frames = audio.format.frames;
        spans.push(AudioSpan {
            audio,
            first_frame: 0,
            frames,
        });
    }
    PreparedAudio {
        clock: None,
        sample_rate: rate,
        channels,
        start_us,
        end_us: start_us
            + (samples.len() as i64 / i64::from(channels) * 1_000_000 / i64::from(rate)),
        track_kind: kind.into(),
        alignment: "verified".into(),
        spans,
    }
}

fn values(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()))
        .collect()
}

#[test]
fn formats_downmix_to_fixed_mono_rate_and_preserve_dc() {
    for rate in [8_000, 16_000, 44_100, 48_000, 96_000, 384_000] {
        for channels in [1, 2, 8] {
            let samples: Vec<f32> = (0..rate / 10)
                .flat_map(|_| (0..channels).map(|channel| (channel + 1) as f32 / 16.0))
                .collect();
            let source = audio(rate, channels, 0, "imported", &samples, rate as usize);
            let mut meeting = MeetingAudio::new(vec![source], 100_000).unwrap();
            assert_eq!(meeting.total_frames, 4800);
            let expected = (channels + 1) as f32 / 32.0;
            assert!(
                values(&meeting.read_pcm(0, 4800).unwrap())
                    .iter()
                    .all(|sample| (*sample - expected).abs() < 1e-6),
                "{rate} / {channels}"
            );
        }
    }
}

#[test]
fn both_mix_keeps_headroom_and_pads_only_known_source_extents() {
    let mic = audio(48_000, 1, 0, "mic", &vec![0.25; 24_000], 12_000);
    let system = audio(16_000, 2, 250_000, "system", &vec![0.75; 16_000], 8000);
    let mut meeting = MeetingAudio::new(vec![mic, system], 1_000_000).unwrap();
    assert_eq!(meeting.track_kind, "both");
    let output = values(&meeting.read_pcm(0, 48_000).unwrap());
    for (range, expected) in [
        (0..12_000, 0.125),
        (12_000..24_000, 0.5),
        (24_000..36_000, 0.375),
        (36_000..48_000, 0.0),
    ] {
        assert!(output[range]
            .iter()
            .all(|sample| (*sample - expected).abs() < 1e-6));
    }
}

#[test]
fn fractional_origin_segments_packets_and_seeks_use_the_same_phase() {
    let rate = 44_100;
    let samples: Vec<f32> = (0..rate * 2)
        .map(|i| (i as f64 * 2.0 * std::f64::consts::PI * 1000.0 / rate as f64).sin() as f32 * 0.5)
        .collect();
    let source = audio(rate, 1, 7, "system", &samples, 25_777);
    let end = source.end_us;
    let mut meeting = MeetingAudio::new(vec![source], end).unwrap();
    let mut full = meeting.read_pcm(0, PREVIEW_FRAMES).unwrap();
    full.extend(
        meeting
            .read_pcm(PREVIEW_FRAMES, meeting.total_frames - PREVIEW_FRAMES)
            .unwrap(),
    );
    assert_eq!(&full[..4], &0_f32.to_le_bytes());
    for (offset, count) in [
        (1, 100),
        (28_000, 1000),
        (PREVIEW_FRAMES - 127, 254),
        (meeting.total_frames - 100, 100),
    ] {
        assert_eq!(
            meeting.read_pcm(offset, count).unwrap(),
            full[offset as usize * 4..(offset + count) as usize * 4]
        );
    }
    // Independently check the expected 1 kHz waveform/time origin away from edges.
    for i in 1000..2000 {
        let expected =
            ((i as f64 / 48_000.0 - 7e-6) * 2.0 * std::f64::consts::PI * 1000.0).sin() * 0.5;
        let actual = f32::from_le_bytes(full[i * 4..i * 4 + 4].try_into().unwrap());
        assert!((f64::from(actual) - expected).abs() < 0.001);
    }
}

#[test]
fn downsampling_preserves_speech_band_and_rejects_aliases() {
    let measure = |frequency: f64| {
        let samples: Vec<f32> = (0..96_000)
            .map(|i| {
                (i as f64 * 2.0 * std::f64::consts::PI * frequency / 96_000.0).sin() as f32 * 0.5
            })
            .collect();
        let source = audio(96_000, 1, 0, "system", &samples, 40_000);
        let mut meeting = MeetingAudio::new(vec![source], 1_000_000).unwrap();
        let output = values(&meeting.read_pcm(1000, 40_000).unwrap());
        (output
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>()
            / output.len() as f64)
            .sqrt()
    };
    assert!(measure(1000.0) > 0.34);
    assert!(measure(30_000.0) < 0.002);
}

#[test]
fn maximum_rate_read_spans_multiple_bounded_native_reads() {
    let source = audio(384_000, 1, 0, "system", &vec![0.25; 384_000], 100_000);
    let mut meeting = MeetingAudio::new(vec![source], 1_000_000).unwrap();
    assert!(values(&meeting.read_pcm(0, 48_000).unwrap())
        .iter()
        .all(|sample| (*sample - 0.25).abs() < 1e-6));
}

#[test]
fn invalid_combinations_metadata_and_spans_are_rejected() {
    let fixture = || audio(48_000, 1, 0, "mic", &vec![0.25; 4800], 4800);
    assert!(MeetingAudio::new(vec![], 100_000).is_err());
    assert!(MeetingAudio::new(vec![fixture(), fixture()], 100_000).is_err());
    assert!(MeetingAudio::new(vec![fixture(), fixture(), fixture()], 100_000).is_err());
    for case in 0..10 {
        let mut source = fixture();
        match case {
            0 => source.sample_rate = 0,
            1 => source.channels = 9,
            2 => source.start_us = -1,
            3 => source.end_us = 0,
            4 => source.end_us = 200_000,
            5 => source.spans[0].first_frame = u64::MAX,
            6 => source.spans[0].frames = 0,
            7 => source.spans[0].frames = 100,
            8 => source.track_kind = "foreign".into(),
            _ => source.spans.clear(),
        }
        assert!(
            MeetingAudio::new(vec![source], 100_000).is_err(),
            "case {case}"
        );
    }
    assert!(output_frame(i64::MAX).is_err());
}

#[test]
fn frame_rounding_and_large_timeline_do_not_require_full_recording_allocation() {
    let start = 14_399_000_001;
    let source = audio(48_000, 1, start, "mic", &vec![0.25; 4800], 4800);
    let mut meeting = MeetingAudio::new(vec![source], 14_400_000_000).unwrap();
    assert_eq!(meeting.total_frames, 691_200_000);
    assert_eq!(meeting.read_pcm(0, 10).unwrap(), vec![0; 40]);
    assert_eq!(meeting.read_pcm(691_199_990, 10).unwrap(), vec![0; 40]);
    assert!(meeting.read_pcm(0, 0).is_err());
    assert!(meeting.read_pcm(0, PREVIEW_FRAMES + 1).is_err());
    assert!(meeting.read_pcm(u64::MAX, 1).is_err());
    assert!(meeting.read_pcm(meeting.total_frames, 1).is_err());
}

#[test]
fn meeting_tokens_share_packet_contract_eviction_and_release() {
    let store = PlaybackStore::default();
    let fixture = || audio(48_000, 1, 0, "mic", &vec![0.25; 48_000], 48_000);
    let old = store.insert(fixture()).unwrap();
    let info = store
        .insert_meeting(MeetingAudio::new(vec![fixture()], 2_000_000).unwrap())
        .unwrap();
    assert_eq!(info.sample_rate, 48_000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.total_frames, 96_000);
    assert_eq!(info.start_us, 0);
    assert_eq!(info.end_us, 2_000_000);
    assert_eq!(info.alignment, "unverified");
    let packet = store.read(&info.token, 95_990).unwrap();
    assert_eq!(
        &packet[..12],
        &[
            48_000_u32.to_le_bytes(),
            1_u32.to_le_bytes(),
            10_u32.to_le_bytes()
        ]
        .concat()
    );
    assert_eq!(packet.len(), 52);
    assert!(packet[12..].iter().all(|byte| *byte == 0));
    assert!(store.read(&info.token, info.total_frames).is_err());
    for _ in 0..3 {
        store.insert(fixture()).unwrap();
    }
    assert!(store.read(&old.token, 0).is_err());
    assert!(store.read(&info.token, 0).is_ok());
    store.close(&info.token).unwrap();
    store.close(&info.token).unwrap();
    assert!(store.read(&info.token, 0).is_err());
}

#[test]
fn prepared_source_mutation_is_not_hidden_by_resampling() {
    let samples = vec![0.25; 8000];
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|sample: &f32| sample.to_le_bytes())
        .collect();
    let bytes = caf_fixture(8000, 1, 3, 32, 4, &pcm);
    let mut writer = tempfile::tempfile().unwrap();
    writer.write_all(&bytes).unwrap();
    let reader = writer.try_clone().unwrap();
    let verified = VerifiedAudio::open(
        reader,
        bytes.len() as u64,
        &format!("{:x}", Sha256::digest(&bytes)),
    )
    .unwrap();
    let source = PreparedAudio {
        clock: None,
        sample_rate: 8000,
        channels: 1,
        start_us: 0,
        end_us: 1_000_000,
        track_kind: "mic".into(),
        alignment: "unverified".into(),
        spans: vec![AudioSpan {
            audio: verified,
            first_frame: 0,
            frames: 8000,
        }],
    };
    let mut meeting = MeetingAudio::new(vec![source], 1_000_000).unwrap();
    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(&[0; 4]).unwrap();
    assert!(meeting.read_pcm(0, 100).is_err());
}
