//! Reserve before expensive I/O. Cancellation never creates a reservation;
//! an unknown, released or already claimed ID cannot start file verification.
use super::{PlaybackInfo, PlaybackStore};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const LIMIT: usize = 8;
const CANCELLED: &str = "音声の再生準備を取り消しました";
struct Entry {
    claimed: bool,
    cancelled: Arc<AtomicBool>,
    published: Option<String>,
}
#[derive(Default)]
pub(crate) struct PlaybackPreparations(Arc<Mutex<HashMap<String, Entry>>>);
pub(crate) struct PreparationGuard {
    owner: Arc<Mutex<HashMap<String, Entry>>>,
    id: String,
    cancelled: Arc<AtomicBool>,
}
fn validate(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id) {
        Ok(())
    } else {
        Err("音声準備のIDが不正です".into())
    }
}
impl PlaybackPreparations {
    pub(crate) fn reserve(&self, id: &str) -> Result<(), String> {
        validate(id)?;
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "音声準備ロックを取得できません")?;
        if entries.contains_key(id) {
            return Err("同じ音声準備IDが既に登録されています".into());
        }
        if entries.len() >= LIMIT {
            return Err("音声準備が多すぎます。取消または完了を待ってください".into());
        }
        entries.insert(
            id.into(),
            Entry {
                claimed: false,
                cancelled: Arc::new(AtomicBool::new(false)),
                published: None,
            },
        );
        Ok(())
    }
    pub(crate) fn claim(&self, id: &str) -> Result<PreparationGuard, String> {
        validate(id)?;
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "音声準備ロックを取得できません")?;
        let entry = entries
            .get_mut(id)
            .ok_or("音声準備IDが未登録または取消済みです")?;
        if entry.claimed || entry.cancelled.load(Ordering::Acquire) {
            return Err("音声準備IDは使用中または取消済みです".into());
        }
        entry.claimed = true;
        Ok(PreparationGuard {
            owner: self.0.clone(),
            id: id.into(),
            cancelled: entry.cancelled.clone(),
        })
    }
    pub(crate) fn cancel(&self, id: &str, store: &PlaybackStore) -> Result<(), String> {
        validate(id)?;
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "音声準備ロックを取得できません")?;
        if let Some(entry) = entries.get(id) {
            entry.cancelled.store(true, Ordering::Release);
            if let Some(token) = &entry.published {
                // Close even when the prepare reply and token were lost.
                store.close(token)?;
                entries.remove(id);
                return Ok(());
            }
            // Claimed work retains its slot until its guard actually drops.
            if !entry.claimed {
                entries.remove(id);
            }
        }
        Ok(())
    }
    pub(crate) fn acknowledge(&self, id: &str, token: &str) -> Result<(), String> {
        validate(id)?;
        validate(token)?;
        let mut entries = self
            .0
            .lock()
            .map_err(|_| "音声準備ロックを取得できません")?;
        let entry = entries
            .get(id)
            .ok_or("音声準備が取消済みまたは解放済みです")?;
        if entry.cancelled.load(Ordering::Acquire) || entry.published.as_deref() != Some(token) {
            return Err("音声準備と受け取った再生トークンが一致しません".into());
        }
        entries.remove(id);
        Ok(())
    }
}
impl PreparationGuard {
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
    // Serialize the cancellation decision with publishing a playback token.
    // Retain ownership until receipt acknowledgement or cancellation.
    pub(crate) fn publish(
        &self,
        insert: impl FnOnce() -> Result<PlaybackInfo, String>,
    ) -> Result<PlaybackInfo, String> {
        let mut entries = self
            .owner
            .lock()
            .map_err(|_| "音声準備ロックを取得できません")?;
        if !entries
            .get(&self.id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.cancelled, &self.cancelled))
        {
            return Err("音声準備の所有が失われています".into());
        }
        self.check()?;
        if entries
            .get(&self.id)
            .is_some_and(|entry| entry.published.is_some())
        {
            return Err("この音声準備は公開済みです".into());
        }
        let info = insert()?;
        entries.get_mut(&self.id).unwrap().published = Some(info.token.clone());
        Ok(info)
    }
}
impl Drop for PreparationGuard {
    fn drop(&mut self) {
        let mut entries = self.owner.lock().unwrap_or_else(|error| error.into_inner());
        if entries.get(&self.id).is_some_and(|entry| {
            Arc::ptr_eq(&entry.cancelled, &self.cancelled) && entry.published.is_none()
        }) {
            entries.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    fn info() -> PlaybackInfo {
        PlaybackInfo {
            token: id(),
            sample_rate: 48000,
            channels: 1,
            total_frames: 1,
            start_us: 0,
            end_us: 1,
            track_kind: "mic".into(),
            alignment: "unverified".into(),
            missing_ranges: vec![],
        }
    }
    #[test]
    fn preparation_requires_an_exact_reserved_id_and_cannot_be_claimed_twice() {
        let p = PlaybackPreparations::default();
        let token = id();
        for invalid in ["", "not-a-uuid", &token.to_uppercase()] {
            assert!(p.reserve(invalid).is_err());
            assert!(p.claim(invalid).is_err());
            assert!(p.cancel(invalid, &PlaybackStore::default()).is_err());
        }
        assert!(p.claim(&token).is_err());
        p.cancel(&token, &PlaybackStore::default()).unwrap();
        assert!(p.claim(&token).is_err());
        p.reserve(&token).unwrap();
        assert!(p.reserve(&token).is_err());
        let guard = p.claim(&token).unwrap();
        assert!(p.claim(&token).is_err());
        assert!(!guard.cancelled());
        drop(guard);
        assert!(p.claim(&token).is_err());
    }
    #[test]
    fn cancellation_before_claim_releases_only_that_reservation() {
        let p = PlaybackPreparations::default();
        let first = id();
        let second = id();
        p.reserve(&first).unwrap();
        p.reserve(&second).unwrap();
        p.cancel(&first, &PlaybackStore::default()).unwrap();
        p.cancel(&first, &PlaybackStore::default()).unwrap();
        assert!(p.claim(&first).is_err());
        assert!(!p.claim(&second).unwrap().cancelled());
        assert!(p.0.lock().unwrap().is_empty());
    }
    #[test]
    fn claimed_cancellation_is_scoped_and_slot_is_not_reused_until_actual_drop() {
        let p = PlaybackPreparations::default();
        let first = id();
        let second = id();
        p.reserve(&first).unwrap();
        p.reserve(&second).unwrap();
        let a = p.claim(&first).unwrap();
        let b = p.claim(&second).unwrap();
        p.cancel(&first, &PlaybackStore::default()).unwrap();
        assert!(a.cancelled());
        assert!(!b.cancelled());
        assert!(a.check().is_err());
        assert!(p.reserve(&first).is_err());
        assert!(p.claim(&first).is_err());
        drop(a);
        assert_eq!(p.0.lock().unwrap().len(), 1);
        drop(b);
        assert!(p.0.lock().unwrap().is_empty());
    }
    #[test]
    fn capacity_never_evicts_running_hashes_and_reclaims_finished_or_unstarted_work() {
        let p = PlaybackPreparations::default();
        let ids: Vec<_> = (0..LIMIT).map(|_| id()).collect();
        for token in &ids {
            p.reserve(token).unwrap();
        }
        assert!(p.reserve(&id()).is_err());
        let guard = p.claim(&ids[0]).unwrap();
        p.cancel(&ids[0], &PlaybackStore::default()).unwrap();
        assert!(p.reserve(&id()).is_err());
        p.cancel(&ids[1], &PlaybackStore::default()).unwrap();
        p.reserve(&id()).unwrap();
        assert!(p.reserve(&id()).is_err());
        drop(guard);
        p.reserve(&id()).unwrap();
    }
    #[test]
    fn cancelled_work_cannot_publish_and_panics_release_the_guard() {
        let p = PlaybackPreparations::default();
        let token = id();
        p.reserve(&token).unwrap();
        let guard = p.claim(&token).unwrap();
        p.cancel(&token, &PlaybackStore::default()).unwrap();
        assert!(guard
            .publish(|| -> Result<PlaybackInfo, String> {
                panic!("cancelled publication must not run")
            })
            .is_err());
        drop(guard);
        let token = id();
        p.reserve(&token).unwrap();
        assert!(std::panic::catch_unwind(|| {
            let _guard = p.claim(&token).unwrap();
            panic!("worker panic");
        })
        .is_err());
        assert!(p.0.lock().unwrap().is_empty());
    }
    #[test]
    fn lost_reply_keeps_exact_token_owned_until_acknowledgement_or_cancel() {
        let p = PlaybackPreparations::default();
        let token = id();
        p.reserve(&token).unwrap();
        let guard = p.claim(&token).unwrap();
        let receipt = guard.publish(|| Ok(info())).unwrap();
        assert!(guard.publish(|| Ok(info())).is_err());
        drop(guard);
        assert!(p.acknowledge(&token, &id()).is_err());
        assert!(p.claim(&token).is_err());
        assert_eq!(p.0.lock().unwrap().len(), 1);
        p.acknowledge(&token, &receipt.token).unwrap();
        assert!(p.0.lock().unwrap().is_empty());
        assert!(p.acknowledge(&token, &receipt.token).is_err());
        let token = id();
        p.reserve(&token).unwrap();
        let guard = p.claim(&token).unwrap();
        let receipt = guard.publish(|| Ok(info())).unwrap();
        drop(guard);
        p.cancel(&token, &PlaybackStore::default()).unwrap();
        assert!(p.acknowledge(&token, &receipt.token).is_err());
        assert!(p.0.lock().unwrap().is_empty());
    }
}
