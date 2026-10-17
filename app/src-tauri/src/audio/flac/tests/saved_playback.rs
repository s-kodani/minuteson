use super::*;

fn saved(path: &Path, size: u64, hash: &str) -> Result<VerifiedAudio, String> {
    VerifiedAudio::open_saved_cancellable(
        File::open(path).unwrap(),
        size,
        hash,
        &crate::test_support::never_cancelled,
    )
}

#[test]
fn saved_flac_seeks_match_full_validation_including_short_final_block() {
    let root = tempfile::tempdir().unwrap();
    for (rate, channels) in [(8000, 1), (44100, 2), (48000, 1)] {
        let (mut input, samples) = source(root.path(), rate, channels, 70013);
        let path = root.path().join(format!("{rate}.flac"));
        let info = write_segment(
            &mut input,
            0,
            70013,
            &path,
            &crate::test_support::never_cancelled,
            &|_| {},
        )
        .unwrap();
        let mut audio = saved(&path, info.size, &info.sha256).unwrap();
        for (first, count) in [(69999, 14), (4090, 20), (0, 65536), (65530, 4483)] {
            assert_samples(&mut audio, first, count, &samples);
        }
    }
}

#[test]
fn tiny_saved_flac_can_rewind_after_eof_for_overlapping_resampling_reads() {
    let root = tempfile::tempdir().unwrap();
    let pcm: Vec<_> = (0..4410).flat_map(|_| 0.5_f32.to_le_bytes()).collect();
    let bytes = caf_fixture(44100, 1, 3, 32, 4, &pcm);
    let caf = root.path().join("constant.caf");
    std::fs::write(&caf, &bytes).unwrap();
    let mut input = read(&caf).unwrap();
    let path = root.path().join("tiny.flac");
    let info = write_segment(
        &mut input,
        0,
        4410,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let open = || saved(&path, info.size, &info.sha256).unwrap();
    let store = crate::playback::PlaybackStore::default();
    let audio = crate::playback::MeetingAudio::new(vec![prepared(open())], 100000).unwrap();
    let full = store.insert_meeting(audio).unwrap();
    let initial = store.read(&full.token, 0).unwrap();
    for first in [593, 0, 3000, 593] {
        let again = store.read(&full.token, first).unwrap();
        assert_eq!(&again[12..], &initial[12 + first as usize * 4..]);
    }
    let source = crate::playback::MeetingAudio::new(vec![prepared(open())], 100000).unwrap();
    let interval = crate::playback::MeetingIntervalAudio::new(source, 12345, 87654).unwrap();
    let sliced = store.insert_interval(interval).unwrap();
    let first = crate::audio_clock::frame_at(12345, 48000).unwrap() as usize;
    assert_eq!(
        &store.read(&sliced.token, 0).unwrap()[12..],
        &initial[12 + first * 4..12 + (first + sliced.total_frames as usize) * 4]
    );
}

#[test]
fn saved_flac_checks_packet_crc_size_and_later_mutation() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, _) = source(root.path(), 48000, 1, 9000);
    let path = root.path().join("saved.flac");
    let info = write_segment(
        &mut input,
        0,
        9000,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let mut audio = saved(&path, info.size, &info.sha256).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(audio.read_frames(0, 1).is_err());
    // An unread damaged packet is detected when reached, not by a full scan
    // before playback. Source publication and AI retain whole-file validation.
    let mut reopened = saved(&path, info.size, &info.sha256).unwrap();
    assert!(reopened.read_frames(8999, 1).is_err());
    assert!(VerifiedAudio::open(File::open(&path).unwrap(), info.size, &info.sha256).is_err());
    assert!(saved(&path, info.size + 1, &info.sha256).is_err());
}

#[test]
fn saved_flac_header_open_can_be_cancelled() {
    let root = tempfile::tempdir().unwrap();
    let (mut input, _) = source(root.path(), 48000, 1, 9000);
    let path = root.path().join("saved.flac");
    let info = write_segment(
        &mut input,
        0,
        9000,
        &path,
        &crate::test_support::never_cancelled,
        &|_| {},
    )
    .unwrap();
    let calls = Cell::new(0);
    let result = VerifiedAudio::open_saved_cancellable(
        File::open(&path).unwrap(),
        info.size,
        &info.sha256,
        &|| {
            calls.set(calls.get() + 1);
            calls.get() >= 2
        },
    );
    assert!(result.err().unwrap().contains("中断"));
    assert!(saved(&path, info.size, &info.sha256).is_ok());
}

// Generate an independent long FLAC outside the test, then pass its path as
// MON_PLAYBACK_BENCH_FLAC. No audio output or application data writes.
#[test]
#[ignore]
fn long_saved_flac_preparation_benchmark() {
    let path = std::path::PathBuf::from(std::env::var_os("MON_PLAYBACK_BENCH_FLAC").unwrap());
    let bytes = std::fs::read(&path).unwrap();
    let size = bytes.len() as u64;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    drop(bytes);
    let start = std::time::Instant::now();
    let mut full = VerifiedAudio::open(File::open(&path).unwrap(), size, &hash).unwrap();
    let full_time = start.elapsed();
    let start = std::time::Instant::now();
    let mut indexed = saved(&path, size, &hash).unwrap();
    let indexed_time = start.elapsed();
    assert_eq!(full.format.frames, indexed.format.frames);
    for first in [0, indexed.format.frames / 2, indexed.format.frames - 48000] {
        let start = std::time::Instant::now();
        let pcm = indexed.read_frames(first, 48000).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(pcm, full.read_frames(first, 48000).unwrap());
        eprintln!("seek frame={first}: {elapsed:?}");
    }
    eprintln!(
        "frames={} rate={} channels={} bytes={size} full={full_time:?} saved={indexed_time:?}",
        indexed.format.frames, indexed.format.sample_rate, indexed.format.channels
    );
}
