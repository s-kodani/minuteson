use super::*;

fn fixture(bits: u16, valid_bits: Option<u16>, codec: u16, samples: &[u8]) -> Vec<u8> {
    let extended = valid_bits.is_some();
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&(if extended { 0xfffeu16 } else { codec }).to_le_bytes());
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&48000u32.to_le_bytes());
    fmt.extend_from_slice(&(48000u32 * u32::from(bits / 8)).to_le_bytes());
    fmt.extend_from_slice(&(bits / 8).to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    if let Some(valid) = valid_bits {
        fmt.extend_from_slice(&22u16.to_le_bytes());
        fmt.extend_from_slice(&valid.to_le_bytes());
        fmt.extend_from_slice(&0u32.to_le_bytes());
        fmt.extend_from_slice(&codec.to_le_bytes());
        fmt.extend_from_slice(&[0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    }
    let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    wav.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    wav.extend_from_slice(&fmt);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    wav.extend_from_slice(samples);
    if samples.len() % 2 != 0 {
        wav.push(0);
    }
    let length = (wav.len() - 8) as u32;
    wav[4..8].copy_from_slice(&length.to_le_bytes());
    wav
}

fn read(bytes: &[u8]) -> Result<(Wave, Vec<f32>), String> {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(bytes).unwrap();
    file.rewind().unwrap();
    let format = wave(&mut file)?;
    file.seek(SeekFrom::Start(format.offset)).unwrap();
    let mut samples = vec![0; format.frames as usize * usize::from(format.bits / 8)];
    file.read_exact(&mut samples).unwrap();
    let pcm = samples
        .chunks_exact(usize::from(format.bits / 8))
        .map(|s| format.sample(s))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((format, pcm))
}

#[test]
fn extensible_integer_uses_left_aligned_valid_bits() {
    // Deliberately nonzero padding must not become part of the sample.
    let samples = [i32::MIN | 0xff, -1073741824 | 0xff, 0xff, 1073741824 | 0xff];
    let raw: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let (format, pcm) = read(&fixture(32, Some(24), 1, &raw)).unwrap();
    assert_eq!(format.valid_bits, 24);
    assert_eq!(format.frames, 4);
    assert_eq!(format.rate, 48000);
    assert_eq!(format.channels, 1);
    assert_eq!(pcm, [-1.0, -0.5, 0.0, 0.5]);
    for (bits, valid, raw) in [
        (24, 20, vec![15, 0, 128, 15, 0, 192, 15, 0, 0, 15, 0, 64]),
        (16, 12, vec![15, 128, 15, 192, 15, 0, 15, 64]),
        (8, 4, vec![15, 79, 143, 207]),
        (8, 1, vec![127, 127, 255, 255]),
    ] {
        let (_, pcm) = read(&fixture(bits, Some(valid), 1, &raw)).unwrap();
        let expected = if valid == 1 {
            [-1.0, -1.0, 0.0, 0.0]
        } else {
            [-1.0, -0.5, 0.0, 0.5]
        };
        assert_eq!(pcm, expected, "{valid} in {bits}");
    }
}

#[test]
fn ordinary_integer_and_float_wav_keep_their_scaling() {
    for (bits, codec, raw) in [
        (8, 1, vec![0, 64, 128, 192]),
        (16, 1, vec![0, 128, 0, 192, 0, 0, 0, 64]),
        (24, 1, vec![0, 0, 128, 0, 0, 192, 0, 0, 0, 0, 0, 64]),
        (
            32,
            1,
            [-2147483648i32, -1073741824, 0, 1073741824]
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect(),
        ),
        (
            32,
            3,
            [-1.0f32, -0.5, 0.0, 0.5]
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect(),
        ),
        (
            64,
            3,
            [-1.0f64, -0.5, 0.0, 0.5]
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect(),
        ),
    ] {
        for extension in [None, Some(bits)] {
            assert_eq!(
                read(&fixture(bits, extension, codec, &raw)).unwrap().1,
                [-1.0, -0.5, 0.0, 0.5]
            );
        }
    }
}

#[test]
fn malformed_valid_bits_and_float_nonfinite_values_are_rejected() {
    for (bits, valid, codec) in [(32, 0, 1), (16, 17, 1), (32, 24, 3), (64, 0, 3)] {
        assert!(read(&fixture(
            bits,
            Some(valid),
            codec,
            &vec![0; usize::from(bits / 8)]
        ))
        .is_err());
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(read(&fixture(32, Some(32), 3, &value.to_le_bytes())).is_err());
    }
    let mut truncated_extension = fixture(32, Some(24), 1, &[0; 4]);
    truncated_extension[36..38].copy_from_slice(&23u16.to_le_bytes());
    assert!(read(&truncated_extension).is_err());
}

#[cfg(windows)]
#[test]
fn windows_decoder_publishes_valid24_in32_as_float_caf() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("valid24-in32.wav");
    let output = root.path().join("decoded.caf");
    let raw: Vec<_> = [i32::MIN, -1073741824, 0, 1073741824]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    std::fs::write(&input, fixture(32, Some(24), 1, &raw)).unwrap();
    let info = decode(&input, Some(&output), &|| false).unwrap();
    assert_eq!(info.frames, 4);
    assert_eq!(info.sample_rate, 48000);
    let caf = std::fs::read(output).unwrap();
    let expected: Vec<_> = [-1.0f32, -0.5, 0.0, 0.5]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    assert!(caf.ends_with(&expected));
}
