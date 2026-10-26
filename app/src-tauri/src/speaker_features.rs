//! A job owns verified audio indexes and its warm model; only bounded feature
//! vectors survive between jobs in the current Storage instance (never disk).
use crate::{
    audio,
    diarization::{embedding, sherpa::SherpaConfig},
    internal_logs::{self, Context},
    local_engine::{resident, Fingerprint, StableFile},
    storage::{RecordedAudioPlan, Storage},
    voice::Window,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct Cache {
    values: HashMap<String, Vec<f32>>,
    order: VecDeque<String>,
    bytes: usize,
}
impl Cache {
    fn get(&self, key: &str) -> Option<Vec<f32>> {
        self.values.get(key).cloned()
    }
    fn put(&mut self, key: String, vector: Vec<f32>) {
        if self.values.contains_key(&key) || crate::voice::normalize(&vector).is_err() {
            return;
        }
        while self.values.len() >= 2048 || self.bytes + vector.len() * 4 > 16 * 1024 * 1024 {
            let Some(old) = self.order.pop_front() else {
                return;
            };
            if let Some(v) = self.values.remove(&old) {
                self.bytes -= v.len() * 4;
            }
        }
        self.bytes += vector.len() * 4;
        self.order.push_back(key.clone());
        self.values.insert(key, vector);
    }
}
impl Storage {
    fn cached_feature(&self, key: &str) -> Option<Vec<f32>> {
        self.feature_vectors.lock().ok().and_then(|c| c.get(key))
    }
    fn cache_feature(&self, key: String, vector: Vec<f32>) {
        if let Ok(mut c) = self.feature_vectors.lock() {
            c.put(key, vector);
        }
    }
}

pub(crate) struct Session<'a> {
    storage: &'a Storage,
    meeting: &'a str,
    engine: &'a SherpaConfig,
    extractor: &'a embedding::ExtractorConfig,
    namespace: String,
    source: &'a RecordedAudioPlan,
    watched: Vec<StableFile>,
    files: crate::local_engine::verification::Scope,
    audio: audio::reuse::Scope,
    _resident: resident::Scope,
    context: Context,
    started: Instant,
    hits: usize,
    extractions: usize,
}
impl<'a> Session<'a> {
    pub(crate) fn new(
        storage: &'a Storage,
        meeting: &'a str,
        engine: &'a SherpaConfig,
        extractor: &'a embedding::ExtractorConfig,
        source: &'a RecordedAudioPlan,
        runtime: Option<&resident::Snapshot>,
        context: Context,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let files = crate::local_engine::verification::Scope::enter(true)?;
        engine.verify(cancelled)?;
        extractor.helper.verify(cancelled)?;
        extractor.library.verify(cancelled)?;
        let mut watched = Vec::new();
        for f in [
            &engine.executable,
            &engine.segmentation_model,
            &engine.embedding_model,
            &extractor.helper,
            &extractor.library,
        ]
        .into_iter()
        .chain(engine.runtime_libraries.iter())
        {
            watched.push(StableFile::watch(f)?);
        }
        let namespace=format!("{:x}",Sha256::digest(serde_json::to_vec(&serde_json::json!({"version":"speaker-window-pcm16-v1","extractorVersion":crate::voice::EXTRACTOR,"source":source,"engine":engine,"extractor":extractor,"residentRuntime":runtime})).map_err(|e|e.to_string())?));
        let audio = audio::reuse::Scope::enter()?;
        let worker = resident::Scope::enter(storage.root(), runtime)?;
        internal_logs::detail(context.clone(),"preparing","info",Duration::ZERO,&serde_json::json!({"event":"話者特徴セッション開始","warmModel":runtime.is_some(),"audioIndexReuse":true,"featureCache":"memory"}).to_string());
        Ok(Self {
            storage,
            meeting,
            engine,
            extractor,
            namespace,
            source,
            watched,
            files,
            audio,
            _resident: worker,
            context,
            started: Instant::now(),
            hits: 0,
            extractions: 0,
        })
    }
    fn verify(&self) -> Result<(), String> {
        if self.storage.recorded_analysis_input(self.meeting)?.0 != *self.source {
            return Err("処理中に音声区間・時刻補正が変更されました".into());
        }
        for f in &self.watched {
            f.verify()?;
        }
        self.audio.verify()
    }
    pub(crate) fn extract(
        &mut self,
        window: &Window,
        directory: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Vec<f32>>, String> {
        if cancelled() {
            return Ok(None);
        }
        self.verify()?;
        // This always rechecks DB ownership, source coverage and the open file's
        // identity; a feature-cache hit cannot bypass deleted/changed audio.
        let audio = self.storage.prepare_track_audio_cancellable(
            self.meeting,
            &window.track_id,
            window.start_us,
            window.end_us,
            cancelled,
        )?;
        let key = format!(
            "{}:{}:{}:{}",
            self.namespace, window.track_id, window.start_us, window.end_us
        );
        if let Some(vector) = self.storage.cached_feature(&key) {
            for span in &audio.spans {
                span.audio.check_unchanged()?;
            }
            self.verify()?;
            self.hits += 1;
            internal_logs::detail(self.context.clone(),"running","info",self.started.elapsed(),&serde_json::json!({"event":"話者特徴キャッシュ再利用","trackId":window.track_id,"startUs":window.start_us,"endUs":window.end_us}).to_string());
            return Ok(Some(vector));
        }
        std::fs::create_dir(directory).map_err(|e| e.to_string())?;
        let track = self
            .source
            .tracks
            .iter()
            .find(|t| t.id == window.track_id)
            .ok_or("声の参照先が共通音声にありません")?;
        let path = self.storage.meeting_mix_path(self.meeting, track)?;
        let first = crate::audio_clock::frame_at(window.start_us, 16000)?;
        let end = crate::audio_clock::frame_at(window.end_us, 16000)?;
        let wave = Fingerprint::capture_retained(&path, cancelled)?;
        let output = embedding::run(
            self.extractor,
            self.engine,
            &wave,
            first,
            end,
            directory,
            Duration::from_secs(300),
            cancelled,
        )?;
        for span in &audio.spans {
            span.audio.check_unchanged()?;
        }
        self.verify()?;
        self.extractions += 1;
        let vector = output.map(|o| o.vector(first, end)).transpose()?.flatten();
        if let Some(v) = &vector {
            self.storage.cache_feature(key, v.clone());
        }
        // The resident worker still checks its first startup log on later requests.
        // Retain that bounded log until the job-owned temp directory is dropped.
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() != "worker-startup.log" {
                std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        Ok(vector)
    }
    pub(crate) fn finish(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        self.verify()?;
        self.engine.verify(cancelled)?;
        self.extractor.helper.verify(cancelled)?;
        self.extractor.library.verify(cancelled)?;
        self.files.finish(cancelled)?;
        self.verify()
    }
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        let (hits, misses) = self.audio.counts();
        internal_logs::detail(self.context.clone(),"stopped","info",self.started.elapsed(),&serde_json::json!({"event":"話者特徴セッション終了","featureCacheHits":self.hits,"featureExtractions":self.extractions,"audioIndexHits":hits,"audioFullVerifications":misses}).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feature_cache_is_bounded_and_keeps_exact_valid_vectors() {
        let mut cache = Cache::default();
        for i in 0..2050 {
            cache.put(i.to_string(), vec![0.25, -0.75]);
        }
        assert_eq!(cache.values.len(), 2048);
        assert_eq!(cache.bytes, 2048 * 8);
        assert!(cache.get("0").is_none());
        assert_eq!(cache.get("2049"), Some(vec![0.25, -0.75]));
        for v in [vec![], vec![0.0], vec![f32::NAN], vec![f32::INFINITY]] {
            cache.put("invalid".into(), v);
        }
        assert!(cache.get("invalid").is_none());
        cache.put("oversize".into(), vec![1.0; 16 * 1024 * 1024 / 4 + 1]);
        assert!(cache.get("oversize").is_none());
        assert!(cache.bytes <= 16 * 1024 * 1024);
    }
}
