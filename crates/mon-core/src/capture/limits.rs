pub const SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_SEGMENTS: u32 = 32_768;
pub const MAX_ENTRY_BYTES: u64 = 65 * 1024;
pub const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;
pub const TIP_RESERVE_BYTES: u64 = 1024;
pub const MAX_CLOCK_MAPS: u64 = 65_536;
pub const MAX_PENDING_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_PENDING_NANOSECONDS: u64 = 20_000_000_000;
pub const MAX_PENDING_PACKETS: u32 = 4096;
pub const ACQUISITION_TIMEOUT_NANOSECONDS: u64 = 5_000_000_000;
pub const RETRY_NANOSECONDS: u64 = 1_000_000_000;

/// Test before creating the next receipt or advertising its prefix. Reserved
/// bytes keep the terminal tip update within the same reader budget.
pub fn permits_commit(
    segments: u32,
    metadata: u64,
    maps: u64,
    next_bytes: u64,
    next_maps: u64,
) -> bool {
    segments < MAX_SEGMENTS
        && next_bytes > 0
        && next_bytes <= MAX_ENTRY_BYTES
        && metadata
            .checked_add(next_bytes)
            .is_some_and(|n| n <= MAX_METADATA_BYTES - TIP_RESERVE_BYTES)
        && next_maps > 0
        && maps
            .checked_add(next_maps)
            .is_some_and(|n| n <= MAX_CLOCK_MAPS)
}

pub fn permits_pending(
    bytes: u64,
    nanoseconds: u64,
    packets: u32,
    next_bytes: u64,
    next_nanoseconds: u64,
) -> bool {
    packets < MAX_PENDING_PACKETS
        && bytes
            .checked_add(next_bytes)
            .is_some_and(|n| n <= MAX_PENDING_BYTES)
        && nanoseconds
            .checked_add(next_nanoseconds)
            .is_some_and(|n| n <= MAX_PENDING_NANOSECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commit_boundaries_reject_before_prefix_changes() {
        assert!(permits_commit(
            MAX_SEGMENTS - 1,
            MAX_METADATA_BYTES - TIP_RESERVE_BYTES - 1,
            MAX_CLOCK_MAPS - 1,
            1,
            1
        ));
        assert!(!permits_commit(MAX_SEGMENTS, 0, 0, 1, 1));
        assert!(!permits_commit(
            0,
            MAX_METADATA_BYTES - TIP_RESERVE_BYTES,
            0,
            1,
            1
        ));
        assert!(!permits_commit(0, 0, MAX_CLOCK_MAPS, 1, 1));
        assert!(!permits_commit(0, u64::MAX, 0, 1, 1));
        assert!(!permits_commit(0, 0, 0, MAX_ENTRY_BYTES + 1, 1));
    }
    #[test]
    fn pending_budget_has_independent_byte_time_and_packet_limits() {
        assert!(permits_pending(
            MAX_PENDING_BYTES - 1,
            MAX_PENDING_NANOSECONDS - 1,
            MAX_PENDING_PACKETS - 1,
            1,
            1
        ));
        assert!(!permits_pending(MAX_PENDING_BYTES, 0, 0, 1, 1));
        assert!(!permits_pending(0, MAX_PENDING_NANOSECONDS, 0, 1, 1));
        assert!(!permits_pending(0, 0, MAX_PENDING_PACKETS, 1, 1));
        assert!(!permits_pending(u64::MAX, 0, 0, 1, 1));
    }
}
