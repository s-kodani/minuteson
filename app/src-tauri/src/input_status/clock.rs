//! Per-reader clocks keep freshness tests independent of filesystem/CI latency.
//! Production always measures both monotonic time and wall time.
use std::time::{Duration, Instant, SystemTime};

#[derive(Default)]
pub(super) struct Clock {
    #[cfg(test)]
    fixed: Option<(Instant, SystemTime)>,
}
impl Clock {
    pub fn instant(&self) -> Instant {
        #[cfg(test)]
        if let Some((instant, _)) = self.fixed {
            return instant;
        }
        Instant::now()
    }
    pub fn wall(&self) -> SystemTime {
        #[cfg(test)]
        if let Some((_, wall)) = self.fixed {
            return wall;
        }
        SystemTime::now()
    }
    pub fn elapsed(&self, since: Instant) -> Duration {
        self.instant().saturating_duration_since(since)
    }
    #[cfg(test)]
    pub fn fixed(wall: SystemTime) -> Self {
        Self {
            fixed: Some((Instant::now(), wall)),
        }
    }
    #[cfg(test)]
    pub fn advance(&mut self, duration: Duration) {
        let (instant, wall) = self
            .fixed
            .as_mut()
            .expect("advance requires a manual clock");
        *instant += duration;
        *wall += duration;
    }
}
