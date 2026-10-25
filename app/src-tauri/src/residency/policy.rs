//! All deadlines are measured from the first continuous loss of every entry.
//! Poll gaps (sleep/suspension) restart observation, never expire a sleeping app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Healthy,
    Restore,
    Drain,
    Terminate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Observation {
    Present,
    Absent,
    Unknown,
}
#[derive(Default)]
pub(crate) struct Watch {
    missing: Option<u64>,
    last: Option<u64>,
    confirmed: u32,
}
impl Watch {
    pub(crate) fn observe(
        &mut self,
        now_ms: u64,
        observation: Observation,
        paused: bool,
    ) -> Action {
        if observation != Observation::Absent {
            self.tick(now_ms, true, paused);
            return if observation == Observation::Unknown && !paused {
                Action::Restore
            } else {
                Action::Healthy
            };
        }
        self.tick(now_ms, false, paused)
    }
    pub(crate) fn tick(&mut self, now_ms: u64, entry: bool, paused: bool) -> Action {
        if self
            .last
            .is_some_and(|last| now_ms.saturating_sub(last) > 5000)
        {
            self.missing = None;
            self.confirmed = 0;
        }
        self.last = Some(now_ms);
        if entry || paused {
            self.missing = None;
            self.confirmed = 0;
            return Action::Healthy;
        }
        self.confirmed = self.confirmed.saturating_add(1);
        let missing = *self.missing.get_or_insert(now_ms);
        match now_ms.saturating_sub(missing) {
            0..=4999 => Action::Restore,
            5000..=24999 => Action::Drain,
            _ if self.confirmed >= 20 => Action::Terminate,
            _ => Action::Drain,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_or_one_transient_failure_can_never_authorize_termination() {
        let mut watch = Watch::default();
        for second in 0..100 {
            assert_eq!(
                watch.observe(second * 1000, Observation::Unknown, false),
                Action::Restore
            );
        }
        for second in 100..124 {
            watch.observe(second * 1000, Observation::Absent, false);
        }
        assert_eq!(
            watch.observe(124000, Observation::Unknown, false),
            Action::Restore
        );
        assert_eq!(
            watch.observe(125000, Observation::Absent, false),
            Action::Restore
        );
    }
    #[test]
    fn continuous_loss_has_bounded_restore_drain_and_termination() {
        let mut watch = Watch::default();
        for second in 0..=30 {
            assert_eq!(
                watch.tick(second * 1000, false, false),
                if second < 5 {
                    Action::Restore
                } else if second < 25 {
                    Action::Drain
                } else {
                    Action::Terminate
                }
            );
        }
    }
    #[test]
    fn any_entry_pause_or_poll_gap_prevents_false_termination() {
        let mut watch = Watch::default();
        for second in 0..25 {
            watch.tick(second * 1000, false, false);
        }
        assert_eq!(watch.tick(25_000, true, false), Action::Healthy);
        assert_eq!(watch.tick(26_000, false, false), Action::Restore);
        assert_eq!(watch.tick(27_000, false, true), Action::Healthy);
        assert_eq!(watch.tick(28_000, false, false), Action::Restore);
        assert_eq!(watch.tick(600_000, false, false), Action::Restore);
    }
}
