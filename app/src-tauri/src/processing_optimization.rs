//! Required, frozen optimization policy for current jobs.
pub(crate) struct Scope {
    files: crate::local_engine::verification::Scope,
    audio: Option<crate::audio::reuse::Scope>,
}
impl Scope {
    pub(crate) fn enter(version: Option<u32>) -> Result<Self, String> {
        if version != Some(crate::current_formats::OPTIMIZATION) {
            return Err("未対応の処理最適化です".into());
        }
        Ok(Self {
            files: crate::local_engine::verification::Scope::enter(true)?,
            audio: Some(crate::audio::reuse::Scope::enter()?),
        })
    }
    pub(crate) fn finish(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        if let Some(a) = &self.audio {
            a.verify()?;
        }
        self.files.finish(cancelled)
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(a) = &self.audio {
            let (hits, misses) = a.counts();
            crate::internal_logs::detail(crate::internal_logs::current_context("analysis"),"stopped","info",std::time::Duration::ZERO,&serde_json::json!({"event":"音声検証共有終了","audioIndexHits":hits,"audioFullVerifications":misses}).to_string());
        }
    }
}
