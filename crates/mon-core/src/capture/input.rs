//! Input rejection is separate from durable storage failure. These checks run
//! before a packet changes converter state or acquires recording credits.
pub fn geometry(rate: u32, channels: u32, frames: u64) -> bool {
    (8000..=384000).contains(&rate) && (1..=8).contains(&channels) && (1..=96_000).contains(&frames)
}

pub fn samples(values: &[f32]) -> bool {
    !values.is_empty()
        && values.len() <= 96_000 * 8
        && values.iter().all(|v| v.is_finite() && v.abs() <= 16.0)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct InputRetry {
    pub failures: u32,
    pub reserved: u32,
    pub retry_at: u64,
    pub healthy_since: u64,
    pub last_valid: u64,
}
impl InputRetry {
    pub fn fault(&mut self, now: u64) {
        self.failures = self.failures.saturating_add(1);
        let seconds = (1u64 << self.failures.saturating_sub(1).min(5)).min(30);
        self.retry_at = now.saturating_add(seconds * 1_000_000_000);
        self.healthy_since = 0;
        self.last_valid = 0;
    }
    pub fn valid(&mut self, now: u64) {
        if self.failures == 0 {
            return;
        }
        if self.healthy_since == 0 || now < self.last_valid || now - self.last_valid > 3_000_000_000
        {
            self.healthy_since = now.saturating_add(1);
        }
        self.last_valid = now;
        if now.saturating_add(1).saturating_sub(self.healthy_since) >= 3_000_000_000 {
            *self = Self::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_input_never_counts_as_silence_or_valid_geometry() {
        assert!(geometry(48_000, 2, 480));
        assert!(!geometry(0, 2, 480));
        assert!(!geometry(48_000, 0, 480));
        assert!(!geometry(48_000, 2, 0));
        assert!(!geometry(48_000, 2, 96_001));
        assert!(samples(&[0.0, -1.5, 16.0]));
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 16.01] {
            assert!(!samples(&[0.1, invalid]));
        }
    }
    #[test]
    fn repeated_faults_back_off_without_ending_the_session() {
        let mut retry = InputRetry::default();
        for delay in [1, 2, 4, 8, 16, 30, 30, 30] {
            retry.fault(1);
            assert_eq!(retry.retry_at, 1 + delay * 1_000_000_000);
        }
        retry.valid(1);
        retry.valid(1_000_000_001);
        retry.valid(2_000_000_001);
        assert_ne!(retry.failures, 0);
        retry.valid(3_000_000_001);
        assert_eq!(retry.failures, 0);
        retry.fault(4_000_000_001);
        assert_eq!(retry.retry_at, 5_000_000_001);
    }
    #[test]
    fn quiet_time_or_clock_reversal_does_not_prove_healthy_flow() {
        let mut retry = InputRetry::default();
        retry.fault(1);
        retry.valid(2);
        retry.valid(10_000_000_000);
        assert_ne!(retry.failures, 0);
        retry.valid(3);
        assert_ne!(retry.failures, 0);
    }
}
