//! Job-local verified audio headers and FLAC packet indexes; never PCM or persisted data.
use super::*;
use std::cell::RefCell;
use std::collections::HashMap;
thread_local! {static CACHE: RefCell<Option<Cache>>=const {RefCell::new(None)};}
pub(crate) fn release_all() {
    CACHE.with(|cache| {if let Some(cache)=cache.borrow_mut().as_mut() {cache.entries.clear();cache.bytes=0;}});
}
#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    bytes: usize,
    hits: usize,
    misses: usize,
}
#[derive(Clone)]
struct Entry {
    format: PcmFormat,
    index: Option<flac::VerifiedIndex>,
    audit: std::sync::Arc<File>,
    identity: String,
}
pub(crate) struct Scope {
    owns: bool,
}
impl Scope {
    pub(crate) fn enter() -> Result<Self, String> {
        CACHE.with(|c| {
            if c.borrow().is_some() {
                return Err("音声検証セッションが重複しています".into());
            }
            *c.borrow_mut() = Some(Cache::default());
            Ok(Self { owns: true })
        })
    }
    pub(crate) fn join_or_enter() -> Result<Self, String> {
        if CACHE.with(|c| c.borrow().is_some()) {
            Ok(Self { owns: false })
        } else {
            Self::enter()
        }
    }
    pub(crate) fn verify(&self) -> Result<(), String> {
        CACHE.with(|c| {
            if let Some(cache) = c.borrow().as_ref() {
                for e in cache.entries.values() {
                    if stamp(&crate::native_fs::metadata(&e.audit).map_err(|e| e.to_string())?)
                        != e.identity
                    {
                        return Err("処理中に検証済み音声が変更されました".into());
                    }
                }
            }
            Ok(())
        })
    }
    pub(crate) fn counts(&self) -> (usize, usize) {
        CACHE.with(|c| c.borrow().as_ref().map_or((0, 0), |v| (v.hits, v.misses)))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if !self.owns {
            return;
        }
        CACHE.with(|c| {
            c.borrow_mut().take();
        });
    }
}
pub(crate) fn stamp(m: &Metadata) -> String {
    #[cfg(any(unix, windows))]
    {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec()
        )
    }
    #[cfg(not(any(unix, windows)))]
    {
        format!("{}:{:?}:{:?}", m.len(), m.modified(), m.created())
    }
}
pub(super) fn open(
    file: &File,
    size: u64,
    hash: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<VerifiedAudio>, String> {
    if cancelled() {
        return Err("音声ファイルの照合を中断しました".into());
    }
    let metadata = crate::native_fs::metadata(&file).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() != size {
        return Err("音声ファイルのサイズが保存時と一致しません".into());
    }
    let identity = stamp(&metadata);
    let key = format!("{hash}:{identity}");
    let entry = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let cache = c.as_mut()?;
        let entry = cache.entries.get(&key).cloned();
        if entry.is_some() {
            cache.hits += 1;
        } else {
            cache.misses += 1;
        }
        entry
    });
    let Some(entry) = entry else {
        return Ok(None);
    };
    if stamp(&crate::native_fs::metadata(&entry.audit).map_err(|e| e.to_string())?)
        != entry.identity
    {
        return Err("検証済み音声が変更されました".into());
    }
    let audio = VerifiedAudio {
        file: file.try_clone().map_err(|e| e.to_string())?,
        size,
        modified: metadata.modified().map_err(|e| e.to_string())?,
        identity,
        format: entry.format,
        flac: entry.index.map(|v| v.reopen(file)).transpose()?,
    };
    audio.check_unchanged()?;
    Ok(Some(audio))
}
pub(super) fn remember(audio: &VerifiedAudio, hash: &str) -> Result<(), String> {
    if !CACHE.with(|c| c.borrow().is_some()) {
        return Ok(());
    }
    audio.check_unchanged()?;
    let index = audio.flac.as_ref().and_then(|f| f.verified_index());
    // Header-only playback handles never become proof of a full FLAC check.
    if audio.flac.is_some() && index.is_none() {
        return Ok(());
    }
    let bytes = index.as_ref().map_or(0, |v| v.bytes());
    let audit = std::sync::Arc::new(audio.file.try_clone().map_err(|e| e.to_string())?);
    CACHE.with(|c| {
        if let Some(cache) = c.borrow_mut().as_mut() {
            let key = format!("{hash}:{}", audio.identity);
            if !cache.entries.contains_key(&key)
                && cache.entries.len() < 64
                && cache.bytes + bytes <= 64 * 1024 * 1024
            {
                cache.entries.insert(
                    key,
                    Entry {
                        format: audio.format.clone(),
                        index,
                        audit,
                        identity: audio.identity.clone(),
                    },
                );
                cache.bytes += bytes;
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>, String) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.caf");
        let samples: Vec<u8> = (0..20013)
            .flat_map(|i| ((i % 100) as f32 / 128.).to_le_bytes())
            .collect();
        let bytes = crate::audio::tests::caf_fixture(48000, 1, 3, 32, 4, &samples);
        std::fs::write(&path, &bytes).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        (root, path, bytes, hash)
    }
    fn read(path: &std::path::Path, hash: &str) -> Result<VerifiedAudio, String> {
        VerifiedAudio::open(
            File::open(path).unwrap(),
            crate::native_fs::metadata(&path).unwrap().len(),
            hash,
        )
    }
    #[test]
    fn verified_indexes_reuse_exact_caf_and_flac_samples_then_expire() {
        let (_root, path, _, hash) = fixture();
        let mut original = read(&path, &hash).unwrap();
        let flac = path.with_extension("flac");
        let saved =
            flac::write_segment(&mut original, 0, 20013, &flac, &|| false, &|_| {}).unwrap();
        for (path, hash) in [(&path, &hash), (&flac, &saved.sha256)] {
            let scope = Scope::enter().unwrap();
            assert!(Scope::enter().is_err());
            let mut cold = read(path, hash).unwrap();
            let mut warm = read(path, hash).unwrap();
            for (first, frames) in [(4090, 20), (17000, 1900), (100, 2), (0, 10000)] {
                assert_eq!(
                    cold.read_frames(first, frames).unwrap(),
                    warm.read_frames(first, frames).unwrap()
                );
            }
            assert_eq!(scope.counts(), (1, 1));
            scope.verify().unwrap();
            assert!(read(path, &"0".repeat(64)).is_err());
            assert!(VerifiedAudio::open_cancellable(
                File::open(path).unwrap(),
                crate::native_fs::metadata(&path).unwrap().len(),
                hash,
                &|| true
            )
            .is_err());
            drop(scope);
            let scope = Scope::enter().unwrap();
            read(path, hash).unwrap();
            assert_eq!(scope.counts(), (0, 1));
        }
    }
    #[test]
    fn changes_with_restored_mtime_and_file_replacement_cannot_reuse_verification() {
        let (_root, path, bytes, hash) = fixture();
        let scope = Scope::enter().unwrap();
        let original = read(&path, &hash).unwrap();
        let modified = crate::native_fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap();
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::End(-4)).unwrap();
        file.write_all(&1.0_f32.to_le_bytes()).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(original.check_unchanged().is_err());
        assert!(scope.verify().is_err());
        assert!(read(&path, &hash).is_err());
        drop(scope);
        std::fs::write(&path, &bytes).unwrap();
        let scope = Scope::enter().unwrap();
        read(&path, &hash).unwrap();
        let replacement = path.with_extension("replacement");
        std::fs::write(&replacement, bytes).unwrap();
        std::fs::rename(replacement, &path).unwrap();
        read(&path, &hash).unwrap();
        assert_eq!(scope.counts(), (0, 2));
    }
    #[test]
    #[ignore = "read-only benchmark of a local FLAC; set MON_AUDIO_BENCH"]
    fn verified_audio_reuse_benchmark() {
        let path = std::path::PathBuf::from(std::env::var("MON_AUDIO_BENCH").unwrap());
        let hash = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
        let scope = Scope::enter().unwrap();
        let start = std::time::Instant::now();
        let mut cold = read(&path, &hash).unwrap();
        let expected = cold.read_frames(48000 * 60, 48000).unwrap();
        let cold_ms = start.elapsed().as_millis();
        let start = std::time::Instant::now();
        for _ in 0..49 {
            assert_eq!(
                read(&path, &hash)
                    .unwrap()
                    .read_frames(48000 * 60, 48000)
                    .unwrap(),
                expected
            );
        }
        eprintln!(
            "coldMs={cold_ms}, indexed49Ms={}, counts={:?}",
            start.elapsed().as_millis(),
            scope.counts()
        );
    }
}
