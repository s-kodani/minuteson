use super::limits::SAFE_INTEGER;

/// Shared lifecycle reducer. Native drivers separately own asynchronous OS
/// operations and binding tokens; timeout never implies native closure.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputLifecycle {
    pub epoch: u64,
    pub interruptions: u64,
    pub phase: u32,
    pub interrupted: u32,
}
pub const RECONNECTING: u32 = 0;
pub const CONNECTED: u32 = 1;
pub const DISCONNECTED: u32 = 2;
pub const UNAVAILABLE: u32 = 3;
pub const STOPPED: u32 = 4;
impl InputLifecycle {
    pub fn begin(&mut self) -> bool {
        let old = self.epoch.max(self.interruptions);
        if self.phase == STOPPED || old >= SAFE_INTEGER {
            return false;
        }
        self.epoch = old + 1;
        self.phase = RECONNECTING;
        true
    }
    pub fn connect(&mut self) -> bool {
        if self.phase == STOPPED {
            return false;
        }
        self.phase = CONNECTED;
        self.interrupted = 0;
        true
    }
    pub fn interrupt(&mut self, unavailable: bool) -> Option<bool> {
        if self.phase == STOPPED {
            return None;
        }
        let first = self.interrupted == 0;
        if first {
            if self.interruptions >= SAFE_INTEGER {
                return None;
            }
            self.interruptions += 1;
        }
        self.interrupted = 1;
        self.phase = if unavailable {
            UNAVAILABLE
        } else {
            DISCONNECTED
        };
        Some(first)
    }
    pub fn stop(&mut self) {
        self.phase = STOPPED;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_and_duplicate_loss_have_identical_monotonic_status() {
        let mut state = InputLifecycle::default();
        assert!(state.begin());
        assert_eq!(state.epoch, 1);
        assert!(state.connect());
        assert_eq!(state.interrupt(false), Some(true));
        assert_eq!(state.interrupt(true), Some(false));
        assert_eq!(state.interruptions, 1);
        assert!(state.begin());
        assert_eq!(state.phase, RECONNECTING);
        assert_eq!(state.epoch, 2);
        assert!(state.connect());
        state.stop();
        assert!(!state.begin());
        assert!(!state.connect());
        assert_eq!(state.interrupt(false), None);
    }
}
