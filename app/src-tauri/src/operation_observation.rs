//! Transient, worker-local measurements. Never a storage or deletion permit.
use serde::Serialize;
use std::{cell::RefCell, time::Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    pub stage: String,
    pub completed: u64,
    pub total: Option<u64>,
    pub unit: &'static str,
}
struct Observation {
    category: &'static str,
    started: Instant,
    notified: Instant,
    progress: Progress,
    bytes: u64,
    scans: u64,
    phase_started: Instant,
    db_wait_ns: u128,
    db_hold_ns: u128,
    callback: Box<dyn Fn(Progress)>,
}
thread_local! { static CURRENT: RefCell<Option<Observation>> = const { RefCell::new(None) }; }
pub(crate) struct Scope(Option<Observation>);
impl Scope {
    pub(crate) fn enter(category: &'static str, callback: impl Fn(Progress) + 'static) -> Self {
        let prior = CURRENT.with(|c| {
            c.replace(Some(Observation {
                category,
                started: Instant::now(),
                notified: Instant::now(),
                progress: Progress {
                    stage: "preparing".into(),
                    completed: 0,
                    total: None,
                    unit: "bytes",
                },
                bytes: 0,
                scans: 0,
                phase_started: Instant::now(),
                db_wait_ns: 0,
                db_hold_ns: 0,
                callback: Box::new(callback),
            }))
        });
        Self(prior)
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let old = CURRENT.with(|c| c.replace(self.0.take()));
        if let Some(o) = old {
            (o.callback)(o.progress);
            crate::internal_logs::detail(crate::internal_logs::current_context(o.category),
                "measured", "info", o.started.elapsed(), &serde_json::json!({
                    "readBytes":o.bytes,"fullFileChecks":o.scans,
                    "dbWaitMs":o.db_wait_ns as f64 / 1_000_000.0,"dbHoldMs":o.db_hold_ns as f64 / 1_000_000.0,
                }).to_string());
        }
    }
}
pub(crate) fn stage(name: &str, total: Option<u64>, unit: &'static str) {
    CURRENT.with(|c| {
        if let Some(o) = c.borrow_mut().as_mut() {
            crate::internal_logs::detail(
                crate::internal_logs::current_context(o.category),
                &o.progress.stage,
                "info",
                o.phase_started.elapsed(),
                "工程完了",
            );
            o.progress = Progress {
                stage: name.into(),
                completed: 0,
                total,
                unit,
            };
            o.phase_started = Instant::now();
            o.notified = Instant::now();
            (o.callback)(o.progress.clone());
        }
    });
}
/// Report an absolute worker count without restarting an unchanged stage.
pub(crate) fn report(name: &str, completed: u64, total: u64, unit: &'static str) {
    let changed = CURRENT.with(|c| {
        c.borrow().as_ref().is_some_and(|o| {
            o.progress.stage != name || o.progress.total != Some(total) || o.progress.unit != unit
        })
    });
    if changed {
        stage(name, Some(total), unit);
    }
    CURRENT.with(|c| {
        if let Some(o) = c.borrow_mut().as_mut() {
            let previous = o.progress.completed;
            o.progress.completed = completed;
            // Always publish the final count, even for very short recordings.
            if previous != completed
                && (completed == total || o.notified.elapsed().as_millis() >= 250)
            {
                o.notified = Instant::now();
                (o.callback)(o.progress.clone());
            }
        }
    });
}
pub(crate) struct ConnectionGuard<'a> {
    guard: std::sync::MutexGuard<'a, rusqlite::Connection>,
    started: Instant,
}
pub(crate) fn lock(
    connection: &std::sync::Mutex<rusqlite::Connection>,
) -> Result<ConnectionGuard<'_>, String> {
    let started = Instant::now();
    let guard = connection.lock().map_err(|_| "DBロックを取得できません")?;
    CURRENT.with(|c| {
        if let Some(o) = c.borrow_mut().as_mut() {
            o.db_wait_ns += started.elapsed().as_nanos();
        }
    });
    Ok(ConnectionGuard {
        guard,
        started: Instant::now(),
    })
}
impl std::ops::Deref for ConnectionGuard<'_> {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}
impl std::ops::DerefMut for ConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}
impl Drop for ConnectionGuard<'_> {
    fn drop(&mut self) {
        CURRENT.with(|c| {
            if let Some(o) = c.borrow_mut().as_mut() {
                o.db_hold_ns += self.started.elapsed().as_nanos();
            }
        });
    }
}
pub(crate) fn advance(count: u64) {
    CURRENT.with(|c| {
        if let Some(o) = c.borrow_mut().as_mut() {
            o.progress.completed = o.progress.completed.saturating_add(count);
            if o.notified.elapsed().as_millis() >= 250 {
                o.notified = Instant::now();
                (o.callback)(o.progress.clone());
            }
        }
    });
}
pub(crate) fn read(count: usize) {
    let bytes = CURRENT.with(|c| {
        if let Some(o) = c.borrow_mut().as_mut() {
            o.bytes = o.bytes.saturating_add(count as u64);
            o.progress.unit == "bytes"
        } else {
            false
        }
    });
    if bytes {
        advance(count as u64);
    }
}
pub(crate) fn file_check() {
    CURRENT.with(|c| {
        if let Some(o) = c.borrow_mut().as_mut() {
            o.scans += 1;
        }
    });
}
pub(crate) fn measurements() -> (u64, u64) {
    CURRENT.with(|c| c.borrow().as_ref().map_or((0, 0), |o| (o.bytes, o.scans)))
}

// Join child measurements without treating inspection bytes as saved capacity.
pub(crate) fn absorb(bytes: u64, scans: u64) {
    CURRENT.with(|c| {
        if let Some(o) = c.borrow_mut().as_mut() {
            o.bytes = o.bytes.saturating_add(bytes);
            o.scans = o.scans.saturating_add(scans);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn absolute_progress_preserves_counts_and_throttles_until_final_frame() {
        let notifications = Rc::new(RefCell::new(Vec::new()));
        let captured = notifications.clone();
        let _scope = Scope::enter("recording_save", move |p| captured.borrow_mut().push(p));
        report("mixing_audio", 0, 30, "frames");
        report("mixing_audio", 10, 30, "frames");
        assert_eq!(notifications.borrow().len(), 1);
        CURRENT.with(|c| {
            let mut current = c.borrow_mut();
            let o = current.as_mut().unwrap();
            assert_eq!(o.progress.completed, 10);
            o.notified = Instant::now() - Duration::from_millis(250);
        });
        report("mixing_audio", 20, 30, "frames");
        // Deterministically keep the final update inside the throttle window.
        CURRENT.with(|c| c.borrow_mut().as_mut().unwrap().notified = Instant::now());
        report("mixing_audio", 30, 30, "frames");
        let counts: Vec<_> = notifications.borrow().iter().map(|p| p.completed).collect();
        assert_eq!(counts, vec![0, 20, 30]);
        assert!(notifications.borrow().iter().all(|p| p.total == Some(30)));
    }

    #[test]
    fn absolute_progress_initializes_a_new_stage_once() {
        let notifications = Rc::new(RefCell::new(Vec::new()));
        let captured = notifications.clone();
        let _scope = Scope::enter("recording_save", move |p| captured.borrow_mut().push(p));
        stage("checking_audio", None, "bytes");
        report("mixing_audio", 0, 10, "frames");
        report("mixing_audio", 10, 10, "frames");
        report("mixing_audio", 10, 10, "frames");
        assert_eq!(notifications.borrow().len(), 3);
        assert_eq!(notifications.borrow()[2].completed, 10);
        // A different total identifies a new measured stage.
        report("mixing_audio", 0, 20, "frames");
        assert_eq!(notifications.borrow()[3].completed, 0);
        assert_eq!(notifications.borrow()[3].total, Some(20));
    }
}
