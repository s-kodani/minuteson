//! Geometry adapter for pre-compact interrupted recordings. New producers
//! always write v3 PCM16. Kept separate so legacy recovery can be retired.
pub fn audio_bytes(
    version: u32,
    encoding: Option<&str>,
    rate: u32,
    channels: u32,
    frames: u64,
) -> Option<u64> {
    let width = match (version, encoding) {
        (3, Some("pcm16")) if rate == 16000 && channels == 1 => 2,
        (2, None) if (8000..=384000).contains(&rate) && (1..=8).contains(&channels) => {
            u64::from(channels) * 4
        }
        _ => return None,
    };
    frames.checked_mul(width)?.checked_add(68)
}
