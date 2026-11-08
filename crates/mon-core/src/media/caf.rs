/// Exact v1 CAF header for interleaved little-endian Float32 PCM. Header bytes
/// are shared by both producers and Rust import; existing CAFs are unchanged.
pub fn header(rate: u32, channels: u32, frames: u64) -> Result<[u8; 68], &'static str> {
    pcm_header(rate, channels, frames, true)
}

pub fn voice_header(frames: u64) -> Result<[u8; 68], &'static str> {
    pcm_header(16000, 1, frames, false)
}

fn pcm_header(
    rate: u32,
    channels: u32,
    frames: u64,
    float: bool,
) -> Result<[u8; 68], &'static str> {
    if !(8000..=384000).contains(&rate) || !(1..=8).contains(&channels) {
        return Err("invalid CAF PCM geometry");
    }
    let length = frames
        .checked_mul(u64::from(channels) * if float { 4 } else { 2 })
        .and_then(|n| n.checked_add(4))
        .and_then(|n| i64::try_from(n).ok())
        .ok_or("CAF PCM length exceeds capacity")?;
    let mut bytes = b"caff\0\x01\0\0desc".to_vec();
    bytes.extend(32i64.to_be_bytes());
    bytes.extend(f64::from(rate).to_be_bytes());
    bytes.extend(b"lpcm");
    for n in [
        if float { 3 } else { 2 },
        channels * if float { 4 } else { 2 },
        1,
        channels,
        if float { 32 } else { 16 },
    ] {
        bytes.extend(n.to_be_bytes());
    }
    bytes.extend(b"data");
    bytes.extend(length.to_be_bytes());
    bytes.extend([0; 4]);
    bytes.try_into().map_err(|_| "invalid CAF header size")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caf_bytes_bind_geometry_without_overflow() {
        let bytes = header(48000, 2, 192000).unwrap();
        assert_eq!(&bytes[..12], b"caff\0\x01\0\0desc");
        assert_eq!(&bytes[20..28], &48000f64.to_be_bytes());
        assert_eq!(&bytes[56..64], &(192000i64 * 8 + 4).to_be_bytes());
        assert!(header(0, 2, 1).is_err());
        assert!(header(48000, 9, 1).is_err());
        assert!(header(48000, 2, u64::MAX).is_err());
    }
}
