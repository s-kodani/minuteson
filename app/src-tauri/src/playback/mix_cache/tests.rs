use super::*;
use crate::audio::{tests::caf_fixture, VerifiedAudio};
use crate::playback::{AudioSpan, MeetingIntervalAudio, PlaybackMissingRange, PreparedAudio};
use sha2::{Digest, Sha256};

fn mixed() -> MeetingAudio {
    let mut sources = vec![];
    for (kind, rate, value) in [("mic", 48000, 0.25_f32), ("system", 16000, 0.5_f32)] {
        let pcm: Vec<_> = (0..rate)
            .flat_map(|i| (value + (i % 97) as f32 / 1024.0).to_le_bytes())
            .collect();
        let bytes = caf_fixture(rate, 1, 3, 32, 4, &pcm);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&bytes).unwrap();
        let audio = VerifiedAudio::open(
            file,
            bytes.len() as u64,
            &format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        sources.push(PreparedAudio {
            sample_rate: rate,
            channels: 1,
            start_us: 0,
            end_us: 1000000,
            track_kind: kind.into(),
            alignment: "unverified".into(),
            clock: None,
            spans: vec![AudioSpan {
                audio,
                first_frame: 0,
                frames: u64::from(rate),
            }],
        });
    }
    let mut audio = MeetingAudio::new(sources, 1000000).unwrap();
    audio.set_cache_key("fixture").unwrap();
    audio
}

#[test]
fn cached_mix_and_interval_match_live_pcm_and_key_survives_reading() {
    let cache = MixCache::default();
    let mut audio = mixed();
    let key = audio.cache_key.clone();
    let expected = audio.read_pcm(0, 48000).unwrap();
    audio.set_cache_key("fixture").unwrap();
    assert_eq!(key, audio.cache_key);
    cache
        .render(audio, &crate::test_support::never_cancelled)
        .unwrap();
    let cached = cache.get(&key).unwrap();
    assert_eq!(cached.read(0, 48000).unwrap(), expected);
    // Reuse the same sources, as an actual new preparation does.
    let mut audio = mixed();
    audio.cache_key = key.clone();
    audio.use_cache(&cache);
    assert_eq!(
        audio.read_pcm(123, 4567).unwrap(),
        expected[123 * 4..4690 * 4]
    );
    let mut interval = MeetingIntervalAudio::new(audio, 12345, 765432).unwrap();
    interval.use_cache(&cache);
    let first = crate::audio_clock::frame_at(12345, 48000).unwrap() as usize;
    let count = interval.total_frames;
    assert_eq!(
        interval.read_pcm(0, count).unwrap(),
        expected[first * 4..(first + count as usize) * 4]
    );
    assert!(cache.get("different-source").is_none());
}

#[test]
fn incomplete_cancelled_mix_is_not_published_and_gaps_stay_unplayable() {
    let cache = MixCache::default();
    let audio = mixed();
    let key = audio.cache_key.clone();
    assert!(cache.render(audio, &|| true).is_err());
    assert!(cache.get(&key).is_none());
    let mut audio = mixed();
    audio
        .missing_ranges
        .push(PlaybackMissingRange::new("mic", 250000, 500000, "missing", 1000000).unwrap());
    let key = audio.cache_key.clone();
    cache
        .render(audio, &crate::test_support::never_cancelled)
        .unwrap();
    let mut audio = mixed();
    audio.cache_key = key;
    audio
        .missing_ranges
        .push(PlaybackMissingRange::new("mic", 250000, 500000, "missing", 1000000).unwrap());
    audio.use_cache(&cache);
    assert!(audio.read_pcm(12000, 1).is_err());
    assert!(audio.read_pcm(23999, 2).is_err());
    assert!(audio.read_pcm(24000, 1024).is_ok());
}

#[test]
fn maintenance_cancels_and_drains_renderer_and_clears_cached_audio() {
    let cache = MixCache::default();
    let audio = mixed();
    let key = audio.cache_key.clone();
    cache
        .render(audio, &crate::test_support::never_cancelled)
        .unwrap();
    let job = cache.reserve("meeting").unwrap();
    let other = cache.clone();
    let thread = std::thread::spawn(move || other.suspend());
    while !job.cancel.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    assert!(cache.reserve("another").is_none());
    drop(job);
    thread.join().unwrap();
    assert!(cache.get(&key).is_none());
    cache.resume();
    assert!(cache.reserve("another").is_some());
}

#[test]
fn overlapping_maintenance_does_not_resume_another_owners_suspension() {
    let cache = MixCache::default();
    cache.suspend();
    cache.suspend();
    cache.resume();
    assert!(cache.reserve("meeting").is_none());
    cache.resume();
    assert!(cache.reserve("meeting").is_some());
}

#[test]
fn short_meetings_cannot_accumulate_unbounded_open_cache_files() {
    let cache = MixCache::default();
    for index in 0..MAX_ENTRIES + 2 {
        let mut audio = mixed();
        audio.cache_key = format!("meeting-{index}");
        cache
            .render(audio, &crate::test_support::never_cancelled)
            .unwrap();
    }
    assert!(cache.get("meeting-0").is_none());
    assert!(cache.get("meeting-1").is_none());
    assert_eq!(cache.0.state.lock().unwrap().cached.len(), MAX_ENTRIES);
}
