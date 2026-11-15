//! Verified logical spans with a bounded number of physical readers. Deferred
//! files are fully checked before preparation and again whenever reopened.
use crate::audio::{PcmFormat, VerifiedAudio};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct SpanCache(Arc<Mutex<VecDeque<(PathBuf, VerifiedAudio)>>>);

pub struct SpanAudio {
    pub format: PcmFormat,
    source: Source,
}
enum Source {
    Open(VerifiedAudio),
    Deferred { proof: Proof, cache: SpanCache },
}
struct Proof {
    path: PathBuf,
    identity: String,
    size: u64,
    hash: String,
}
impl From<VerifiedAudio> for SpanAudio {
    fn from(audio: VerifiedAudio) -> Self {
        Self {
            format: audio.format.clone(),
            source: Source::Open(audio),
        }
    }
}
impl Proof {
    fn open(&self) -> Result<File, String> {
        if self
            .path
            .canonicalize()
            .map_err(|error| error.to_string())?
            != self.path
        {
            return Err("保存音声の管理パスが変更されました".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000).share_mode(7);
        }
        let file = options
            .open(&self.path)
            .map_err(|error| error.to_string())?;
        self.require(&file)?;
        Ok(file)
    }
    fn require(&self, file: &File) -> Result<(), String> {
        let metadata = crate::native_fs::metadata(file).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.len() != self.size
            || crate::audio::reuse::stamp(&metadata) != self.identity
        {
            return Err("検証済み音声の実体・内容が変更されました".into());
        }
        Ok(())
    }
}
impl SpanAudio {
    pub(crate) fn deferred(
        path: PathBuf,
        audio: VerifiedAudio,
        size: u64,
        hash: &str,
        cache: SpanCache,
    ) -> Result<Self, String> {
        let proof = Proof {
            path,
            identity: audio.identity_stamp()?,
            size,
            hash: hash.into(),
        };
        proof.open()?;
        crate::audio::reuse::forget(&audio, hash);
        Ok(Self {
            format: audio.format.clone(),
            source: Source::Deferred { proof, cache },
        })
    }
    pub(crate) fn check_unchanged(&self) -> Result<(), String> {
        match &self.source {
            Source::Open(audio) => audio.check_unchanged(),
            Source::Deferred { proof, cache } => {
                proof.open()?;
                let cached = cache
                    .0
                    .lock()
                    .map_err(|_| "音声読出しロックを取得できません")?;
                if let Some((_, audio)) = cached.iter().find(|(path, _)| path == &proof.path) {
                    audio.check_unchanged()?;
                }
                Ok(())
            }
        }
    }
    pub(crate) fn read_frames(&mut self, first: u64, frames: u64) -> Result<Vec<u8>, String> {
        match &mut self.source {
            Source::Open(audio) => audio.read_frames(first, frames),
            Source::Deferred { proof, cache } => {
                proof.open()?;
                let mut cached = cache
                    .0
                    .lock()
                    .map_err(|_| "音声読出しロックを取得できません")?;
                let index =
                    if let Some(index) = cached.iter().position(|(path, _)| path == &proof.path) {
                        index
                    } else {
                        // Evict before opening, including the FLAC decoder's own
                        // descriptor copies. Adjacent-chunk filter halos use two.
                        if cached.len() == 2 {
                            cached.pop_front();
                        }
                        let audio = VerifiedAudio::open_cancellable(
                            proof.open()?,
                            proof.size,
                            &proof.hash,
                            &|| false,
                        )?;
                        if audio.identity_stamp()? != proof.identity {
                            return Err("音声の再読込み中に実体が変更されました".into());
                        }
                        crate::audio::reuse::forget(&audio, &proof.hash);
                        cached.push_back((proof.path.clone(), audio));
                        cached.len() - 1
                    };
                let result = cached[index].1.read_frames(first, frames)?;
                proof.open()?;
                Ok(result)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn many_spans(count: usize) -> (tempfile::TempDir, Vec<SpanAudio>, SpanCache) {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().canonicalize().unwrap();
        let cache = SpanCache::default();
        let mut spans = Vec::new();
        let _verification = crate::audio::reuse::Scope::join_or_enter().unwrap();
        for index in 0..count {
            let path = directory.join(format!("{index}.caf"));
            let mut bytes = mon_core::media::caf::voice_header(8).unwrap().to_vec();
            for _ in 0..8 {
                bytes.extend((index as i16).to_le_bytes());
            }
            std::fs::write(&path, &bytes).unwrap();
            let hash = format!("{:x}", Sha256::digest(&bytes));
            let audio =
                VerifiedAudio::open(File::open(&path).unwrap(), bytes.len() as u64, &hash).unwrap();
            spans.push(
                SpanAudio::deferred(path, audio, bytes.len() as u64, &hash, cache.clone()).unwrap(),
            );
        }
        assert!(cache.0.lock().unwrap().is_empty());
        (root, spans, cache)
    }

    fn exercise_many_spans(count: usize) {
        let (_root, mut spans, cache) = many_spans(count);
        for (index, span) in spans.iter_mut().enumerate() {
            let pcm = span.read_frames(0, 8).unwrap();
            assert_eq!(pcm.len(), 8 * 4);
            assert_eq!(
                f32::from_le_bytes(pcm[..4].try_into().unwrap()),
                index as f32 / 32768.
            );
            assert!(cache.0.lock().unwrap().len() <= 2);
        }
        for span in &spans {
            span.check_unchanged().unwrap();
        }
        assert_eq!(spans[0].read_frames(0, 1).unwrap(), 0_f32.to_le_bytes());
    }

    #[test]
    fn deferred_thousand_chunk_source_keeps_only_two_physical_readers() {
        exercise_many_spans(1024);
    }

    #[test]
    fn deferred_source_rejects_replacement_and_changes_to_cached_audio() {
        let (root, mut spans, _) = many_spans(4);
        spans[3].read_frames(0, 1).unwrap();
        let directory = root.path().canonicalize().unwrap();
        let replacement = directory.join("replacement.caf");
        std::fs::copy(directory.join("0.caf"), &replacement).unwrap();
        std::fs::rename(replacement, directory.join("0.caf")).unwrap();
        assert!(spans[0].check_unchanged().is_err());
        assert!(spans[0].read_frames(0, 1).is_err());
        let path = directory.join("3.caf");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[68] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        // Rapid same-size writes can share a Windows clock tick. Make this
        // fixture's completed mutation observable without a timing-based sleep.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(modified - std::time::Duration::from_secs(2)),
            )
            .unwrap();
        assert!(spans[3].read_frames(0, 1).is_err());
        assert!(spans[3].check_unchanged().is_err());
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "Owned low-FD child; only the dedicated parent test may launch it"]
    fn low_descriptor_child() {
        assert_eq!(
            std::env::var("MON_SPAN_LOW_FD_CHILD").unwrap(),
            "synthetic-only"
        );
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0
        );
        limit.rlim_cur = limit.rlim_cur.min(64);
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
        exercise_many_spans(1024);
    }

    #[cfg(unix)]
    #[test]
    fn thousand_chunks_work_in_owned_process_with_64_descriptor_limit() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "playback::span::tests::low_descriptor_child",
                "--ignored",
                "--nocapture",
            ])
            .env("MON_SPAN_LOW_FD_CHILD", "synthetic-only")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
