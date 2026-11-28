//! Synthetic, memory-only SHA-256 timing using the selected Cargo dependency.
//! `cargo test --no-run --message-format=json` の `compiler-artifact` から
//! sha2 の `.rlib` を取得し、`--extern sha2=<artifact>` を指定する。
//! 依存先の `.rlib` がある各ディレクトリを `-L dependency=<directory>` で渡す。
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let bytes = vec![0x5a; 1024 * 1024];
    // Independently computed with Python hashlib, not the measured Rust code.
    let expected = "bf63d8a95fcc2e64619813aae35fdcbe871fdd9264caa3f365eb3aed0f679129";
    let started = Instant::now();
    for _ in 0..64 {
        let digest = Sha256::digest(black_box(&bytes));
        assert_eq!(format!("{digest:x}"), expected);
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "verified_sha256 mib=64 seconds={seconds:.6} mib_per_second={:.3}",
        64.0 / seconds
    );
}
