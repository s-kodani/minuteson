use super::*;
use crate::audio::{tests::caf_fixture, PREVIEW_FRAMES};
use std::cell::Cell;

mod saved_playback;

#[test]
fn canonical_verifier_rejects_valid_flac_from_another_source_and_trailing_data() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, _) = source(root.path(), 8000, 2, 12001);
    let path = root.path().join("owned.part");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let saved =
        encode_segment_file_pending(&mut input, 0, 12001, &mut file, &|| false, &|_| {}).unwrap();
    verify_source(
        &mut file,
        saved.size,
        &saved.sha256,
        &mut input,
        0,
        saved.frames,
        &|| false,
    )
    .unwrap();
    let other = tempfile::tempdir().unwrap();
    let (other_audio, _) = source(other.path(), 8000, 2, 12001);
    drop(other_audio);
    let raw = other.path().join("source.caf");
    let mut bytes = std::fs::read(&raw).unwrap();
    bytes[83..87].copy_from_slice(&0.75_f32.to_le_bytes());
    std::fs::write(&raw, &bytes).unwrap();
    let mut forged = VerifiedAudio::open(
        File::open(&raw).unwrap(),
        bytes.len() as u64,
        &format!("{:x}", Sha256::digest(&bytes)),
    )
    .unwrap();
    assert!(verify_source(
        &mut file,
        saved.size,
        &saved.sha256,
        &mut forged,
        0,
        saved.frames,
        &|| false
    )
    .is_err());
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(b"unexpected tail").unwrap();
    assert!(verify_source(
        &mut file,
        saved.size,
        &saved.sha256,
        &mut input,
        0,
        saved.frames,
        &|| false
    )
    .is_err());
}

#[test]
fn conversion_budget_limits_parallel_work_and_cancels_waiters() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let running = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..6 {
            let running = &running;
            let peak = &peak;
            scope.spawn(move || {
                let _permit = ConversionPermit::acquire(&|| false).unwrap();
                let _nested = ConversionPermit::acquire(&|| false).unwrap();
                let n = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(10));
                running.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    assert!(peak.load(Ordering::SeqCst) <= 2);
    assert!(ConversionPermit::acquire(&|| true).is_err());
}

fn source(root: &Path, rate: u32, channels: u32, frames: usize) -> (VerifiedAudio, Vec<f32>) {
    // Exactly representable signed 24-bit values, including opposite channels.
    let samples: Vec<f32> = (0..frames)
        .flat_map(|i| {
            (0..channels).map(move |c| {
                (((i * 371 + c as usize * 179) % 16000) as i32 - 8000) as f32 / 8192.0
            })
        })
        .collect();
    let pcm: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let bytes = caf_fixture(rate, channels, 3, 32, 4, &pcm);
    let path = root.join("source.caf");
    std::fs::write(&path, &bytes).unwrap();
    (
        VerifiedAudio::open(
            File::open(path).unwrap(),
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap(),
        samples,
    )
}
fn read(path: &Path) -> Result<VerifiedAudio, String> {
    let bytes = std::fs::read(path).unwrap();
    VerifiedAudio::open(
        File::open(path).unwrap(),
        bytes.len() as u64,
        &format!("{:x}", Sha256::digest(&bytes)),
    )
}
fn assert_samples(audio: &mut VerifiedAudio, start: usize, count: usize, expected: &[f32]) {
    let pcm = audio.read_frames(start as u64, count as u64).unwrap();
    let channels = audio.format.channels as usize;
    assert_eq!(pcm.len(), count * channels * 4);
    for (i, sample) in pcm.chunks_exact(4).enumerate() {
        assert_eq!(
            f32::from_le_bytes(sample.try_into().unwrap()),
            expected[start * channels + i],
            "sample {i} from {start}"
        );
    }
}

#[test]
fn flac_round_trip_preserves_all_stereo_samples_and_random_access_across_blocks() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, samples) = source(root.path(), 48000, 2, 20_013);
    let original = std::fs::read(root.path().join("source.caf")).unwrap();
    let path = root.path().join("result.flac");
    let progress = Cell::new(0);
    let saved = write_segment(
        &mut input,
        0,
        20013,
        &path,
        &crate::test_support::never_cancelled,
        &|n| {
            assert!(n > progress.get());
            progress.set(n);
        },
    )
    .unwrap();
    assert_eq!(
        (
            saved.frames,
            saved.sample_rate,
            saved.channels,
            saved.bits_per_sample
        ),
        (20013, 48000, 2, 24)
    );
    assert_eq!(saved.size, std::fs::metadata(&path).unwrap().len());
    assert_eq!(
        saved.sha256,
        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()))
    );
    let mut output = read(&path).unwrap();
    for (start, count) in [
        (0, 20013),
        (4090, 20),
        (20012, 1),
        (17000, 1900),
        (100, 2),
        (4096, 4096),
    ] {
        assert_samples(&mut output, start, count, &samples);
    }
    assert!(output.read_frames(0, PREVIEW_FRAMES + 1).is_err());
    assert!(output.read_frames(20013, 1).is_err());
    assert_eq!(
        original,
        std::fs::read(root.path().join("source.caf")).unwrap()
    );
}

#[test]
fn flac_subrange_short_final_frame_and_24_bit_quantization_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, samples) = source(root.path(), 8000, 1, 4100);
    let path = root.path().join("partial.flac");
    let saved = write_segment(
        &mut input,
        23,
        4077,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    assert_eq!(saved.frames, 4077);
    let mut output = read(&path).unwrap();
    assert_samples(&mut output, 0, 4077, &samples[23..]);
    // Float PCM is not asserted to survive bit-for-bit. At most half a 24-bit
    // quantization step is lost, except +1 maps to the highest signed value.
    let bytes = caf_fixture(
        8000,
        1,
        3,
        32,
        4,
        &[0.123456789_f32, 1.0, -1.0]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let p = root.path().join("float.caf");
    std::fs::write(&p, &bytes).unwrap();
    let mut input = read(&p).unwrap();
    let path = root.path().join("float.flac");
    write_segment(
        &mut input,
        0,
        3,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let mut result = read(&path).unwrap();
    let pcm = result.read_frames(0, 3).unwrap();
    let values: Vec<f32> = pcm
        .chunks_exact(4)
        .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
        .collect();
    assert!((values[0] - 0.123456789).abs() <= 1.0 / 16_777_216.0);
    assert_eq!(values[1], 8_388_607.0 / 8_388_608.0);
    assert_eq!(values[2], -1.0);
}

#[test]
fn cancelled_or_failed_flac_write_before_publish_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, _) = source(root.path(), 8000, 1, 5000);
    let path = root.path().join("cancelled.flac");
    let stop = Cell::new(false);
    assert!(
        write_segment(&mut input, 0, 5000, &path, &|| stop.get(), &|_| stop
            .set(true))
        .is_err()
    );
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    std::fs::write(&path, b"existing user data").unwrap();
    assert!(write_segment(
        &mut input,
        0,
        5000,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"existing user data");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    assert!(write_segment(
        &mut input,
        4999,
        2,
        &root.path().join("invalid.flac"),
        &crate::test_support::never_cancelled,
        &|_| {}
    )
    .is_err());
}

#[test]
fn directory_sync_failure_reports_published_audio_for_recovery() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, samples) = source(root.path(), 8000, 1, 5000);
    let path = root.path().join("published.flac");
    let error = write_segment_with_sync(
        &mut input,
        0,
        5000,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
        &|_| Err("injected directory sync failure".into()),
    )
    .unwrap_err();
    assert!(error.published);
    assert_eq!(error.message, "injected directory sync failure");
    assert_samples(&mut read(&path).unwrap(), 0, 5000, &samples);
    // A retry never replaces the already published file.
    let original = std::fs::read(&path).unwrap();
    let error = write_segment(
        &mut input,
        0,
        5000,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap_err();
    assert!(!error.published);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}

fn prepared(audio: VerifiedAudio) -> crate::playback::PreparedAudio {
    let frames = audio.format.frames;
    crate::playback::PreparedAudio {
        clock: None,
        sample_rate: audio.format.sample_rate,
        channels: audio.format.channels,
        start_us: 0,
        end_us: (frames * 1_000_000 / u64::from(audio.format.sample_rate)) as i64,
        track_kind: "imported".into(),
        alignment: "unverified".into(),
        spans: vec![crate::playback::AudioSpan {
            audio,
            first_frame: 0,
            frames,
        }],
    }
}

#[test]
fn flac_playback_and_both_ai_wav_paths_match_verified_caf() {
    use crate::transcription::wave::{write_track, write_window};
    // Crosses PCM preview, FLAC block, and streamed WAV resampler boundaries.
    let root = tempfile::tempdir().unwrap();
    let frames = 48000 * 2 + 127;
    let (mut input, _) = source(root.path(), 48000, 2, frames);
    let path = root.path().join("managed.flac");
    write_segment(
        &mut input,
        0,
        frames as u64,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let mut caf = prepared(input);
    let mut flac = prepared(read(&path).unwrap());
    for (start, count) in [(65530, 23), (0, 65536), (96000, 127)] {
        assert_eq!(
            caf.read_pcm(start, count).unwrap(),
            flac.read_pcm(start, count).unwrap()
        );
    }
    let caf_window = root.path().join("caf-window.wav");
    let flac_window = root.path().join("flac-window.wav");
    assert_eq!(
        write_window(
            &mut caf,
            4090,
            48000,
            &caf_window,
            &crate::test_support::never_cancelled
        )
        .unwrap(),
        write_window(
            &mut flac,
            4090,
            48000,
            &flac_window,
            &crate::test_support::never_cancelled
        )
        .unwrap(),
    );
    assert_eq!(
        std::fs::read(caf_window).unwrap(),
        std::fs::read(flac_window).unwrap()
    );
    let caf_track = root.path().join("caf-track.wav");
    let flac_track = root.path().join("flac-track.wav");
    assert_eq!(
        write_track(&mut caf, &caf_track, &crate::test_support::never_cancelled).unwrap(),
        write_track(
            &mut flac,
            &flac_track,
            &crate::test_support::never_cancelled
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read(caf_track).unwrap(),
        std::fs::read(flac_track).unwrap()
    );
    let store = crate::playback::PlaybackStore::default();
    let info = store.insert(flac).unwrap();
    assert_eq!(info.track_kind, "imported");
    assert_eq!(info.alignment, "unverified");
    let packet = store.read(&info.token, 65530).unwrap();
    assert_eq!(
        &packet[12..],
        &caf.read_pcm(65530, frames as u64 - 65530).unwrap()
    );
    store.close(&info.token).unwrap();
    assert!(store.read(&info.token, 0).is_err());
}

#[test]
fn flac_validates_sha256_crc_md5_sample_count_and_metadata_bounds() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, _) = source(root.path(), 16000, 2, 9000);
    let path = root.path().join("valid.flac");
    let saved = write_segment(
        &mut input,
        0,
        9000,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    assert!(VerifiedAudio::open(File::open(&path).unwrap(), saved.size, &"0".repeat(64)).is_err());
    let bytes = std::fs::read(&path).unwrap();
    let mutate = |name: &str, alter: fn(&mut Vec<u8>)| {
        let mut broken = bytes.clone();
        alter(&mut broken);
        let p = root.path().join(name);
        std::fs::write(&p, broken).unwrap();
        assert!(read(&p).is_err(), "accepted {name}");
    };
    mutate("md5.flac", |b| b[26] ^= 1);
    mutate("crc.flac", |b| *b.last_mut().unwrap() ^= 1);
    mutate("count.flac", |b| b[25] ^= 1);
    mutate("short.flac", |b| {
        b.truncate(b.len() - 9);
    });
    mutate("metadata.flac", |b| {
        b[5] = 255;
        b[6] = 255;
        b[7] = 255;
    });
    // Large declared duration is rejected before full decoding.
    mutate("duration.flac", |b| {
        b[21] |= 15;
        b[22] = 255;
        b[23] = 255;
        b[24] = 255;
        b[25] = 255;
    });
}

#[test]
fn flac_without_declared_total_is_counted_and_changes_after_verification_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, samples) = source(root.path(), 44100, 1, 8201);
    let path = root.path().join("stream.flac");
    write_segment(
        &mut input,
        0,
        8201,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[21] &= 0xf0;
    bytes[22..26].fill(0);
    std::fs::write(&path, &bytes).unwrap();
    let mut result = read(&path).unwrap();
    assert_eq!(result.format.frames, 8201);
    assert_samples(&mut result, 8000, 201, &samples);
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    (&file).write_all(b"changed").unwrap();
    assert!(result.read_frames(0, 1).is_err());
}

#[test]
fn cancellable_flac_open_does_not_ignore_cancel_during_decoding() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, _) = source(root.path(), 8000, 1, 18000);
    let path = root.path().join("open.flac");
    let saved = write_segment(
        &mut input,
        0,
        18000,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let calls = Cell::new(0);
    let result = VerifiedAudio::open_cancellable(
        File::open(&path).unwrap(),
        saved.size,
        &saved.sha256,
        &|| {
            calls.set(calls.get() + 1);
            calls.get() > 5
        },
    );
    assert!(result.is_err());
    assert!(calls.get() > 5);
}

#[test]
fn unused_metadata_nested_lengths_are_never_given_to_the_decoder() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, samples) = source(root.path(), 8000, 1, 8201);
    let path = root.path().join("metadata.flac");
    write_segment(
        &mut input,
        0,
        8201,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let original = std::fs::read(&path).unwrap();
    for (kind, payload) in [
        // Picture MIME length declares ~4 GiB inside an 8-byte block.
        (6, vec![0, 0, 0, 3, 255, 255, 255, 255]),
        // One comment declares ~4 GiB inside a 12-byte block.
        (4, vec![0, 0, 0, 0, 1, 0, 0, 0, 255, 255, 255, 255]),
        // An invalid base64 embedded picture likewise remains unused.
        (4, {
            let tag = b"METADATA_BLOCK_PICTURE=AAAAA/////8=";
            let mut p = vec![0, 0, 0, 0, 1, 0, 0, 0];
            p.extend_from_slice(&(tag.len() as u32).to_le_bytes());
            p.extend_from_slice(tag);
            p
        }),
    ] {
        let mut bytes = original[..42].to_vec();
        bytes[4] = 0;
        bytes.extend_from_slice(&[0x80 | kind, 0, 0, payload.len() as u8]);
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(&original[42..]);
        std::fs::write(&path, &bytes).unwrap();
        let mut decoded = read(&path).unwrap();
        assert_samples(&mut decoded, 4090, 20, &samples);
        assert_samples(&mut decoded, 0, 8201, &samples);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn independent_rfc_flac_example_decodes_exact_pcm() {
    // RFC 9639 Appendix D.1: independently published verbatim stereo frame,
    // with wasted bits and a one-sample short block. No local encoder used.
    let hex="664c6143800000221000100000000f00000f0ac442f0000000013e84b41807dc690307586a3dad1a2e0ffff869180000bf0358fd03128baa9a";
    let bytes: Vec<u8> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rfc.flac");
    std::fs::write(&path, &bytes).unwrap();
    let mut result = read(&path).unwrap();
    assert_eq!(
        (
            result.format.frames,
            result.format.sample_rate,
            result.format.channels
        ),
        (1, 44100, 2)
    );
    assert_samples(&mut result, 0, 1, &[25588.0 / 32768.0, 10416.0 / 32768.0]);
}

#[test]
#[ignore = "requires an explicitly selected independent ffmpeg executable"]
fn independent_ffmpeg_flac_round_trip() {
    use std::process::Command;
    let executable = std::env::var_os("MON_FLAC_FFMPEG").expect("set MON_FLAC_FFMPEG");
    let root = tempfile::tempdir().unwrap();
    let (mut input, samples) = source(root.path(), 48000, 2, 20013);
    let encoded = root.path().join("native.flac");
    write_segment(
        &mut input,
        0,
        20013,
        &encoded,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let decoded = Command::new(&executable)
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(&encoded)
        .args(["-f", "f32le", "-acodec", "pcm_f32le", "pipe:1"])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert_eq!(
        decoded.stdout,
        samples
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect::<Vec<_>>()
    );
    let external = root.path().join("external.flac");
    let encoded = Command::new(&executable)
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(root.path().join("source.caf"))
        .args(["-c:a", "flac", "-sample_fmt", "s32"])
        .arg(&external)
        .output()
        .unwrap();
    assert!(
        encoded.status.success(),
        "{}",
        String::from_utf8_lossy(&encoded.stderr)
    );
    let mut decoded = read(&external).unwrap();
    assert_samples(&mut decoded, 0, 20013, &samples);
    assert_samples(&mut decoded, 16000, 4013, &samples);
}
